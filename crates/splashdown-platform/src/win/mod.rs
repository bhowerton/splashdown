//! Windows backend: WH_KEYBOARD_LL hotkeys, UI Automation for focus and
//! selection, SendInput / clipboard insertion, and a WS_EX_NOACTIVATE
//! layered overlay window.
//!
//! On other non-macOS systems (where nothing is implemented) the stub in
//! `unsupported.rs` keeps the shared crates building.

#[cfg(windows)]
mod app_audio;
#[cfg(windows)]
mod clipboard;
#[cfg(windows)]
mod keys;
#[cfg(windows)]
mod minibar;
#[cfg(windows)]
mod output;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
mod sound;
#[cfg(windows)]
pub mod speech;
#[cfg(windows)]
mod uia;

#[cfg(windows)]
pub use app_audio::AppLoopback;
#[cfg(windows)]
pub(crate) use app_audio::{app_loopback, audio_apps};
#[cfg(windows)]
pub(crate) use clipboard::{
    clipboard_text, copy_selection, copy_to_clipboard, paste_text, press, press_combo,
    press_media_key, refresh_shortcut_keys, type_text,
};
#[cfg(windows)]
pub(crate) use keys::{KeyListener, altgr_types};
#[cfg(windows)]
pub(crate) use minibar::MiniBar;
#[cfg(windows)]
pub(crate) use output::{
    QuietOutput, can_quiet_output, output_volume, pause_players, quiet_output, resume_players,
    set_output_muted, set_output_volume,
};
#[cfg(windows)]
pub(crate) use overlay::Overlay;
#[cfg(windows)]
pub(crate) use shell::shell_open;
#[cfg(windows)]
pub(crate) use sound::{play_sound, reduce_motion};
#[cfg(windows)]
pub(crate) use uia::{
    FocusElement, activate_app, app_display, capture_focus, caret_rect, cursor_offset, field_text,
    frontmost_app, is_chromium, is_current, is_secure_field, microphone_users, os_version,
    permissions, request_accessibility, running_apps, secure_input_active, select_range, selection,
    surrounding_text,
};

#[cfg(windows)]
pub(crate) const NAME: &str = "windows";

/// `dwExtraInfo` on every key event we synthesize, so our own hook can tell
/// them apart from the user's typing ("SPLD").
#[cfg(windows)]
const INJECTED_TAG: usize = 0x5350_4C44;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
pub(crate) use unsupported::*;

/// UTF-16 with a trailing NUL, for Win32 string parameters.
#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A UTF-16 buffer up to its first NUL (or its end).
#[cfg(windows)]
fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
