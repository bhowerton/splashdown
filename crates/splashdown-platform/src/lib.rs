//! OS integration: global hotkeys, text insertion, focus and selection
//! context, the status overlay, and permission checks.
//!
//! The API is the same on every platform; `mac/` and `win/` implement it.
//! Gesture rules (hold, double-tap, chords) live in `splashdown_core::gesture`, so
//! this crate only reports raw key events.

pub use splashdown_core::gesture::{KeyEvent, PhysicalKey};

#[cfg(target_os = "macos")]
#[path = "mac/mod.rs"]
mod imp;

#[cfg(not(target_os = "macos"))]
#[path = "win/mod.rs"]
mod imp;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{0} permission is needed")]
    Permission(&'static str),
    #[error("not supported on this platform yet")]
    Unsupported,
    #[error("{0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, PlatformError>;

/// Human-readable name of the platform backend compiled into this build.
pub fn backend_name() -> &'static str {
    imp::NAME
}

// ---------------------------------------------------------------- keys

/// Called for every key event, on the listener's own thread; must be quick.
/// Returns `true` to swallow the event so the focused app never sees it.
pub type KeyHandler = Box<dyn FnMut(KeyEvent) -> bool + Send + 'static>;

/// Listens for keys until dropped.
pub struct KeyListener {
    _inner: imp::KeyListener,
}

/// Starts a system-wide key listener. Needs Accessibility permission on
/// macOS.
pub fn listen_keys(handler: KeyHandler) -> Result<KeyListener> {
    Ok(KeyListener {
        _inner: imp::KeyListener::start(handler)?,
    })
}

// ---------------------------------------------------------------- focus

/// The app that has keyboard focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    /// Bundle identifier (macOS) or executable name (Windows).
    pub id: String,
    pub name: String,
    pub pid: i32,
    pub window_title: Option<String>,
    /// The page address when the focused field is in a web page.
    pub url: Option<String>,
    /// The file or folder the focused window shows, when the app says: a
    /// terminal's working folder, an editor's open file.
    pub document: Option<std::path::PathBuf>,
}

/// The byte offset of the `units`-th UTF-16 unit in `text`.
fn utf16_to_byte(text: &str, units: usize) -> Option<usize> {
    let mut seen = 0;
    for (byte, c) in text.char_indices() {
        if seen >= units {
            return Some(byte);
        }
        seen += c.len_utf16();
    }
    (seen >= units).then_some(text.len())
}

/// `file:///Users/me/My%20Project/` → `/Users/me/My Project/`.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn file_url_path(url: &str) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok().map(std::path::PathBuf::from)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Text(String),
    /// The field reports that nothing is selected.
    Empty,
    /// The app doesn't expose its selection; try `copy_selection`.
    Unknown,
}

/// Where keyboard focus was when dictation started.
pub struct Focus {
    pub app: AppInfo,
    inner: imp::FocusElement,
}

impl Focus {
    /// Captures the current focus, or `None` if no app has focus.
    pub fn capture() -> Option<Self> {
        let (app, inner) = imp::capture_focus()?;
        Some(Self { app, inner })
    }

    /// Whether the same app and field still have focus.
    pub fn is_current(&self) -> bool {
        imp::is_current(&self.app, &self.inner)
    }

    /// Selected text in the focused field, via the accessibility API.
    pub fn selection(&self) -> Selection {
        imp::selection(&self.inner)
    }

    /// Whether the focused field is a password field.
    pub fn is_secure_field(&self) -> bool {
        imp::is_secure_field(&self.inner)
    }

    /// The field's current text, if the app exposes it (skipped for very
    /// long documents).
    pub fn field_text(&self) -> Option<String> {
        imp::field_text(&self.inner)
    }

    /// The field's whole text and where the insertion point is in it (a
    /// byte offset into the text), if the app reports both.
    pub fn text_and_cursor(&self) -> Option<(String, usize)> {
        let text = imp::field_text(&self.inner)?;
        let cursor = imp::cursor_offset(&self.inner)?;
        Some((text.clone(), utf16_to_byte(&text, cursor)?))
    }

