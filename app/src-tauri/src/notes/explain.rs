//! On a Mac, the first time the default notes folder is made in Documents,
//! macOS asks whether Splashdown may use the Documents folder. This says
//! why first, in a plain alert, once.
//!
//! Threads: the alert only runs on the main thread. A caller there shows it
//! itself; any other caller asks the main thread to and waits until it's
//! dismissed. Nothing here holds a lock while the alert is up or while
//! waiting for it, and the main thread never waits for another thread.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use splashdown_core::sync::Locked;
use tauri::AppHandle;

use super::folder::{default_folder, fallback_folder};

#[cfg(target_os = "macos")]
const TITLE: &str = "Your notes are kept in Documents";
#[cfg(target_os = "macos")]
const TEXT: &str = "Splashdown keeps your notes as files in Documents › Splashdown Notes, where you can open them yourself. macOS will now ask to let Splashdown use your Documents folder. You can keep them somewhere else in Settings → Notes.";
#[cfg(target_os = "macos")]
const BUTTON: &str = "Continue";

/// Left in the settings folder once the alert has been shown, so it's
/// never shown again, even if the notes folder is deleted later.
const SHOWN_FILE: &str = "notes-folder-explained";

/// How long a caller off the main thread waits for the main thread to
/// start showing the alert. Normally a moment; longer means the main
/// thread is stuck, and going on without the alert beats hanging.
const MAIN_THREAD_WAIT: Duration = Duration::from_secs(20);

/// What decides whether to explain.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Situation {
    /// On a Mac.
    pub mac: bool,
    /// The folder is the default one in Documents, not one picked in
    /// Settings (picking a folder there already lets Splashdown use it).
    pub default: bool,
    /// The folder is already there.
    pub exists: bool,
    /// Splashdown's own folder is there: Documents was refused before.
    pub fallback_exists: bool,
    /// The alert was shown before.
    pub explained: bool,
}

/// Whether making the folder now needs the alert first.
pub(super) fn should_explain(s: Situation) -> bool {
    s.mac && s.default && !s.exists && !s.fallback_exists && !s.explained
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Not shown (or not known to be needed) yet.
    Idle,
    /// A background caller asked the main thread to show it.
    Asked,
    /// Up on the main thread.
    Showing,
    /// Shown, or not needed for the rest of the run.
    Done,
}

struct Gate {
    phase: Mutex<Phase>,
    changed: Condvar,
}

fn gate() -> &'static Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    GATE.get_or_init(|| Gate {
        phase: Mutex::new(Phase::Idle),
        changed: Condvar::new(),
    })
}

fn app() -> &'static OnceLock<AppHandle> {
    static APP: OnceLock<AppHandle> = OnceLock::new();
    &APP
}

/// Lets a background thread ask the main thread for the alert. Call once
/// at startup.
pub fn init(app_handle: &AppHandle) {
    let _ = app().set(app_handle.clone());
}

fn on_main_thread() -> bool {
    std::thread::current().name() == Some("main")
}

fn shown_file() -> Option<PathBuf> {
    crate::data::path(SHOWN_FILE).ok()
}

