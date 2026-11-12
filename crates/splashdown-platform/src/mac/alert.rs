//! A plain native alert for questions the app must ask before any of its
//! windows exist (moving itself to the Applications folder), and telling
//! whether a copy of an app is open from a given place.

use std::path::Path;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSApplication, NSWorkspace};
use objc2_foundation::NSString;

/// Shows `title` and `text` with `buttons` (the first is the default, the
/// second also answers Escape) and waits for one: its index. `None` if it
/// couldn't be shown, which only happens off the main thread.
pub fn alert(title: &str, text: &str, buttons: &[&str]) -> Option<usize> {
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    // A menu bar app isn't in front when it starts: the alert would open
    // behind whatever window is. The replacement call needs macOS 14.
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(text));
    for (i, label) in buttons.iter().enumerate() {
        let button = alert.addButtonWithTitle(&NSString::from_str(label));
        if i == 1 {
            button.setKeyEquivalent(&NSString::from_str("\u{1b}"));
        }
    }
    let answer = alert.runModal() - NSAlertFirstButtonReturn;
    usize::try_from(answer).ok().filter(|&i| i < buttons.len())
}

/// Makes this app the active one, so a window it shows next comes to the
/// front instead of opening behind the app that was. Main thread only (does
/// nothing elsewhere). A menu bar app is never active until it asks.
pub fn activate_self() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // The replacement call is cooperative on macOS 14 and may be refused
    // while another app is in front; this one still works.
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}

/// True if an app other than this one is running from the bundle at `path`.
pub fn app_running_at(path: &Path) -> bool {
    let own = std::process::id();
    let wanted = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|app| u32::try_from(app.processIdentifier()).ok() != Some(own))
        .filter_map(|app| app.bundleURL()?.path())
        .any(|p| {
            let p = Path::new(&p.to_string()).to_path_buf();
            std::fs::canonicalize(&p).unwrap_or(p) == wanted
        })
}
