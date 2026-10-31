//! Quieting other sounds while recording: the default playback device's
//! volume or mute, through `IAudioEndpointVolume`, put back afterwards.

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::core::PCWSTR;

use crate::Quieten;

/// "Lower" turns other sounds down to this share of their volume.
const LOWER_TO: f32 = 0.25;

fn enumerator() -> Option<IMMDeviceEnumerator> {
    // SAFETY: no reserved pointer. Joins the MTA, or fails harmlessly on a
    // thread that's already single-threaded (COM still works there). Never
    // balanced, like the UI Automation client's (see `uia.rs`).
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    // SAFETY: MMDeviceEnumerator is the documented class implementing
    // IMMDeviceEnumerator; no aggregation.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()
}

fn volume_of(device: &IMMDevice) -> Option<IAudioEndpointVolume> {
    // SAFETY: plain values on a live device; no activation parameters.
    unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()
}

/// The default playback device's volume control, and the device's ID
/// (NUL-terminated) to find it again. Only the ID is kept between calls,
/// so the guard below stays `Send`.
fn endpoint() -> Option<(IAudioEndpointVolume, Vec<u16>)> {
    // SAFETY: plain values on a live enumerator.
    let device = unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
    // SAFETY: GetId returns a NUL-terminated string we own and free with
    // CoTaskMemFree after copying it.
    let id = unsafe {
        let raw = device.GetId().ok()?;
        let id = raw.as_wide().iter().copied().chain([0]).collect();
        CoTaskMemFree(Some(raw.0 as *const _));
        id
    };
    Some((volume_of(&device)?, id))
}

/// The device the volume was changed on, even if it's no longer the
/// default (headphones plugged in meanwhile).
fn endpoint_by_id(id: &[u16]) -> Option<IAudioEndpointVolume> {
    // SAFETY: `id` is NUL-terminated and outlives the call.
    let device = unsafe { enumerator()?.GetDevice(PCWSTR(id.as_ptr())) }.ok()?;
    volume_of(&device)
}

pub(crate) struct QuietOutput {
    /// The device it was done on.
    device: Vec<u16>,
    /// The volume before, and the one we set.
    volume: Option<(f32, f32)>,
    muted: bool,
}

pub(crate) fn quiet_output(how: Quieten) -> Option<QuietOutput> {
    let (volume, device) = endpoint()?;
    match how {
        Quieten::Mute => {
            // SAFETY: plain calls on a live endpoint; no event context.
            let muted = unsafe {
                !volume.GetMute().ok()?.as_bool() && volume.SetMute(true, std::ptr::null()).is_ok()
            };
            muted.then_some(QuietOutput {
                device,
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
                    device,
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
        let Some(volume) = endpoint_by_id(&self.device) else {
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