    /// Selects the text between two byte offsets of `text_and_cursor`'s
    /// text. Returns whether the app did it.
    pub fn select(&self, text: &str, start: usize, end: usize) -> bool {
        let utf16 = |b: usize| text.get(..b).map(|s| s.encode_utf16().count());
        match (utf16(start), utf16(end)) {
            (Some(from), Some(to)) if to >= from => imp::select_range(&self.inner, from, to - from),
            _ => false,
        }
    }

    /// Where the insertion point is on screen, if the app reports it.
    pub fn caret_rect(&self) -> Option<CaretRect> {
        imp::caret_rect(&self.inner)
    }

    /// Up to `before` characters before the insertion point and `after`
    /// characters after it, with "…" where the text was cut. `None` for
    /// password fields, empty fields, or apps that don't expose their text.
    /// Without a reported cursor, the end of the text is used.
    pub fn text_around_cursor(&self, before: usize, after: usize) -> Option<String> {
        let text = self.field_text()?;
        let cursor = imp::cursor_offset(&self.inner);
        window_around(&text, cursor, before, after)
    }
}

/// The text within `before`/`after` characters of `cursor` (a UTF-16 offset,
/// as accessibility APIs report it), trimmed, with "…" marking cuts.
fn window_around(
    text: &str,
    cursor_utf16: Option<usize>,
    before: usize,
    after: usize,
) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    // UTF-16 offset → char index.
    let cursor = match cursor_utf16 {
        Some(target) => {
            let mut units = 0;
            chars
                .iter()
                .position(|c| {
                    let reached = units >= target;
                    units += c.len_utf16();
                    reached
                })
                .unwrap_or(chars.len())
        }
        None => chars.len(),
    };
    let start = cursor.saturating_sub(before);
    let end = (cursor + after).min(chars.len());
    let window: String = chars[start..end].iter().collect();
    let window = window.trim();
    if window.is_empty() {
        return None;
    }
    let before_mark = if start > 0 { "…" } else { "" };
    let after_mark = if end < chars.len() { "…" } else { "" };
    Some(format!("{before_mark}{window}{after_mark}"))
}

/// A rectangle on screen in points, with the origin at the top left of the
/// primary display (accessibility coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// ---------------------------------------------------------------- text

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InsertMethod {
    /// Clipboard + paste: fast and a single undo step.
    #[default]
    Paste,
    /// Simulated typing, for apps where paste misbehaves. With
    /// `soft_breaks`, line breaks are typed as Shift+Return, which chat apps
    /// take as a new line rather than "send"; terminals want plain Return.
    Type { soft_breaks: bool },
}

/// Inserts `text` into the focused field. Blocks for a few hundred ms
/// (paste waits before restoring the clipboard); don't call on the UI thread.
pub fn insert_text(text: &str, method: InsertMethod) -> Result<()> {
    match method {
        InsertMethod::Paste => imp::paste_text(text),
        InsertMethod::Type { soft_breaks } => imp::type_text(text, soft_breaks),
    }
}

/// Terminal apps, by bundle ID (macOS) or executable name (Windows). There
/// Shift+Return isn't a line break, so typed text uses plain Return.
const TERMINALS: &[&str] = &[
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "dev.warp.Warp-Stable",
    "com.mitchellh.ghostty",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "io.alacritty",
    "com.github.wez.wezterm",
    "co.zeit.hyper",
    "WindowsTerminal.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "conhost.exe",
    "OpenConsole.exe",
    "mintty.exe",
    "putty.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
];

