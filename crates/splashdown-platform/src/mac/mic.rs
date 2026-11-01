//! Which other apps are using a microphone right now, from Core Audio's
//! process objects (macOS 14 or later; earlier versions report none).

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

use super::output::{Address, SCOPE_GLOBAL, SYSTEM_OBJECT, code, get, get_list};

/// `kAudioHardwarePropertyProcessObjectList`.
const PROCESS_LIST: u32 = code(b"prs#");
/// `kAudioProcessPropertyPID`.
const PID: u32 = code(b"ppid");
/// `kAudioProcessPropertyBundleID`.
const BUNDLE_ID: u32 = code(b"pbid");
/// `kAudioProcessPropertyIsRunningInput`.
const RUNNING_INPUT: u32 = code(b"piri");

fn global(selector: u32) -> Address {
    Address {
        selector,
        scope: SCOPE_GLOBAL,
        element: 0,
    }
}

/// Bundle IDs of the other processes recording from an input device now
/// (a browser's or an app's helper process has its own, such as
/// `com.google.Chrome.helper`).
pub(crate) fn microphone_users() -> Vec<String> {
    let me = std::process::id() as i32;
    get_list(SYSTEM_OBJECT, &global(PROCESS_LIST))
        .into_iter()
        .filter(|&process| get::<u32>(process, &global(RUNNING_INPUT)) == Some(1))
        .filter(|&process| get::<i32>(process, &global(PID)) != Some(me))
        .filter_map(|process| {
            let id = get::<usize>(process, &global(BUNDLE_ID)).filter(|&p| p != 0)?;
            // SAFETY: Core Audio hands over a retained CFString (the
            // caller releases it), which the wrapper now owns.
            let id = unsafe { CFString::wrap_under_create_rule(id as CFStringRef) };
            Some(id.to_string()).filter(|s| !s.is_empty())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// Prints the apps using a microphone; start a recording elsewhere
    /// first: `cargo test -p splashdown-platform -- --ignored --nocapture mic`.
    #[test]
    #[ignore = "depends on what's running"]
    fn lists_microphone_users() {
        println!("{:?}", super::microphone_users());
    }
}
