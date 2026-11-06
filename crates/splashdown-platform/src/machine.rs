//! What the computer has to spare: memory, to choose a local model that
//! fits, and disk space, to say before a download that it won't. And the
//! language it's set to.

use std::path::Path;

/// The computer's memory in bytes (`None` where it isn't known).
pub fn memory_bytes() -> Option<u64> {
    imp::memory_bytes()
}

/// Free space on the disk holding `path` (which must exist), in bytes.
pub fn free_disk_bytes(path: &Path) -> Option<u64> {
    imp::free_disk_bytes(path)
}

/// The language the user set the computer to, as a two-letter code ("de"),
/// the first of their preferred languages.
pub fn system_language() -> Option<String> {
    let tag = imp::preferred_language()?;
    let code = tag.split(['-', '_']).next()?.to_ascii_lowercase();
    (code.len() == 2 || code.len() == 3).then_some(code)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub fn memory_bytes() -> Option<u64> {
        Some(objc2_foundation::NSProcessInfo::processInfo().physicalMemory())
    }

    pub fn preferred_language() -> Option<String> {
        let languages = objc2_foundation::NSLocale::preferredLanguages();
        languages.firstObject().map(|l| l.to_string())
    }

    pub fn free_disk_bytes(path: &Path) -> Option<u64> {
        let path = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: `path` is NUL-terminated and `stats` is written in full
        // when the call succeeds, which is checked before it's read.
        let stats = unsafe {
            if libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) != 0 {
                return None;
            }
            stats.assume_init()
        };
        Some(u64::from(stats.f_bavail) * stats.f_frsize)
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::core::PCWSTR;

    pub fn preferred_language() -> Option<String> {
        let mut name = [0u16; 85];
        // SAFETY: the buffer is LOCALE_NAME_MAX_LENGTH (85) wide characters,
        // and the call writes at most that, NUL included.
        let len = unsafe { windows::Win32::Globalization::GetUserDefaultLocaleName(&mut name) };
        (len > 1).then(|| String::from_utf16_lossy(&name[..len as usize - 1]))
    }

    pub fn memory_bytes() -> Option<u64> {
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: `status` is a MEMORYSTATUSEX with its length set, as the
        // call requires.
        unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
        Some(status.ullTotalPhys)
    }

    pub fn free_disk_bytes(path: &Path) -> Option<u64> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        // SAFETY: `wide` is NUL-terminated UTF-16 that outlives the call,
        // and `free` is a valid place for the answer.
        unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }.ok()?;
        Some(free)
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use std::path::Path;

    pub fn memory_bytes() -> Option<u64> {
        None
    }

    pub fn free_disk_bytes(_path: &Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(target_os = "macos", windows))]
    fn knows_memory_and_disk() {
        let memory = super::memory_bytes().unwrap();
        assert!(memory > 1 << 30, "{memory}");
        let free = super::free_disk_bytes(&std::env::temp_dir()).unwrap();
        assert!(free > 0);
    }
}
