//! Choosing which microphone to open.
//!
//! "Automatic" uses the system default input unless it's a Bluetooth headset
//! (AirPods etc.). Opening a headset's mic switches it into low-quality
//! call mode, which also degrades whatever the user is listening to, so
//! the built-in mic is used instead when there is one.

/// What the platform knows about an input device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputInfo {
    pub name: String,
    pub bluetooth: bool,
    pub built_in: bool,
    pub is_default: bool,
}

/// The device to open: the first of `preferred` that's connected,
/// otherwise the automatic choice. `None` means "the system default".
pub fn choose(preferred: &[String], inputs: &[InputInfo]) -> Option<String> {
    if let Some(name) = preferred
        .iter()
        .find(|name| inputs.iter().any(|i| &i.name == *name))
    {
        return Some(name.clone());
    }
    let default = inputs.iter().find(|i| i.is_default)?;
    if !default.bluetooth {
        return None;
    }
    inputs.iter().find(|i| i.built_in).map(|i| i.name.clone())
}

/// Input devices with their transport, for choosing and for Settings.
pub fn inputs() -> Vec<InputInfo> {
    imp::inputs()
}

/// The name of the device the computer plays through now (to notice it
/// changing, e.g. headphones plugged in during a meeting).
pub fn default_output() -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}

#[cfg(target_os = "macos")]
mod imp {
    //! Core Audio: device list, names, transport type and default input.

    use std::ffi::c_void;
    use std::ptr;

    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

    use super::InputInfo;

    type AudioObjectId = u32;

    #[repr(C)]
    struct PropertyAddress {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const fn fourcc(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }

    const SYSTEM_OBJECT: AudioObjectId = 1;
    const DEVICES: u32 = fourcc(b"dev#");
    const DEFAULT_INPUT: u32 = fourcc(b"dIn ");
    const NAME: u32 = fourcc(b"lnam");
    const TRANSPORT: u32 = fourcc(b"tran");
    const STREAMS: u32 = fourcc(b"stm#");
    const SCOPE_GLOBAL: u32 = fourcc(b"glob");
    const SCOPE_INPUT: u32 = fourcc(b"inpt");
    const ELEMENT_MAIN: u32 = 0;
    const TRANSPORT_BUILT_IN: u32 = fourcc(b"bltn");
    const TRANSPORT_BLUETOOTH: u32 = fourcc(b"blue");
    const TRANSPORT_BLUETOOTH_LE: u32 = fourcc(b"blea");

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(
            object: AudioObjectId,
            address: *const PropertyAddress,
            qualifier_size: u32,
            qualifier: *const c_void,
            data_size: *mut u32,
        ) -> i32;
        fn AudioObjectGetPropertyData(
            object: AudioObjectId,
            address: *const PropertyAddress,
            qualifier_size: u32,
            qualifier: *const c_void,
            data_size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    fn address(selector: u32, scope: u32) -> PropertyAddress {
        PropertyAddress {
            selector,
            scope,
            element: ELEMENT_MAIN,
        }
    }

    fn get_u32(object: AudioObjectId, selector: u32, scope: u32) -> Option<u32> {
        let addr = address(selector, scope);
        let mut value = 0u32;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: `addr` and `size` are valid for the call, and `size` says
        // `value` has room for exactly one u32, so Core Audio can't write
        // past it.
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                &addr,
                0,
                ptr::null(),
                &mut size,
                (&mut value as *mut u32).cast(),
            )
        };
        (status == 0).then_some(value)
    }

    fn size_of_property(object: AudioObjectId, selector: u32, scope: u32) -> u32 {
        let addr = address(selector, scope);
        let mut size = 0u32;
        // SAFETY: `addr` is valid for the call and `size` a valid out-pointer;
        // no qualifier.
        let status =
            unsafe { AudioObjectGetPropertyDataSize(object, &addr, 0, ptr::null(), &mut size) };
        if status == 0 { size } else { 0 }
    }