/// Whether `app_id` is a terminal emulator.
pub fn is_terminal(app_id: &str) -> bool {
    TERMINALS.iter().any(|t| t.eq_ignore_ascii_case(app_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Return / Enter.
    Return,
    /// Cmd+Z on macOS, Ctrl+Z on Windows.
    Undo,
    /// Delete one character back (or the selection).
    Backspace,
    /// Delete one word back: Option+Delete on macOS, Ctrl+Backspace on
    /// Windows.
    DeleteWord,
    /// Cmd+B / Ctrl+B.
    Bold,
    /// Cmd+I / Ctrl+I.
    Italic,
    /// Right arrow: collapses a selection to its end.
    Right,
}

/// Presses a key in the focused app.
pub fn press(action: KeyAction) -> Result<()> {
    imp::press(action)
}

/// Re-reads the keyboard layout so synthetic Copy/Paste/Undo shortcuts use
/// the right keys (Dvorak, AZERTY…). Call on the main thread: at startup
/// and whenever the layout may have changed (e.g. as dictation starts).
pub fn refresh_shortcut_keys() {
    imp::refresh_shortcut_keys();
}

/// Puts `text` on the clipboard for the user to paste themselves.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    imp::copy_to_clipboard(text)
}

/// Whether the app is a Chromium-based browser, whose pastes are worth
/// double-checking.
pub fn is_chromium(app_id: &str) -> bool {
    imp::is_chromium(app_id)
}

/// The text on the clipboard now, if any.
pub fn clipboard_text() -> Option<String> {
    imp::clipboard_text()
}

/// Fallback selection read: simulates Copy and reads the clipboard, then
/// restores it. Only call when no dictation key is held.
pub fn copy_selection() -> Option<String> {
    imp::copy_selection()
}

/// Whether some app has turned on secure keyboard input (a password field
/// is focused). While it's on, key listeners receive nothing.
pub fn secure_input_active() -> bool {
    imp::secure_input_active()
}

// ---------------------------------------------------------------- permissions

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Granted,
    Denied,
    /// The user hasn't been asked yet.
    NotAsked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    /// Hearing the hotkey (an active key tap), reading the focused field and
    /// typing into it. On macOS this one permission covers all three; Input
    /// Monitoring is only needed by listen-only taps, which we don't use.
    pub accessibility: bool,
    pub microphone: Access,
}

impl Permissions {
    pub fn all_granted(&self) -> bool {
        self.accessibility && self.microphone == Access::Granted
    }
}

pub fn permissions() -> Permissions {
    imp::permissions()
}

/// Shows the system's Accessibility prompt, which also adds the app to the
/// list in System Settings so the user only has to switch it on.
pub fn request_accessibility() {
    imp::request_accessibility()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKind {
    Accessibility,
    Microphone,
}

/// Opens the system settings page where the user grants `kind`.
pub fn open_permission_settings(kind: PermissionKind) -> Result<()> {
    let url = if cfg!(target_os = "macos") {
        match kind {
            PermissionKind::Accessibility => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            PermissionKind::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
        }
    } else {
        // Windows only gates the microphone.
        "ms-settings:privacy-microphone"
    };
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        std::process::Command::new("open")
    };
    cmd.arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| PlatformError::Failed(e.to_string()))
}

/// Opens a web page in the default browser.
pub fn open_url(url: &str) -> Result<()> {
    // Not `cmd /C start` on Windows: cmd would read `&` and `%` in a query
    // string as its own syntax.
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("open")
    };
    cmd.arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| PlatformError::Failed(e.to_string()))
}

/// Shows `path` selected in Finder or File Explorer.
pub fn reveal_in_file_manager(path: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        // Explorer wants `/select,"C:\a b\c.md"` as written: Rust's usual
        // quoting of the whole argument confuses it, and so do the forward
        // slashes of a note in a subfolder.
        let path = path.to_string_lossy().replace('/', "\\");
        let mut c = std::process::Command::new("explorer");
        c.raw_arg(format!("/select,\"{path}\""));
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    };
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| PlatformError::Failed(e.to_string()))
}

/// The OS version, e.g. "26.0.1" (`None` where it isn't known).
pub fn os_version() -> Option<String> {
    imp::os_version()
}

/// An installed app as people know it.
#[derive(Debug, Clone)]
pub struct AppDisplay {
    pub name: String,
    /// A small square icon (PNG), if one could be drawn.
    pub icon_png: Option<Vec<u8>>,
}

/// The name and icon of the app with this id (a bundle ID on macOS), or
/// `None` if it isn't installed or can't be looked up here.
pub fn app_display(app_id: &str) -> Option<AppDisplay> {
    imp::app_display(app_id)
}

/// The app in front: its process id and name. Doesn't ask Accessibility,
/// so it's cheap and safe on the main thread, even when we are in front.
pub fn frontmost_app() -> Option<(i32, String)> {
    imp::frontmost_app()
}

/// Brings the app with this process id to the front, to hand focus back
/// after one of our windows had it. Call while our app is active (macOS
/// only lets the active app pass activation on).
pub fn activate_app(pid: i32) -> Result<()> {
    imp::activate_app(pid)
}

