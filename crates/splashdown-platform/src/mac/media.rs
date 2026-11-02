//! Media keys (play/pause, next, previous) for voice actions: the same
//! system-defined events the keyboard's media keys send, so whichever app
//! is playing answers them. And pausing music while recording, by asking
//! the players themselves: a media key can't tell whether anything is
//! playing, and would start the music when nothing is.

use std::ffi::c_void;

use objc2::encode::{Encode, Encoding};
use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSPoint;

use crate::{MediaKey, PlatformError, Result};

/// A `CGEventRef`: a pointer to CoreGraphics' opaque event struct.
#[repr(transparent)]
#[derive(Clone, Copy)]
struct CgEventRef(*mut c_void);

// SAFETY: the encoding of `CGEventRef`, a pointer to `struct __CGEvent`.
unsafe impl Encode for CgEventRef {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("__CGEvent", &[]));
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventPost(tap: u32, event: *mut c_void);
}

/// `NSEventTypeSystemDefined`.
const SYSTEM_DEFINED: usize = 14;
/// `NSSystemDefined` subtype for the keyboard's auxiliary keys.
const AUX_CONTROL_BUTTONS: i16 = 8;
/// `kCGHIDEventTap`.
const HID_EVENT_TAP: u32 = 0;
/// IOKit's `NX_KEYTYPE_*`.
const NX_PLAY: isize = 16;
const NX_NEXT: isize = 17;
const NX_PREVIOUS: isize = 18;

pub(crate) fn press_media_key(key: MediaKey) -> Result<()> {
    let code = match key {
        MediaKey::PlayPause => NX_PLAY,
        MediaKey::Next => NX_NEXT,
        MediaKey::Previous => NX_PREVIOUS,
    };
    post(code, true)?;
    post(code, false)
}

fn post(code: isize, down: bool) -> Result<()> {
    with_event(code, down, |cg| {
        // SAFETY: a valid event, posted to the HID stream.
        unsafe { CGEventPost(HID_EVENT_TAP, cg.0) };
    })
}

/// Makes the key's event and hands it to `use_it` while it's alive.
fn with_event(code: isize, down: bool, use_it: impl FnOnce(CgEventRef)) -> Result<()> {
    let state: isize = if down { 0xa } else { 0xb };
    autoreleasepool(|_| {
        // SAFETY: NSEvent's documented factory for system-defined events,
        // with plain values; it returns an autoreleased event or nil.
        let event: *mut AnyObject = unsafe {
            msg_send![
                class!(NSEvent),
                otherEventWithType: SYSTEM_DEFINED,
                location: NSPoint::new(0.0, 0.0),
                modifierFlags: (state << 8) as usize,
                timestamp: 0.0f64,
                windowNumber: 0isize,
                context: std::ptr::null_mut::<AnyObject>(),
                subtype: AUX_CONTROL_BUTTONS,
                data1: (code << 16) | (state << 8),
                data2: -1isize
            ]
        };
        if event.is_null() {
            return Err(PlatformError::Failed(
                "couldn't make a media key event".into(),
            ));
        }
        // SAFETY: `event` is a live NSEvent; `CGEvent` returns one it owns
        // (valid while the event is, within this pool).
        let cg: CgEventRef = unsafe { msg_send![event, CGEvent] };
        if cg.0.is_null() {
            return Err(PlatformError::Failed(
                "couldn't make a media key event".into(),
            ));
        }
        use_it(cg);
        Ok(())
    })
}

/// Players that can be asked whether they're playing (AppleScript), by
/// bundle ID. The first time, macOS asks the user whether Splashdown may
/// control each one.
const PLAYERS: [&str; 2] = ["com.spotify.client", "com.apple.Music"];

/// Pauses the players that are playing, and returns them.
pub(crate) fn pause_players() -> Vec<String> {
    PLAYERS
        .into_iter()
        .filter(|id| running(id))
        .filter(|id| {
            ask(
                id,
                "if player state is playing then\npause\nreturn \"paused\"\nend if",
            )
            .is_some_and(|said| said == "paused")
        })
        .map(String::from)
        .collect()
}

/// Plays again the players `pause_players` paused, unless they were
/// started, stopped or quit meanwhile.
pub(crate) fn resume_players(players: &[String]) {
    for id in players.iter().filter(|id| running(id)) {
        ask(id, "if player state is paused then play");
    }
}

/// Whether the app is running: asking one that isn't would open it.
fn running(id: &str) -> bool {
    objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &objc2_foundation::NSString::from_str(id),
    )
    .count()
        > 0
}

/// Runs `script` inside a `tell` to the app, returning what it returns.
fn ask(id: &str, script: &str) -> Option<String> {
    let out = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(format!("tell application id \"{id}\"\n{script}\nend tell"))
        .output()
        .ok()?;
    if !out.status.success() {
        tracing::debug!(
            "couldn't ask {id} about playing: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Makes (but doesn't send) each key's events: the Objective-C calls'
    /// types are checked as they're made in debug builds.
    #[test]
    fn media_key_events_can_be_made() {
        for code in [NX_PLAY, NX_NEXT, NX_PREVIOUS] {
            for down in [true, false] {
                let mut made = false;
                with_event(code, down, |cg| made = !cg.0.is_null()).unwrap();
                assert!(made);
            }
        }
    }
}
