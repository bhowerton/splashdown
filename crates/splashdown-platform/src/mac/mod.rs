//! macOS backend: CGEventTap hotkeys, Accessibility (AX) for focus and
//! selection, CGEvent / pasteboard insertion, non-activating NSPanel overlay.

mod alert;
mod apps;
mod ax;
mod clipboard;
pub(crate) mod computer;
mod ffi;
mod keys;
mod media;
mod mic;
mod minibar;
mod output;
mod overlay;
mod sound;

pub use alert::{activate_self, alert, app_running_at};
pub(crate) use apps::{activate_app, app_display, frontmost_app};
pub(crate) use ax::{
    FocusElement, capture_focus, caret_rect, cursor_offset, element_at, field_text, is_chromium,
    is_current, is_secure_field, nothing_like_text, permissions, request_accessibility,
    request_microphone, secure_input_active, select_range, selection, surrounding_text, takes_text,
};
pub(crate) use clipboard::{
    clipboard_text, copy_selection, copy_to_clipboard, paste_text, press, refresh_shortcut_keys,
    type_text,
};
pub(crate) use keys::{KeyListener, key_is_down};
pub(crate) use media::{pause_players, press_media_key, resume_players};
pub(crate) use mic::{audio_apps, microphone_users};
pub(crate) use minibar::MiniBar;
pub(crate) use output::{
    QuietOutput, can_quiet_output, output_volume, quiet_output, set_output_muted, set_output_volume,
};
pub(crate) use overlay::Overlay;
pub(crate) use sound::{play_sound, reduce_motion};

pub(crate) const NAME: &str = "macos";

pub(crate) fn press_combo(combo: &splashdown_core::computer::Combo) -> crate::Result<()> {
    computer::key(combo, 1)
}

pub(crate) fn os_version() -> Option<String> {
    let v = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
    Some(format!(
        "{}.{}.{}",
        v.majorVersion, v.minorVersion, v.patchVersion
    ))
}
