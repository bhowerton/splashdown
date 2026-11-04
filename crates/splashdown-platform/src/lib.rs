//! OS integration: global hotkeys, text insertion, focus and selection
//! context, the status overlay, and permission checks.
//!
//! The API is the same on every platform; `mac/` and `win/` implement it,
//! file for file, so a fix on one leads to its twin on the other:
//!
//! | What | Mac | Windows |
//! |---|---|---|
//! | The focused field, its text and selection, permissions | `ax.rs` | `uia.rs` |
//! | Apps: the one in front, switching, names and icons | `apps.rs` | `apps.rs` |
//! | The key hook | `keys.rs` | `keys.rs` |
//! | The clipboard, pasting and typing, key presses | `clipboard.rs` | `clipboard.rs` |
//! | The microphone: permission, apps using it | `mic.rs` | `mic.rs` |
//! | Apps playing sound, and recording one | `mic.rs`, the Apple shim | `app_audio.rs` |
//! | Media keys, pausing players | `media.rs` | `media.rs` |
//! | Quieting the output while recording | `output.rs` | `output.rs` |
//! | The status pill | `overlay.rs` | `overlay.rs` |
//! | The mini bar | `minibar.rs` | `minibar.rs` |
//! | Start and stop sounds | `sound.rs` | `sound.rs` |
//! | Reading aloud with the system's voices | the Apple shim | `speech.rs` |
//! | Desktop control (an experiment) | `computer.rs` | none |
//!
//! Windows also has `registry.rs` and `shell.rs`; macOS has `ffi.rs`
//! (Accessibility and Core Graphics declarations). Other OSes get the
//! stubs in `win/unsupported.rs`.
//! Gesture rules (hold, double-tap, chords) live in `splashdown_core::gesture`, so
//! this crate only reports raw key events.

pub use splashdown_core::gesture::{KeyEvent, PhysicalKey};

#[cfg(target_os = "macos")]
#[path = "mac/mod.rs"]
mod imp;
mod launch;
mod machine;
mod players;

pub use launch::{InstalledApp, installed_apps, open_app};
pub use machine::{free_disk_bytes, memory_bytes};

/// Reading text aloud with Windows' own voices (on macOS it's the Apple
/// shim's).
#[cfg(windows)]
pub use imp::speech;

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

/// The last `chars` characters of `text`, starting at a line or word
/// where there is one close by.
pub(crate) fn keep_end(text: &str, chars: usize) -> String {
    let count = text.chars().count();
    if count <= chars {
        return text.to_string();
    }
    let start = text.char_indices().nth(count - chars).map_or(0, |(i, _)| i);
    let tail = &text[start..];
    // A clean start: the next line, or else the next word, if it's near.
    let cut = tail
        .find('\n')
        .filter(|&i| i < 200)
        .or_else(|| tail.find(' ').filter(|&i| i < 40))
        .map_or(0, |i| i + 1);
    tail[cut..].to_string()
}

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

    /// The text around the field in its window, besides the field's own:
    /// usually the conversation being answered (an email thread, a chat).
    /// Read from the nearest container holding at least `min_chars`, and
    /// at most the last `max_chars` of it. `None` for password fields, or
    /// where the app exposes no text.
    pub fn surrounding_text(&self, min_chars: usize, max_chars: usize) -> Option<String> {
        imp::surrounding_text(&self.inner, min_chars, max_chars)
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
    open_url(url)
}

/// Whether AltGr plus `letter` types a character on one of the user's
/// keyboard layouts (Windows), where a Ctrl+Alt shortcut on it would take
/// the character away. Always false on macOS, which has no AltGr.
pub fn altgr_types(letter: char) -> bool {
    #[cfg(windows)]
    {
        imp::altgr_types(letter)
    }
    #[cfg(not(windows))]
    {
        let _ = letter;
        false
    }
}

// ---------------------------------------------------------------- computer use

/// The control at a point on the screen, as Accessibility describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ElementInfo {
    /// "AXButton", "AXTextField", "AXMenuItem"…
    pub role: String,
    /// A password field.
    pub secure: bool,
    /// What it says: its title or description, or the nearest around it.
    pub label: String,
    /// The app it belongs to.
    pub app: String,
}

/// Driving the mouse and keyboard for computer use (an exploration, macOS
/// only so far). Coordinates are screen points.
pub mod computer {
    use splashdown_core::computer::{Button, Combo, Direction, Mods};

    use crate::{ElementInfo, Result};