// ---------------------------------------------------------------- feedback

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// Recording started.
    Start,
    /// Recording stopped (processing begins).
    Stop,
}

/// Plays a short system sound at `volume` (0.0–1.0). Call on the main
/// thread.
pub fn play_sound(sound: Sound, volume: f32) {
    imp::play_sound(sound, volume);
}

/// The user asked the system to reduce motion (skip animations).
pub fn reduce_motion() -> bool {
    imp::reduce_motion()
}

// ---------------------------------------------------------------- output

/// How other sounds (music, videos, calls) are quieted while recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quieten {
    /// Down to a quarter of their volume.
    Lower,
    Mute,
}

/// The computer's sound output, lowered or muted while recording. Dropping
/// it puts the volume back, unless the user changed it meanwhile.
pub struct QuietOutput(u64);

/// What was lowered or muted, kept here rather than in `QuietOutput` so
/// `restore_output` can put it back when the app quits mid-recording. The
/// number says which `QuietOutput` it belongs to.
static QUIETED: std::sync::Mutex<Option<(u64, imp::QuietOutput)>> = std::sync::Mutex::new(None);
static QUIET_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn quieted() -> std::sync::MutexGuard<'static, Option<(u64, imp::QuietOutput)>> {
    QUIETED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Lowers or mutes the default output device. `None` when there's nothing
/// to do: no device or volume control, or it's already silent or muted.
pub fn quiet_output(how: Quieten) -> Option<QuietOutput> {
    // Put back anything still quieted first, so it isn't taken as the
    // level to return to.
    drop(quieted().take());
    let quiet = imp::quiet_output(how)?;
    let n = QUIET_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *quieted() = Some((n, quiet));
    Some(QuietOutput(n))
}

impl Drop for QuietOutput {
    fn drop(&mut self) {
        let mut slot = quieted();
        if slot.as_ref().is_some_and(|(n, _)| *n == self.0) {
            let quiet = slot.take();
            drop(slot);
            drop(quiet);
        }
    }
}

/// Puts back sound lowered or muted for a recording: for quitting or
/// restarting while one is in progress.
pub fn restore_output() {
    drop(quieted().take());
}

// ---------------------------------------------------------------- overlay

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Warning,
}

/// What the status pill shows.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayState {
    /// `preset` names a non-default preset in use ("Listening · Email").
    Listening {
        preset: Option<String>,
    },
    Instruction,
    Transform {
        words: usize,
    },
    HandsFree {
        preset: Option<String>,
    },
    /// Hands-free recording of the answer to a question (from a coding
    /// agent): shows the question.
    Question {
        text: String,
    },
    Working {
        label: String,
    },
    Message {
        tone: Tone,
        text: String,
    },
}

/// The floating status pill. Never takes focus and ignores the mouse.
/// Create and use it on the main (UI) thread only.
pub struct Overlay(imp::Overlay);

impl Overlay {
    pub fn new() -> Result<Self> {
        imp::Overlay::new().map(Self)
    }

    pub fn show(&self, state: &OverlayState) {
        self.0.show(state);
    }

    /// Feeds the live microphone level (0.0–1.0) to the waveform.
    pub fn set_level(&self, level: f32) {
        self.0.set_level(level);
    }

    /// Replaces the label with the words heard so far (only while a
    /// recording state is shown); `None` restores the state's label.
    pub fn set_live_text(&self, text: Option<&str>) {
        self.0.set_live_text(text);
    }

    /// Shows "↵ Send" after the label while auto-send is on. Cleared by
    /// the next recording, any message, or hiding.
    pub fn set_send(&self, on: bool) {
        self.0.set_send(on);
    }

    /// Shows the overlay just below `caret` (or above it near the bottom of
    /// the screen) instead of at the bottom of the screen. `None` restores
    /// the usual place.
    pub fn set_anchor(&self, caret: Option<CaretRect>) {
        self.0.set_anchor(caret);
    }

    pub fn hide(&self) {
        self.0.hide();
    }

    /// Opacity 0.0–1.0, for fades.
    pub fn set_alpha(&self, alpha: f64) {
        self.0.set_alpha(alpha);
    }

