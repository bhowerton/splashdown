//! Reading the Windows registry: one value, or a key's subkeys.

use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, KEY_READ, RRF_RT_REG_QWORD, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
    RegOpenKeyExW,
};
use windows::core::{PCWSTR, PWSTR};

use super::{from_wide, wide};

/// A number (QWORD) value from the registry.
pub(super) fn reg_qword(root: HKEY, subkey: &str, value: &str) -> Option<u64> {
    let subkey = wide(subkey);
    let value = wide(value);
    let mut data = 0u64;
    let mut size = std::mem::size_of::<u64>() as u32;
    // SAFETY: NUL-terminated strings that outlive the call; `data` is
    // writable for the `size` bytes passed.
    let err = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_QWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut size),
        )
    };
    (err == ERROR_SUCCESS).then_some(data)
}

/// The names of a registry key's subkeys (none if it can't be opened).
pub(super) fn reg_subkeys(root: HKEY, subkey: &str) -> Vec<String> {
    let subkey = wide(subkey);
    let mut key = HKEY::default();
    // SAFETY: a NUL-terminated name that outlives the call and a writable
    // key handle, closed below.
    let opened =
        unsafe { RegOpenKeyExW(root, PCWSTR(subkey.as_ptr()), Some(0), KEY_READ, &mut key) };
    if opened != ERROR_SUCCESS {
        return Vec::new();
    }
    let mut names = Vec::new();
    let mut buf = [0u16; 512];
    for index in 0.. {
        let mut len = buf.len() as u32;
        // SAFETY: `buf` is writable for `len` characters; the rest is unused.
        let err = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if err == ERROR_NO_MORE_ITEMS {
            break;
        }
        if err == ERROR_SUCCESS {
            names.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
    }
    // SAFETY: opened above.
    let _ = unsafe { RegCloseKey(key) };
    names
}

/// A string value from the registry (`None` for the key's default value).
/// Expandable strings come back expanded.
pub(super) fn reg_string(root: HKEY, subkey: &str, value: Option<&str>) -> Option<String> {
    let subkey = wide(subkey);
    let value = value.map(wide);
    let mut buf = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: `subkey` and `value` are NUL-terminated strings that outlive
    // the call (a null value name means the default value); `buf` is
    // writable for the `size` bytes passed, and `size` is a valid in/out
    // pointer.
    let err = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            value
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (err == ERROR_SUCCESS).then(|| from_wide(&buf))
}