    #[cfg(target_os = "macos")]
    use crate::imp::computer as imp;

    #[cfg(not(target_os = "macos"))]
    mod imp {
        use super::*;
        use crate::PlatformError;
        pub(crate) fn cursor() -> Option<(f64, f64)> {
            None
        }
        pub(crate) fn move_to(_: (f64, f64)) -> Result<()> {
            Err(PlatformError::Unsupported)
        }
        pub(crate) fn click(_: (f64, f64), _: Button, _: u8, _: Mods) -> Result<()> {
            Err(PlatformError::Unsupported)
        }
        pub(crate) fn left_button(_: bool) -> Result<()> {
            Err(PlatformError::Unsupported)
        }
        pub(crate) fn drag(_: (f64, f64), _: (f64, f64), _: Mods) -> Result<()> {
            Err(PlatformError::Unsupported)
        }
        pub(crate) fn scroll(_: Option<(f64, f64)>, _: Direction, _: u32, _: Mods) -> Result<()> {
            Err(PlatformError::Unsupported)
        }
        pub(crate) fn key(_: &Combo, _: u32) -> Result<()> {
            Err(PlatformError::Unsupported)
        }
        pub(crate) fn hold(_: &Combo, _: f64) -> Result<()> {
            Err(PlatformError::Unsupported)
        }
    }

    pub fn cursor() -> Option<(f64, f64)> {
        imp::cursor()
    }
    pub fn move_to(at: (f64, f64)) -> Result<()> {
        imp::move_to(at)
    }
    pub fn click(at: (f64, f64), button: Button, count: u8, mods: Mods) -> Result<()> {
        imp::click(at, button, count, mods)
    }
    pub fn left_button(down: bool) -> Result<()> {
        imp::left_button(down)
    }
    pub fn drag(from: (f64, f64), to: (f64, f64), mods: Mods) -> Result<()> {
        imp::drag(from, to, mods)
    }
    pub fn scroll(
        at: Option<(f64, f64)>,
        direction: Direction,
        amount: u32,
        mods: Mods,
    ) -> Result<()> {
        imp::scroll(at, direction, amount, mods)
    }
    pub fn key(combo: &Combo, repeat: u32) -> Result<()> {
        imp::key(combo, repeat)
    }
    pub fn hold(combo: &Combo, seconds: f64) -> Result<()> {
        imp::hold(combo, seconds)
    }

    /// The control at a point (screen points).
    pub fn element_at(x: f64, y: f64) -> Option<ElementInfo> {
        #[cfg(target_os = "macos")]
        {
            crate::imp::element_at(x, y)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (x, y);
            None
        }
    }
}

// ---------------------------------------------------------------- media

/// A media key, as on a keyboard: whichever app is playing answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKey {
    PlayPause,
    Next,
    Previous,
}

/// Presses a key combination ("cmd+shift+4", "right") in the app in
/// front: one of the user's own voice actions.
pub fn press_combo(combo: &splashdown_core::computer::Combo) -> Result<()> {
    imp::press_combo(combo)
}

pub fn press_media_key(key: MediaKey) -> Result<()> {
    // Music paused for the recording that asked for this is played again
    // first, so "pause the music" pauses it rather than starting it.
    players::settle(std::time::Duration::from_secs(2));
    imp::press_media_key(key)
}

/// The default output device's volume, 0.0–1.0.
pub fn output_volume() -> Option<f32> {
    imp::output_volume()
}

/// Sets the default output's volume (0.0–1.0), unmuting it when it goes
/// above nothing.
pub fn set_output_volume(level: f32) -> Result<()> {
    if imp::set_output_volume(level) {
        Ok(())
    } else {
        Err(PlatformError::Failed(
            "the sound's volume can't be changed".into(),
        ))
    }
}

pub fn set_output_muted(muted: bool) -> Result<()> {
    if imp::set_output_muted(muted) {
        Ok(())
    } else {
        Err(PlatformError::Failed("the sound can't be muted".into()))
    }
}

/// Opens a web page in the default browser.
/// On Windows, `ShellExecuteW` (no console, and any characters in a path),
/// which also opens `ms-settings:` pages and files such as an `.ics`.
pub fn open_url(url: &str) -> Result<()> {
    #[cfg(windows)]
    {
        imp::shell_open(url)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| PlatformError::Failed(e.to_string()))
    }
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