    /// Whether the pill is on screen.
    pub fn is_visible(&self) -> bool {
        self.0.is_visible()
    }

    /// Speaks `text` with VoiceOver (if it's running).
    pub fn announce(&self, text: &str) {
        self.0.announce(text);
    }
}

// ---------------------------------------------------------------- mini bar

/// What the mini bar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiniBarState {
    Idle,
    Recording,
    /// The recording is being transcribed and cleaned up.
    Working,
    /// Dictation isn't set up (permission missing, no provider).
    NotReady,
}

/// Where the mini bar is, in the platform's own screen coordinates (the
/// bottom-left corner in AppKit points on macOS, the top-left corner in
/// physical pixels on Windows). Only meant to be saved and handed back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiniBarPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MiniBarEvent {
    /// Pressed and released without dragging.
    Click,
    /// Dragged somewhere else; save this to put it back there next time.
    Moved(MiniBarPosition),
    /// An item from its right-click menu, by index in the list given to
    /// `MiniBar::new`.
    Menu(usize),
}

/// Called on the main thread for each event; must not block.
pub type MiniBarHandler = Box<dyn Fn(MiniBarEvent) + 'static>;

/// A small round button that stays on screen, for starting and finishing
/// dictation with the mouse. Unlike the overlay it takes clicks, but it
/// still never takes focus: clicking it leaves the user's app active and
/// its text field focused. It can be dragged anywhere. Create and use it on
/// the main (UI) thread only.
pub struct MiniBar(imp::MiniBar);

impl MiniBar {
    /// Creates the bar, hidden, at `position` (or a default place by the
    /// right edge of the screen when it's unset or no longer on a screen).
    /// `menu` lists the right-click menu's items (macOS only for now).
    pub fn new(
        position: Option<MiniBarPosition>,
        menu: &[&str],
        handler: MiniBarHandler,
    ) -> Result<Self> {
        imp::MiniBar::new(position, menu, handler).map(Self)
    }

    pub fn show(&self) {
        self.0.show();
    }

    pub fn hide(&self) {
        self.0.hide();
    }

    pub fn set_state(&self, state: MiniBarState) {
        self.0.set_state(state);
    }

    /// The hint shown when the pointer rests on the bar, also read out by
    /// screen readers.
    pub fn set_tooltip(&self, text: &str) {
        self.0.set_tooltip(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_map_to_bytes() {
        let text = "hi 👋 there";
        assert_eq!(utf16_to_byte(text, 0), Some(0));
        assert_eq!(utf16_to_byte(text, 3), Some(3));
        // The emoji is two UTF-16 units and four bytes.
        assert_eq!(utf16_to_byte(text, 5), Some(7));
        assert_eq!(utf16_to_byte(text, 11), Some(text.len()));
        assert_eq!(utf16_to_byte(text, 12), None);
    }

    #[test]
    fn file_urls_become_paths() {
        assert_eq!(
            file_url_path("file:///Users/me/My%20Project/"),
            Some("/Users/me/My Project/".into())
        );
        assert_eq!(
            file_url_path("file://localhost/tmp/a.rs"),
            Some("/tmp/a.rs".into())
        );
        assert_eq!(file_url_path("https://example.com"), None);
    }

    #[test]
    fn window_around_the_cursor() {
        let text = "Priya: can we move the review? Also loop in Anselm.";
        // Cursor at the end (or unknown): the tail.
        assert_eq!(window_around(text, None, 10, 5).unwrap(), "…in Anselm.");
        // Cursor after "Priya:" (6 UTF-16 units).
        assert_eq!(
            window_around(text, Some(6), 6, 8).unwrap(),
            "Priya: can we…"
        );
        // Emoji count as two UTF-16 units.
        assert_eq!(window_around("👋 hi there", Some(5), 3, 0).unwrap(), "…hi…");
        assert_eq!(window_around("   ", None, 10, 10), None);
    }

    #[test]
    fn terminals_by_bundle_id_or_executable() {
        assert!(is_terminal("com.apple.Terminal"));
        assert!(is_terminal("WINDOWSTERMINAL.EXE"));
        assert!(!is_terminal("com.tinyspeck.slackmacgap"));
        assert!(!is_terminal("Code.exe"));
    }
}