    fn name(object: AudioObjectId) -> Option<String> {
        let addr = address(NAME, SCOPE_GLOBAL);
        let mut cf: CFStringRef = ptr::null();
        let mut size = size_of::<CFStringRef>() as u32;
        // SAFETY: `addr` and `size` are valid for the call, and `size` says
        // `cf` has room for exactly one CFStringRef.
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                &addr,
                0,
                ptr::null(),
                &mut size,
                (&mut cf as *mut CFStringRef).cast(),
            )
        };
        if status != 0 || cf.is_null() {
            return None;
        }
        // SAFETY: Core Audio returns this name retained and the caller must
        // release it (Create rule); `cf` is non-null.
        Some(unsafe { CFString::wrap_under_create_rule(cf) }.to_string())
    }

    pub(super) fn inputs() -> Vec<InputInfo> {
        let bytes = size_of_property(SYSTEM_OBJECT, DEVICES, SCOPE_GLOBAL);
        let count = bytes as usize / size_of::<AudioObjectId>();
        let mut ids = vec![0 as AudioObjectId; count];
        let addr = address(DEVICES, SCOPE_GLOBAL);
        // The buffer's real size, which `bytes` could exceed if it weren't a
        // multiple of the element size.
        let mut size = (count * size_of::<AudioObjectId>()) as u32;
        // SAFETY: `addr` and `size` are valid for the call, and `size` is the
        // byte length of `ids`, so Core Audio writes within it (fewer bytes
        // if devices went away since the size query).
        let status = unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &addr,
                0,
                ptr::null(),
                &mut size,
                ids.as_mut_ptr().cast(),
            )
        };
        if status != 0 {
            return Vec::new();
        }
        // Drop slots that weren't filled because the list shrank.
        ids.truncate(size as usize / size_of::<AudioObjectId>());
        let default = get_u32(SYSTEM_OBJECT, DEFAULT_INPUT, SCOPE_GLOBAL);

        ids.into_iter()
            // Only devices with input streams are microphones.
            .filter(|&id| size_of_property(id, STREAMS, SCOPE_INPUT) > 0)
            .filter_map(|id| {
                let transport = get_u32(id, TRANSPORT, SCOPE_GLOBAL).unwrap_or(0);
                Some(InputInfo {
                    name: name(id)?,
                    bluetooth: matches!(transport, TRANSPORT_BLUETOOTH | TRANSPORT_BLUETOOTH_LE),
                    built_in: transport == TRANSPORT_BUILT_IN,
                    is_default: Some(id) == default,
                })
            })
            .collect()
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    //! Elsewhere (until M3): names only, via cpal; no transport info, so
    //! "Automatic" is simply the system default.

    use cpal::traits::{DeviceTrait, HostTrait};

    use super::InputInfo;

    pub(super) fn inputs() -> Vec<InputInfo> {
        let host = cpal::default_host();
        let default = host.default_input_device().and_then(|d| d.name().ok());
        host.input_devices()
            .map(|devices| {
                devices
                    .filter_map(|d| d.name().ok())
                    .map(|name| InputInfo {
                        is_default: Some(&name) == default.as_ref(),
                        name,
                        bluetooth: false,
                        built_in: false,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, bluetooth: bool, built_in: bool, is_default: bool) -> InputInfo {
        InputInfo {
            name: name.into(),
            bluetooth,
            built_in,
            is_default,
        }
    }

    #[test]
    fn avoids_bluetooth_default() {
        let inputs = [
            input("AirPods Pro", true, false, true),
            input("MacBook Pro Microphone", false, true, false),
        ];
        assert_eq!(
            choose(&[], &inputs).as_deref(),
            Some("MacBook Pro Microphone")
        );
    }

    #[test]
    fn keeps_a_wired_default_and_honours_a_choice() {
        let inputs = [
            input("Yeti", false, false, true),
            input("MacBook Pro Microphone", false, true, false),
            input("AirPods Pro", true, false, false),
        ];
        assert_eq!(choose(&[], &inputs), None, "system default (the USB mic)");
        assert_eq!(
            choose(&["AirPods Pro".into()], &inputs).as_deref(),
            Some("AirPods Pro")
        );
        assert_eq!(
            choose(&["Gone".into()], &inputs),
            None,
            "unplugged choice falls back"
        );
    }

    #[test]
    fn the_first_connected_choice_wins() {
        let inputs = [
            input("MacBook Pro Microphone", false, true, true),
            input("AirPods Pro", true, false, false),
        ];
        let wanted = ["Yeti".to_string(), "AirPods Pro".to_string()];
        assert_eq!(choose(&wanted, &inputs).as_deref(), Some("AirPods Pro"));
        let gone = ["Yeti".to_string(), "Rode".to_string()];
        assert_eq!(choose(&gone, &inputs), None, "automatic");
    }

    #[test]
    fn bluetooth_only_uses_it() {
        let inputs = [input("AirPods Pro", true, false, true)];
        assert_eq!(choose(&[], &inputs), None);
    }
}
