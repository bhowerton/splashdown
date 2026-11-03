//! Stub backend for systems that are neither macOS nor Windows: everything
//! reports `Unsupported`, so the shared crates still build there.

use crate::{
    Access, AppInfo, KeyHandler, OverlayState, Permissions, PlatformError, Result, Selection,
};

pub(crate) const NAME: &str = "unsupported";

pub(crate) struct KeyListener;

impl KeyListener {
    pub(crate) fn start(_handler: KeyHandler) -> Result<Self> {
        Err(PlatformError::Unsupported)
    }
}

pub(crate) struct FocusElement;

pub(crate) fn capture_focus() -> Option<(AppInfo, FocusElement)> {
    None
}

pub(crate) fn is_current(_app: &AppInfo, _el: &FocusElement) -> bool {
    false
}

pub(crate) fn selection(_el: &FocusElement) -> Selection {
    Selection::Unknown
}

pub(crate) fn is_secure_field(_el: &FocusElement) -> bool {
    false
}

pub(crate) fn surrounding_text(_el: &FocusElement, _min: usize, _max: usize) -> Option<String> {
    None
}

pub(crate) fn field_text(_el: &FocusElement) -> Option<String> {
    None
}

pub(crate) fn cursor_offset(_el: &FocusElement) -> Option<usize> {
    None
}

pub(crate) fn caret_rect(_el: &FocusElement) -> Option<crate::CaretRect> {
    None
}

pub(crate) fn select_range(_el: &FocusElement, _location: usize, _length: usize) -> bool {
    false
}

pub(crate) fn paste_text(_text: &str) -> Result<()> {
    Err(PlatformError::Unsupported)
}

pub(crate) fn press(_action: crate::KeyAction) -> Result<()> {
    Err(PlatformError::Unsupported)
}

pub(crate) fn type_text(_text: &str, _soft_breaks: bool) -> Result<()> {
    Err(PlatformError::Unsupported)
}

pub(crate) fn copy_to_clipboard(_text: &str) -> Result<()> {
    Err(PlatformError::Unsupported)
}

pub(crate) fn is_chromium(_app_id: &str) -> bool {
    false
}

pub(crate) fn clipboard_text() -> Option<String> {
    None
}

pub(crate) fn copy_selection() -> Option<String> {
    None
}

pub(crate) fn secure_input_active() -> bool {
    false
}

pub(crate) fn permissions() -> Permissions {
    // Nothing here can be granted, so don't send the user to look for it.
    Permissions {
        accessibility: true,
        microphone: Access::Granted,
    }
}

pub(crate) fn request_accessibility() {}

pub(crate) fn refresh_shortcut_keys() {}

pub(crate) fn play_sound(_sound: crate::Sound, _volume: f32) {}

pub(crate) fn os_version() -> Option<String> {
    None
}

pub(crate) fn app_display(_app_id: &str) -> Option<crate::AppDisplay> {
    None
}

pub(crate) fn frontmost_app() -> Option<(i32, String, String)> {
    None
}

pub(crate) fn activate_app(_pid: i32) -> Result<()> {
    Err(PlatformError::Unsupported)
}

pub(crate) fn reduce_motion() -> bool {
    false
}

pub(crate) struct Overlay;

impl Overlay {
    pub(crate) fn new() -> Result<Self> {
        Err(PlatformError::Unsupported)
    }
    pub(crate) fn show(&self, _state: &OverlayState) {}
    pub(crate) fn set_level(&self, _level: f32) {}
    pub(crate) fn set_live_text(&self, _text: Option<&str>) {}
    pub(crate) fn set_send(&self, _on: bool) {}
    pub(crate) fn set_anchor(&self, _caret: Option<crate::CaretRect>) {}
    pub(crate) fn set_alpha(&self, _alpha: f64) {}
    pub(crate) fn is_visible(&self) -> bool {
        false
    }
    pub(crate) fn announce(&self, _text: &str) {}
    pub(crate) fn hide(&self) {}
}

pub(crate) struct MiniBar;

impl MiniBar {
    pub(crate) fn new(
        _position: Option<crate::MiniBarPosition>,
        _menu: &[&str],
        _handler: crate::MiniBarHandler,
    ) -> Result<Self> {
        Err(PlatformError::Unsupported)
    }
    pub(crate) fn show(&self) {}
    pub(crate) fn hide(&self) {}
    pub(crate) fn set_state(&self, _state: crate::MiniBarState) {}
    pub(crate) fn set_tooltip(&self, _text: &str) {}
}

pub(crate) struct QuietOutput;

pub(crate) fn quiet_output(_how: crate::Quieten) -> Option<QuietOutput> {
    None
}

pub(crate) fn can_quiet_output(_how: crate::Quieten) -> bool {
    true
}

pub(crate) fn pause_players() -> Vec<String> {
    Vec::new()
}

pub(crate) fn resume_players(_players: &[String]) {}

pub(crate) fn microphone_users() -> Vec<String> {
    Vec::new()
}

pub(crate) fn audio_apps() -> Vec<crate::AudioApp> {
    Vec::new()
}

pub(crate) fn press_combo(_combo: &splashdown_core::computer::Combo) -> Result<()> {
    Err(PlatformError::Unsupported)
}

pub(crate) fn press_media_key(_key: crate::MediaKey) -> Result<()> {
    Err(PlatformError::Unsupported)
}

pub(crate) fn output_volume() -> Option<f32> {
    None
}

pub(crate) fn set_output_volume(_level: f32) -> bool {
    false
}

pub(crate) fn set_output_muted(_muted: bool) -> bool {
    false
}

pub(crate) fn altgr_types(_letter: char) -> bool {
    false
}

pub(crate) fn running_apps() -> Vec<(String, i32)> {
    Vec::new()
}
