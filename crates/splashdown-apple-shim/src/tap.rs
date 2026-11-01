//! What the computer plays, for meeting notes: the Swift shim's Core Audio
//! tap on every app but Splashdown, or on one app's processes (macOS
//! 14.2+), fed into a `LiveAudio`.

use std::ffi::c_void;

use splashdown_audio::live::{LiveAudio, Pusher};

use crate::ffi;

/// The running tap. Stopping it (on drop) ends the callbacks before the
/// `Pusher` they use is freed.
struct Tap {
    handle: *mut c_void,
    pusher: *mut Pusher,
}

// SAFETY: the handle is only used to stop the tap, which the shim allows
// from any thread; the pusher is only touched by the tap's callbacks until
// then.
unsafe impl Send for Tap {}

impl Drop for Tap {
    fn drop(&mut self) {
        // SAFETY: `handle` came from `splashdown_tap_start` and is stopped
        // once; no callback runs after this returns, so the pusher can go.
        unsafe {
            ffi::splashdown_tap_stop(self.handle);
            drop(Box::from_raw(self.pusher));
        }
    }
}

extern "C" fn on_audio(ctx: *mut c_void, samples: *const f32, count: i32, rate: f64) {
    if samples.is_null() || count <= 0 {
        return;
    }
    // SAFETY: `ctx` is the `Box<Pusher>` owned by the `Tap`, alive until it
    // stops the tap; `samples` holds `count` floats for this call.
    let (pusher, samples) = unsafe {
        (
            &*ctx.cast::<Pusher>(),
            std::slice::from_raw_parts(samples, count as usize),
        )
    };
    pusher.push(samples, rate.round() as u32);
}

/// Starts recording what the computer plays. The first time, macOS asks
/// the user to allow it; until they do, it records silence.
pub fn system_audio() -> Result<LiveAudio, String> {
    app_audio(&[])
}

/// Starts recording what the Core Audio process objects in `processes` play
/// (an app and its helpers, from `splashdown_platform::audio_apps`); with
/// none, every app but Splashdown.
pub fn app_audio(processes: &[u32]) -> Result<LiveAudio, String> {
    let (live, pusher) = LiveAudio::pushed(Box::new(()));
    let pusher = Box::into_raw(Box::new(pusher));
    let (mut code, mut detail) = (0, 0);
    // SAFETY: `pusher` stays valid until the tap is stopped (`Tap::drop`),
    // and `on_audio` only reads it.
    let handle = unsafe {
        ffi::splashdown_tap_start(
            processes.as_ptr(),
            processes.len() as i32,
            pusher.cast(),
            on_audio,
            &mut code,
            &mut detail,
        )
    };
    if handle.is_null() {
        // SAFETY: the tap didn't start, so nothing else has the pusher.
        drop(unsafe { Box::from_raw(pusher) });
        return Err(match code {
            ffi::OS_TOO_OLD => "Recording the computer's sound needs macOS 14.2 or later.".into(),
            _ => format!("Couldn't record the computer's sound (Core Audio error {detail})."),
        });
    }
    Ok(live.keeping(Box::new(Tap { handle, pusher })))
}
