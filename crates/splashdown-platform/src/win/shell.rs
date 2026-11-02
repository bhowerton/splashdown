//! Opening things the way a double-click does (web pages, settings pages,
//! files, Start menu shortcuts, Store apps): `ShellExecuteW`, with wide
//! strings so any name or path works, and no console flashing up.

use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow, SW_SHOWNORMAL};
use windows::core::{HSTRING, w};

use crate::{PlatformError, Result};

/// Opens `target`: a URL, a `ms-settings:` page, a file or shortcut path,
/// or `shell:AppsFolder\<AppUserModelID>`.
pub(crate) fn shell_open(target: &str) -> Result<()> {
    // Some handlers are COM objects that want a single-threaded apartment
    // (Microsoft's advice for ShellExecute); harmless if the thread already
    // has one of either kind.
    // SAFETY: no reserved pointer; never balanced, as elsewhere here.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    // Our process may be in the background (a spoken command): let what it
    // opens come to the front.
    // SAFETY: plain value.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let file = HSTRING::from(target);
    // SAFETY: valid NUL-terminated wide strings that outlive the call; no
    // owner window.
    let result = unsafe { ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL) };
    // Anything above 32 is success.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(PlatformError::Failed(format!(
            "Windows couldn't open it (error {})",
            result.0 as usize
        )))
    }
}
