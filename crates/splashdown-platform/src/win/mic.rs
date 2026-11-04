//! The microphone: whether Splashdown may use it, and which apps are using
//! it now (Windows keeps both in the registry). The Windows twin of
//! `mac/mic.rs` (the apps playing sound are in `app_audio.rs`).

use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use super::registry::*;
use crate::Access;

/// Reading the microphone privacy status properly needs WinRT
/// (`AppCapability`), so this reads the settings the privacy page writes:
/// the device-wide switch, "Microphone access" for apps, and "Let desktop
/// apps access your microphone". Anything other than an explicit "Deny"
/// counts as granted; desktop apps are never asked.
pub(super) fn microphone_access() -> Access {
    const STORE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";
    let denied = consent_denied(HKEY_LOCAL_MACHINE, STORE)
        || consent_denied(HKEY_CURRENT_USER, STORE)
        || consent_denied(HKEY_CURRENT_USER, &format!("{STORE}\\NonPackaged"));
    if denied {
        Access::Denied
    } else {
        Access::Granted
    }
}

pub(super) fn consent_denied(root: HKEY, subkey: &str) -> bool {
    reg_string(root, subkey, Some("Value")).is_some_and(|v| v.eq_ignore_ascii_case("Deny"))
}

/// Apps using a microphone now. Windows' privacy records keep, per app,
/// when it last started and stopped using one; a stop time of 0 means it
/// still is. Packaged apps are listed by package family name
/// (`MSTeams_8wekyb3d8bbwe`), others by their executable's file name
/// (`Zoom.exe`).
pub(crate) fn microphone_users() -> Vec<String> {
    const STORE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let mut users = Vec::new();
    for name in reg_subkeys(HKEY_CURRENT_USER, STORE) {
        if name.eq_ignore_ascii_case("NonPackaged") {
            let apps = format!("{STORE}\\NonPackaged");
            for app in reg_subkeys(HKEY_CURRENT_USER, &apps) {
                // The path, with # for each backslash.
                let file = app.rsplit('#').next().unwrap_or(&app).to_string();
                if !file.eq_ignore_ascii_case(&me) && in_use(&format!("{apps}\\{app}")) {
                    users.push(file);
                }
            }
        } else if in_use(&format!("{STORE}\\{name}")) {
            users.push(name);
        }
    }
    users
}

/// Whether the app with this consent record is using the microphone now.
pub(super) fn in_use(subkey: &str) -> bool {
    reg_qword(HKEY_CURRENT_USER, subkey, "LastUsedTimeStop") == Some(0)
        && reg_qword(HKEY_CURRENT_USER, subkey, "LastUsedTimeStart").is_some_and(|t| t > 0)
}
