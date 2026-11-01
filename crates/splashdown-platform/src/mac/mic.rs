//! Which other apps are using a microphone right now, and which play sound,
//! from Core Audio's process objects (macOS 14 or later; earlier versions
//! report none).

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};

use crate::AudioApp;

use super::output::{Address, SCOPE_GLOBAL, SYSTEM_OBJECT, code, get, get_list};

/// `kAudioHardwarePropertyProcessObjectList`.
const PROCESS_LIST: u32 = code(b"prs#");
/// `kAudioProcessPropertyPID`.
const PID: u32 = code(b"ppid");
/// `kAudioProcessPropertyBundleID`.
const BUNDLE_ID: u32 = code(b"pbid");
/// `kAudioProcessPropertyIsRunningInput`.
const RUNNING_INPUT: u32 = code(b"piri");
/// `kAudioProcessPropertyIsRunningOutput`.
const RUNNING_OUTPUT: u32 = code(b"piro");

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
        .filter_map(bundle_id)
        .collect()
}

/// A process object's bundle ID.
fn bundle_id(process: u32) -> Option<String> {
    let id = get::<usize>(process, &global(BUNDLE_ID)).filter(|&p| p != 0)?;
    // SAFETY: Core Audio hands over a retained CFString (the caller
    // releases it), which the wrapper now owns.
    let id = unsafe { CFString::wrap_under_create_rule(id as CFStringRef) };
    Some(id.to_string()).filter(|s| !s.is_empty())
}

/// The apps Core Audio knows (they've played or recorded sound since they
/// started), each with its process objects: a browser's audio plays from a
/// helper process, which counts as the browser's. Playing ones first.
pub(crate) fn audio_apps() -> Vec<AudioApp> {
    let me = std::process::id() as i32;
    // Apps with a Dock icon: (bundle ID, name).
    let running: Vec<(String, String)> = NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular)
        .filter_map(|app| {
            let id = app.bundleIdentifier()?.to_string();
            let name = app
                .localizedName()
                .map_or_else(|| id.clone(), |n| n.to_string());
            Some((id, name))
        })
        .collect();
    let mut apps: Vec<AudioApp> = Vec::new();
    for process in get_list(SYSTEM_OBJECT, &global(PROCESS_LIST)) {
        let Some(pid) = get::<i32>(process, &global(PID)).filter(|&p| p != me) else {
            continue;
        };
        let Some(bundle) = bundle_id(process) else {
            continue;
        };
        let Some((id, name)) = owner(pid, &bundle, &running) else {
            continue;
        };
        let playing = get::<u32>(process, &global(RUNNING_OUTPUT)) == Some(1);
        match apps.iter_mut().find(|a| a.id == id) {
            Some(app) => {
                app.playing |= playing;
                app.sources.push(process);
            }
            None => apps.push(AudioApp {
                id,
                name,
                playing,
                sources: vec![process],
            }),
        }
    }
    apps.sort_by(|a, b| {
        b.playing
            .cmp(&a.playing)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    apps
}

/// The app a process belongs to: itself if it has a Dock icon, or the app
/// whose helper it is ("com.google.Chrome.helper" is Chrome's; WebKit's
/// shared helpers play Safari's sound). None for system services.
fn owner(pid: i32, bundle: &str, running: &[(String, String)]) -> Option<(String, String)> {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        && app.activationPolicy() == NSApplicationActivationPolicy::Regular
        && let Some(id) = app.bundleIdentifier()
    {
        let id = id.to_string();
        let name = app
            .localizedName()
            .map_or_else(|| id.clone(), |n| n.to_string());
        return Some((id, name));
    }
    let parent = if bundle.starts_with("com.apple.WebKit.") {
        "com.apple.Safari"
    } else {
        bundle
    };
    running
        .iter()
        .find(|(id, _)| parent == id || parent.starts_with(&format!("{id}.")))
        .cloned()
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

    /// Prints the apps that play sound: `cargo test -p splashdown-platform
    /// -- --ignored --nocapture audio_apps`.
    #[test]
    #[ignore = "depends on what's running"]
    fn lists_audio_apps() {
        for app in super::audio_apps() {
            println!("{app:?}");
        }
    }
}
