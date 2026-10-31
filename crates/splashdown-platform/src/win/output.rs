//! Quieting other sounds while recording: the default playback device's
//! volume or mute, through `IAudioEndpointVolume`, put back afterwards.

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};

use crate::Quieten;

/// "Lower" turns other sounds down to this share of their volume.
const LOWER_TO: f32 = 0.25;

/// The default playback device's volume control. Nothing is kept between
/// calls, so the guard below stays `Send`.
fn endpoint() -> Option<IAudioEndpointVolume> {
    // SAFETY: no reserved pointer. Joins the MTA, or fails harmlessly on a
    // thread that's already single-threaded (COM still works there). Never
    // balanced, like the UI Automation client's (see `uia.rs`).
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    // SAFETY: MMDeviceEnumerator is the documented class implementing
    // IMMDeviceEnumerator; no aggregation.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
    // SAFETY: plain values on a live enumerator.
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
    // SAFETY: plain values on a live device; no activation parameters.
    unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()
}

pub(crate) struct QuietOutput {
    /// The volume before, and the one we set.
    volume: Option<(f32, f32)>,
    muted: bool,
}

pub(crate) fn quiet_output(how: Quieten) -> Option<QuietOutput> {
    let volume = endpoint()?;
    match how {
        Quieten::Mute => {
            // SAFETY: plain calls on a live endpoint; no event context.
            let muted = unsafe {
                !volume.GetMute().ok()?.as_bool() && volume.SetMute(true, std::ptr::null()).is_ok()
            };
            muted.then_some(QuietOutput {
                volume: None,
                muted: true,
            })
        }
        Quieten::Lower => {
            // SAFETY: plain calls on a live endpoint; no event context.
            unsafe {
                let was = volume.GetMasterVolumeLevelScalar().ok()?;
                if was <= 0.01 {
                    return None;
                }
                volume
                    .SetMasterVolumeLevelScalar(was * LOWER_TO, std::ptr::null())
                    .ok()?;
                let set = volume
                    .GetMasterVolumeLevelScalar()
                    .unwrap_or(was * LOWER_TO);
                Some(QuietOutput {
                    volume: Some((was, set)),
                    muted: false,
                })
            }
        }
    }
}

impl Drop for QuietOutput {
    /// Puts back what was changed, unless the user changed it meanwhile.
    fn drop(&mut self) {
        let Some(volume) = endpoint() else {
            return;
        };
        // SAFETY: plain calls on a live endpoint; no event context.
        unsafe {
            if let Some((was, set)) = self.volume
                && volume
                    .GetMasterVolumeLevelScalar()
                    .is_ok_and(|v| (v - set).abs() < 0.01)
            {
                let _ = volume.SetMasterVolumeLevelScalar(was, std::ptr::null());
            }
            if self.muted && volume.GetMute().is_ok_and(|m| m.as_bool()) {
                let _ = volume.SetMute(false, std::ptr::null());
            }
        }
    }
}
