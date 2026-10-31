//! Windows backend: WH_KEYBOARD_LL hotkeys, UI Automation for focus and
//! selection, SendInput / clipboard insertion, and a WS_EX_NOACTIVATE
//! layered overlay window.
//!
//! On other non-macOS systems (where nothing is implemented) the stub in
//! `unsupported.rs` keeps the shared crates building.

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
mod sound;
#[cfg(windows)]
mod uia;

#[cfg(windows)]
pub(crate) use clipboard::{
    clipboard_text, copy_selection, copy_to_clipboard, paste_text, press, refresh_shortcut_keys,
    type_text,
};
#[cfg(windows)]
pub(crate) use keys::KeyListener;
#[cfg(windows)]
pub(crate) use minibar::MiniBar;
#[cfg(windows)]
pub(crate) use output::{QuietOutput, quiet_output};
#[cfg(windows)]
pub(crate) use overlay::Overlay;
#[cfg(windows)]
pub(crate) use sound::{play_sound, reduce_motion};
#[cfg(windows)]
pub(crate) use uia::{
    FocusElement, activate_app, app_display, capture_focus, cursor_offset, field_text,
    frontmost_app, is_chromium, is_current, is_secure_field, os_version, permissions,
    request_accessibility, secure_input_active, select_range, selection,
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

/// The caret's screen rectangle. Not read on Windows yet (the overlay
/// doesn't follow the caret there), so it stays in its usual place.
#[cfg(windows)]
pub(crate) fn caret_rect(_el: &FocusElement) -> Option<crate::CaretRect> {
    None
}

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
