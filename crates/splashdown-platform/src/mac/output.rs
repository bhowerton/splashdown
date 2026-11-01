//! Quieting other sounds while recording: the default output device's
//! volume or mute, through CoreAudio's property API, put back afterwards.

use std::ffi::c_void;

use crate::Quieten;

pub(super) const fn code(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// `kAudioObjectSystemObject`.
pub(super) const SYSTEM_OBJECT: u32 = 1;
const DEFAULT_OUTPUT: u32 = code(b"dOut");
pub(super) const SCOPE_GLOBAL: u32 = code(b"glob");
const SCOPE_OUTPUT: u32 = code(b"outp");
/// `kAudioDevicePropertyVolumeScalar` (0.0–1.0).
const VOLUME: u32 = code(b"volm");
/// `kAudioDevicePropertyMute` (0 or 1).
const MUTE: u32 = code(b"mute");
/// The main element; devices without one have per-channel controls.
const MAIN: u32 = 0;
const CHANNELS: [u32; 2] = [1, 2];
/// "Lower" turns other sounds down to this share of their volume.
const LOWER_TO: f32 = 0.25;

#[repr(C)]
pub(super) struct Address {
    pub(super) selector: u32,
    pub(super) scope: u32,
    pub(super) element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectHasProperty(id: u32, address: *const Address) -> u8;
    fn AudioObjectIsPropertySettable(id: u32, address: *const Address, settable: *mut u8) -> i32;
    fn AudioObjectGetPropertyDataSize(
        id: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> i32;
    fn AudioObjectGetPropertyData(
        id: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn AudioObjectSetPropertyData(
        id: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> i32;
}

/// A property value of a fixed size: `u32` (device IDs, mute), `f32`
/// (volume), a pointer.
pub(super) fn get<T: Copy + Default>(id: u32, address: &Address) -> Option<T> {
    let mut value = T::default();
    let mut size = size_of::<T>() as u32;
    // SAFETY: `value` is writable and `size` is its size; no qualifier.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            address,
            0,
            std::ptr::null(),
            &mut size,
            (&raw mut value).cast(),
        )
    };
    (status == 0 && size as usize == size_of::<T>()).then_some(value)
}

/// A property that's a list of object IDs (empty if there's no such
/// property, as on an older macOS).
pub(super) fn get_list(id: u32, address: &Address) -> Vec<u32> {
    let mut size = 0u32;
    // SAFETY: plain values and a writable size; no qualifier.
    let status =
        unsafe { AudioObjectGetPropertyDataSize(id, address, 0, std::ptr::null(), &mut size) };
    if status != 0 || size == 0 {
        return Vec::new();
    }
    let mut ids = vec![0u32; size as usize / size_of::<u32>()];
    // SAFETY: `ids` is writable for `size` bytes; no qualifier.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            address,
            0,
            std::ptr::null(),
            &mut size,
            ids.as_mut_ptr().cast(),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    ids.truncate(size as usize / size_of::<u32>());
    ids
}

fn set<T: Copy>(id: u32, address: &Address, value: T) -> bool {
    // SAFETY: `value` is readable for its size; no qualifier.
    let status = unsafe {
        AudioObjectSetPropertyData(
            id,
            address,
            0,
            std::ptr::null(),
            size_of::<T>() as u32,
            (&raw const value).cast(),
        )
    };
    status == 0
}

fn settable(id: u32, address: &Address) -> bool {
    let mut settable = 0u8;
    // SAFETY: plain values and a writable flag.
    unsafe {
        AudioObjectHasProperty(id, address) != 0
            && AudioObjectIsPropertySettable(id, address, &mut settable) == 0
            && settable != 0
    }
}

fn output(selector: u32, element: u32) -> Address {
    Address {
        selector,
        scope: SCOPE_OUTPUT,
        element,
    }
}

/// The elements carrying `selector`: the main one, or else each channel.
fn elements(device: u32, selector: u32) -> Vec<u32> {
    if settable(device, &output(selector, MAIN)) {
        vec![MAIN]
    } else {
        CHANNELS
            .into_iter()
            .filter(|&e| settable(device, &output(selector, e)))
            .collect()
    }
}

enum Change {
    Volume { element: u32, was: f32, set: f32 },
    Mute { element: u32 },
}

pub(crate) struct QuietOutput {
    device: u32,
    changes: Vec<Change>,
}

pub(crate) fn quiet_output(how: Quieten) -> Option<QuietOutput> {
    let default = Address {
        selector: DEFAULT_OUTPUT,
        scope: SCOPE_GLOBAL,
        element: MAIN,
    };
    let device: u32 = get(SYSTEM_OBJECT, &default).filter(|&d| d != 0)?;
    let mut changes = Vec::new();
    match how {
        Quieten::Mute => {
            for element in elements(device, MUTE) {
                let address = output(MUTE, element);
                if get::<u32>(device, &address) == Some(0) && set(device, &address, 1u32) {
                    changes.push(Change::Mute { element });
                }
            }
        }
        Quieten::Lower => {
            for element in elements(device, VOLUME) {
                let address = output(VOLUME, element);
                let Some(was) = get::<f32>(device, &address).filter(|&v| v > 0.01) else {
                    continue;
                };
                let lower = was * LOWER_TO;
                if set(device, &address, lower) {
                    // What the device actually took, to recognize it later.
                    let set = get::<f32>(device, &address).unwrap_or(lower);
                    changes.push(Change::Volume { element, was, set });
                }
            }
        }
    }
    (!changes.is_empty()).then_some(QuietOutput { device, changes })
}

impl Drop for QuietOutput {
    /// Puts back what was changed, unless the user changed it meanwhile.
    fn drop(&mut self) {
        for change in &self.changes {
            match *change {
                Change::Volume { element, was, set } => {
                    let address = output(VOLUME, element);
                    if get::<f32>(self.device, &address).is_some_and(|v| (v - set).abs() < 0.01) {
                        self::set(self.device, &address, was);
                    }
                }
                Change::Mute { element } => {
                    let address = output(MUTE, element);
                    if get::<u32>(self.device, &address) == Some(1) {
                        set(self.device, &address, 0u32);
                    }
                }
            }
        }
    }
}