/// Called just before `wanted` would be made, with no lock held. Returns
/// once it's fine to go on: nothing to explain, or the alert was
/// dismissed.
pub(super) fn before_making(wanted: &Path) {
    if !cfg!(target_os = "macos") || default_folder().as_deref() != Some(wanted) {
        return;
    }
    let gate = gate();
    if *gate.phase.locked() == Phase::Idle {
        let situation = Situation {
            mac: true,
            default: true,
            // Metadata only: looking doesn't bring up the prompt.
            exists: wanted.is_dir(),
            fallback_exists: fallback_folder().is_ok_and(|f| f.is_dir()),
            explained: shown_file().is_some_and(|f| f.exists()),
        };
        if !should_explain(situation) {
            // Each of these stays true for the rest of the run.
            settle(Phase::Idle);
            return;
        }
    }
    let mut phase = gate.phase.locked();
    let mut asked_at: Option<Instant> = None;
    loop {
        match *phase {
            Phase::Done => return,
            Phase::Showing if on_main_thread() => {
                // Called again from inside the alert's own event loop
                // (another command came in while it's up). Waiting here
                // would stop the alert from ever closing; it's on screen
                // already, so go on.
                return;
            }
            Phase::Idle | Phase::Asked if on_main_thread() => {
                // The main thread never waits on another thread: it shows
                // the alert itself, and a request still queued finds it
                // done.
                *phase = Phase::Showing;
                drop(phase);
                show();
                return;
            }
            Phase::Idle => {
                *phase = Phase::Asked;
                drop(phase);
                let asked = app()
                    .get()
                    .is_some_and(|app| app.run_on_main_thread(show_if_asked).is_ok());
                if !asked {
                    tracing::warn!("couldn't explain the notes folder: no main thread to ask");
                    settle(Phase::Asked);
                    return;
                }
                asked_at = Some(Instant::now());
                phase = gate.phase.locked();
            }
            Phase::Asked => {
                // Wait for the main thread to take it, but not forever.
                let started = *asked_at.get_or_insert_with(Instant::now);
                let left = MAIN_THREAD_WAIT.saturating_sub(started.elapsed());
                if left.is_zero() {
                    drop(phase);
                    tracing::warn!("the main thread didn't show the notes folder alert; going on");
                    settle(Phase::Asked);
                    return;
                }
                phase = gate
                    .changed
                    .wait_timeout(phase, left)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            Phase::Showing => {
                // Up on the main thread: wait for the user, however long.
                phase = gate.changed.wait(phase).unwrap_or_else(|e| e.into_inner());
            }
        }
    }
}

/// On the main thread, for a background caller: shows the alert unless
/// it's up or done already.
fn show_if_asked() {
    {
        let mut phase = gate().phase.locked();
        if *phase != Phase::Asked {
            return;
        }
        *phase = Phase::Showing;
    }
    show();
}

/// Done, if the phase is still `from` (it may have moved on meanwhile),
/// and everyone waiting goes on.
fn settle(from: Phase) {
    let gate = gate();
    let mut phase = gate.phase.locked();
    if *phase == from {
        *phase = Phase::Done;
        gate.changed.notify_all();
    }
}

/// Shows the alert and waits for it, then lets everyone waiting go on.
/// Main thread only, with the phase at `Showing`.
fn show() {
    // Kept first, so it's at most once even if the app quits meanwhile.
    if let Some(file) = shown_file()
        && let Err(e) = std::fs::write(&file, b"")
    {
        tracing::warn!("couldn't keep a note that the notes folder was explained: {e}");
    }
    #[cfg(target_os = "macos")]
    if splashdown_platform::alert(TITLE, TEXT, &[BUTTON]).is_none() {
        tracing::warn!("couldn't show the notes folder alert");
    }
    settle(Phase::Showing);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_mac_user() -> Situation {
        Situation {
            mac: true,
            default: true,
            ..Situation::default()
        }
    }

    #[test]
    fn explains_before_making_the_default_folder_on_a_mac() {
        assert!(should_explain(new_mac_user()));
    }

    #[test]
    fn never_on_windows_or_linux() {
        assert!(!should_explain(Situation {
            mac: false,
            ..new_mac_user()
        }));
    }

    #[test]
    fn not_for_a_folder_picked_in_settings() {
        assert!(!should_explain(Situation {
            default: false,
            ..new_mac_user()
        }));
    }

    #[test]
    fn not_when_the_folder_is_there_already() {
        assert!(!should_explain(Situation {
            exists: true,
            ..new_mac_user()
        }));
    }

    #[test]
    fn not_once_documents_was_refused() {
        assert!(!should_explain(Situation {
            fallback_exists: true,
            ..new_mac_user()
        }));
    }

    #[test]
    fn only_once() {
        assert!(!should_explain(Situation {
            explained: true,
            ..new_mac_user()
        }));
    }
}
