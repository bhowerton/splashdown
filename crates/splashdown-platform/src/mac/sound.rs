//! Start and stop sounds, and the system's reduce-motion setting. The Mac
//! twin of `win/sound.rs`.

use objc2_app_kit::{NSSound, NSWorkspace};
use objc2_foundation::NSString;

use crate::Sound;

/// Quiet system sounds: "Tink" when recording starts, "Pop" when it stops.
pub(crate) fn play_sound(sound: Sound, volume: f32) {
    let name = match sound {
        Sound::Start => "Tink",
        Sound::Stop => "Pop",
    };
    if let Some(s) = NSSound::soundNamed(&NSString::from_str(name)) {
        s.setVolume(volume.clamp(0.0, 1.0));
        s.play();
    }
}

pub(crate) fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}
