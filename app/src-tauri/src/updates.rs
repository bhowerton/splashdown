//! New releases (tauri-plugin-updater). Checks GitHub on launch and once a
//! day when the user allows it, shows what it found in the menu bar and in
//! Settings → About, and installs only when the user clicks. A failed check
//! (offline, nothing published yet) is logged and otherwise ignored.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Error, Update, UpdaterExt};

use crate::tray::TrayMenu;

/// App event with the new `Status` whenever it changes.
pub const UPDATE_STATUS: &str = "update-status";

/// Long enough for launch work (models, permissions) to settle first.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the background thread wakes to see whether a check is due.
/// Wall-clock based, so a Mac that slept through the day still checks.
const TICK: Duration = Duration::from_secs(60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// What Settings → About shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    /// The newer version, when there is one.
    available: Option<String>,
    /// Its release notes, if any were published.
    notes: Option<String>,
    checking: bool,
    installing: bool,
    /// Download progress while installing, 0–100.
    progress: Option<u8>,
    /// Unix milliseconds of the last check that got an answer.
    last_checked: Option<u64>,
    /// Why the last check failed.
    error: Option<String>,
}

pub struct Updates {
    status: Mutex<Status>,
    pending: Mutex<Option<Update>>,
    /// One check or install at a time.
    busy: AtomicBool,
    /// When a check last got an answer. Failed checks (offline) retry at
    /// the next tick instead of waiting a day.
    last_success: Mutex<Option<SystemTime>>,
    tray: TrayMenu,
}

impl Updates {
    pub fn new(tray: TrayMenu) -> Self {
        Self {
            status: Mutex::default(),
            pending: Mutex::default(),
            busy: AtomicBool::new(false),
            last_success: Mutex::default(),
            tray,
        }
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn available(&self) -> Option<String> {
        self.status().available
    }

    fn update(&self, app: &AppHandle, change: impl FnOnce(&mut Status)) {
        let status = {
            let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
            change(&mut s);
            s.clone()
        };
        self.tray.set_update(status.available.as_deref());
        let _ = app.emit(UPDATE_STATUS, status);
    }
}

/// Starts the background thread that checks on launch and then daily.
/// Debug builds only check when asked, so a source build is never offered
/// a release over itself.
pub fn start(app: &AppHandle) {
    if cfg!(debug_assertions) {
        tracing::debug!("debug build: automatic update checks are off");
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("splashdown-updates".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            loop {
                if check_due(&app) {
                    let _ = tauri::async_runtime::block_on(check(&app, false));
                }
                std::thread::sleep(TICK);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("couldn't start update checks: {e}");
    }
}

/// Automatic checks are on and the last answer was a day ago (or never).
fn check_due(app: &AppHandle) -> bool {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return false;
    };
    if !state.settings().general.check_for_updates() {
        return false;
    }
    let last = *state
        .updates
        .last_success
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    last.is_none_or(|at| at.elapsed().unwrap_or(CHECK_INTERVAL) >= CHECK_INTERVAL)
}

/// Asks the release server for a newer version and returns it, if there is
/// one. Only a check the user asked for (`manual`) reports a failure in
/// Settings; automatic ones just log it.
pub async fn check(app: &AppHandle, manual: bool) -> Result<Option<String>, String> {
    let updates = &app.state::<crate::state::AppState>().inner().updates;
    if updates.busy.swap(true, Ordering::SeqCst) {
        return Ok(updates.available());
    }
    updates.update(app, |s| {
        s.checking = true;
        s.error = None;
    });

    let reply = match find(app).await {
        Ok(update) => {
            let version = update.as_ref().map(|u| u.version.clone());
            match &version {
                Some(v) => tracing::info!("update available: {v}"),
                None => tracing::info!("no update available"),
            }
            let notes = update
                .as_ref()
                .and_then(|u| u.body.clone())
                .filter(|b| !b.trim().is_empty());
            *updates.pending.lock().unwrap_or_else(|e| e.into_inner()) = update;
            *updates
                .last_success
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(SystemTime::now());
            updates.update(app, |s| {
                s.available = version.clone();
                s.notes = notes;
                s.last_checked = Some(now_millis());
                s.checking = false;
            });
            Ok(version)
        }
        Err(e) => {
            tracing::info!("couldn't check for updates: {e}");
            let message = "Couldn't reach the update server. Check your connection and try again.";
            updates.update(app, |s| {
                s.checking = false;
                if manual {
                    s.error = Some(message.into());
                }
            });
            Err(message.to_string())
        }
    };
    updates.busy.store(false, Ordering::SeqCst);
    reply
}

async fn find(app: &AppHandle) -> Result<Option<Update>, Error> {
    let updater = app.updater_builder().timeout(REQUEST_TIMEOUT).build()?;
    match updater.check().await {
        // No release manifest at the address (404): nothing published yet.
        Err(Error::ReleaseNotFound) => Ok(None),
        other => other,
    }
}

/// Downloads and installs the update found by the last check, then
/// relaunches. Only ever called from the user's click.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let updates = &app.state::<crate::state::AppState>().inner().updates;
    let update = updates
        .pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or("There's no update to install. Check again.")?;
    // On Windows the installer takes over at once: a meeting would be
    // cut off mid-sentence.
    if crate::meeting::is_running() {
        return Err("Stop the meeting first, then install the update.".into());
    }
    if updates.busy.swap(true, Ordering::SeqCst) {
        return Err("Already checking for or installing an update.".into());
    }
    tracing::info!("installing update {}", update.version);
    updates.update(app, |s| {
        s.installing = true;
        s.progress = Some(0);
        s.error = None;
    });

    let mut done: u64 = 0;
    let mut shown: u8 = 0;
    let result = update
        .download_and_install(
            |chunk, total| {
                done += chunk as u64;
                let Some(total) = total.filter(|t| *t > 0) else {
                    return;
                };
                let percent = (done * 100 / total).min(100) as u8;
                if percent != shown {
                    shown = percent;
                    updates.update(app, |s| s.progress = Some(percent));
                }
            },
            // Downloaded, about to install. On Windows the installer then
            // ends this app at once, skipping what quitting does: put the
            // volume back, and stop the local model's helper, whose file
            // the installer couldn't replace while it runs.
            || {
                splashdown_platform::restore_output();
                splashdown_providers::stop_all_helpers();
            },
        )
        .await;

    match result {
        Ok(()) => {
            tracing::info!("update installed; relaunching");
            // Restarting skips the exit event.
            splashdown_platform::restore_output();
            app.restart();
        }
        Err(e) => {
            tracing::warn!("update failed: {e}");
            updates.busy.store(false, Ordering::SeqCst);
            let message = format!("Couldn't install the update: {e}");
            updates.update(app, |s| {
                s.installing = false;
                s.progress = None;
                s.error = Some(message.clone());
            });
            Err(message)
        }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