/// The app in front: its process id, name, and id (bundle ID on macOS,
/// executable name on Windows). Doesn't ask Accessibility, so it's cheap
/// and safe on the main thread, even when we are in front.
pub fn frontmost_app() -> Option<(i32, String, String)> {
    imp::frontmost_app()
}

/// Brings the app with this process id to the front, to hand focus back
/// after one of our windows had it. Call while our app is active (macOS
/// only lets the active app pass activation on).
pub fn activate_app(pid: i32) -> Result<()> {
    imp::activate_app(pid)
}

/// Apps with a window open on Windows, by name, with their process ids
/// (for "switch to …"). Empty on macOS, where opening an app that's
/// already running brings it forward anyway.
pub fn running_apps() -> Vec<(String, i32)> {
    #[cfg(windows)]
    {
        imp::running_apps()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
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
    /// Players that are playing are paused, and played again afterwards:
    /// Spotify and Music on macOS, any app showing media controls on
    /// Windows.
    Pause,
}

/// Other sounds, quieted while recording. Dropping it puts them back:
/// the volume (unless the user changed it meanwhile), or the music.
pub struct QuietOutput(u64);

/// Only ever dropped: that puts back what it quieted.
#[allow(dead_code)]
enum Quieted {
    Output(imp::QuietOutput),
    Paused(players::Paused),
}

/// What was quieted, kept here rather than in `QuietOutput` so
/// `restore_output` can put it back when the app quits mid-recording. The
/// number says which `QuietOutput` it belongs to.
static QUIETED: std::sync::Mutex<Option<(u64, Quieted)>> = std::sync::Mutex::new(None);
static QUIET_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn quieted() -> std::sync::MutexGuard<'static, Option<(u64, Quieted)>> {
    QUIETED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Other apps recording from a microphone right now, by bundle ID on
/// macOS (14 or later; none on earlier versions) or by executable name or
/// package family name on Windows. Our own process isn't included.
pub fn microphone_users() -> Vec<String> {
    imp::microphone_users()
}

/// An app that plays sound, to transcribe what it plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioApp {
    /// Its bundle ID (macOS) or executable's file name (Windows).
    pub id: String,
    pub name: String,
    /// Playing sound now.
    pub playing: bool,
    /// What to record: the app's Core Audio process objects (macOS, for
    /// the shim's tap), or the process whose tree plays its sound (Windows,
    /// for `app_loopback`).
    pub sources: Vec<u32>,
}

/// The apps that play (or have played) sound since they started, playing
/// ones first. Our own app isn't included.
pub fn audio_apps() -> Vec<AudioApp> {
    imp::audio_apps()
}

/// Recording one app's sound (Windows); it stops when dropped.
#[cfg(windows)]
pub use imp::AppLoopback;

/// Where recorded sound goes: mono samples and their rate, on an audio
/// thread.
pub type SoundSink = Box<dyn FnMut(&[f32], u32) + Send>;

/// Records what process `pid` and the processes it started play (one of
/// `AudioApp::sources`), handing mono samples and their rate to `sink` as
/// they come. Windows 10 2004 or later.
#[cfg(windows)]
pub fn app_loopback(pid: u32, sink: SoundSink) -> Result<AppLoopback> {
    imp::app_loopback(pid, sink)
}

/// Lowers or mutes the default output device, or pauses music. A device
/// whose volume can't be changed (some audio interfaces) gets its music
/// paused instead. `None` when there's nothing to do: no device, or it's
/// already silent or muted.
pub fn quiet_output(how: Quieten) -> Option<QuietOutput> {
    // Put back anything still quieted first, so it isn't taken as the
    // level to return to.
    drop(quieted().take());
    let quiet = match how {
        Quieten::Pause => Quieted::Paused(players::pause()),
        Quieten::Lower | Quieten::Mute => match imp::quiet_output(how) {
            Some(quiet) => Quieted::Output(quiet),
            None if !imp::can_quiet_output(how) => {
                tracing::info!("the sound's volume can't be changed; pausing music instead");
                Quieted::Paused(players::pause())
            }
            None => return None,
        },
    };
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
    players::settle(std::time::Duration::from_secs(3));
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
    /// Text being read aloud.
    Reading {
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
    #[test]
    fn keeps_the_end_from_a_clean_start() {
        assert_eq!(super::keep_end("short", 100), "short");
        let text = "first line\nsecond line\nthird line";
        assert_eq!(super::keep_end(text, 15), "third line");
        assert_eq!(super::keep_end("one two three four", 9), "four");
    }

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
