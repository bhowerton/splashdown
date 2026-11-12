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
//!
//! The cross-platform API is split by job: `focus.rs` (the focused field),
//! `apps.rs` (other apps, links), `quiet.rs` (other sounds while
//! recording), `ui.rs` (the pill and the mini bar), and this file (keys,
//! inserting text, permissions, media).
//! Gesture rules (hold, double-tap, chords) live in `splashdown_core::gesture`, so
//! this crate only reports raw key events.

pub use splashdown_core::gesture::{KeyEvent, PhysicalKey};

mod apps;
mod focus;
mod quiet;
mod ui;

pub use apps::*;
pub use focus::*;
pub use quiet::*;
pub use ui::*;

#[cfg(target_os = "macos")]
#[path = "mac/mod.rs"]
mod imp;
mod launch;
mod machine;
mod players;

pub use launch::{InstalledApp, installed_apps, open_app};

/// A native alert, and whether an app is open from a given bundle: for
/// offering to move the app to the Applications folder at start-up. And
/// making this app the active one, so a window it opens comes to the front.
#[cfg(target_os = "macos")]
pub use imp::{activate_self, alert, app_running_at};
pub use machine::{free_disk_bytes, memory_bytes, system_language};

/// Makes this app the active one (macOS only; elsewhere nothing to do).
#[cfg(not(target_os = "macos"))]
pub fn activate_self() {}

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

/// Whether `key` is physically down right now, where the system can say:
/// `None` for a key it can't be asked about, or on a system without a way
/// to ask. For noticing a release the key listener never got (the screen
/// locked, a password prompt took the keyboard).
pub fn key_is_down(key: PhysicalKey) -> Option<bool> {
    imp::key_is_down(key)
}

// ---------------------------------------------------------------- focus

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

/// Shows the system's microphone prompt (macOS) and waits for the answer:
/// whether it was allowed. None where there's no such prompt (Windows),
/// where opening the microphone is what asks.
pub fn request_microphone() -> Option<bool> {
    imp::request_microphone()
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

/// Recording one app's sound (Windows); it stops when dropped.
#[cfg(windows)]
pub use imp::AppLoopback;

// ---------------------------------------------------------------- overlay

// ---------------------------------------------------------------- mini bar

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
