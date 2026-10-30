//! macOS backend: CGEventTap hotkeys, Accessibility (AX) for focus and
//! selection, CGEvent / pasteboard insertion, non-activating NSPanel overlay.

mod apps;
mod ax;
mod clipboard;
mod ffi;
mod keys;
mod minibar;
mod overlay;

pub(crate) use apps::{activate_app, app_display, frontmost_app};
pub(crate) use ax::{
    FocusElement, capture_focus, caret_rect, cursor_offset, field_text, is_chromium, is_current,
    is_secure_field, permissions, request_accessibility, secure_input_active, select_range,
    selection,
};
pub(crate) use clipboard::{
    clipboard_text, copy_selection, copy_to_clipboard, paste_text, press, refresh_shortcut_keys,
    type_text,
};
pub(crate) use keys::KeyListener;
pub(crate) use minibar::MiniBar;
pub(crate) use overlay::{Overlay, play_sound, reduce_motion};

pub(crate) const NAME: &str = "macos";

pub(crate) fn os_version() -> Option<String> {
    let v = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
    Some(format!(
        "{}.{}.{}",
        v.majorVersion, v.minorVersion, v.patchVersion
    ))
}
