//! New releases (tauri-plugin-updater). Checks GitHub on launch and once a
//! day when the user allows it, shows what it found in the menu bar and in
//! Settings → About, and installs only when the user clicks. A failed check
//! (offline, nothing published yet) is logged and otherwise ignored.
//!
//! With `general.install_updates` on, a release installs by itself, but
//! only once it has been out a couple of days (`SETTLED`: a bad release is
//! usually found by then) and nothing is going on (`idle`), since
//! installing restarts the app. The next launch says what it was updated
//! to, in the menu bar and in Settings → About, for a day.

use splashdown_core::sync::Locked;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Error, Update, UpdaterExt};

use crate::tray::TrayMenu;

/// App event with the new `Status` whenever it changes.
pub const UPDATE_STATUS: &str = "update-status";

/// Long enough for launch work (models, permissions) to settle first.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the background thread wakes to see whether a check is due,
/// or whether now is a good time to install. Wall-clock based, so a Mac
/// that slept through the day still checks.
const TICK: Duration = Duration::from_secs(10 * 60);
/// A check that got no answer (offline) is tried again after this long.
const RETRY: Duration = Duration::from_secs(60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// A release has to have been out this long before it installs unasked.
const SETTLED: Duration = Duration::from_secs(2 * 24 * 60 * 60);
/// …and Splashdown has to have been left alone this long.
const QUIET: Duration = Duration::from_secs(10 * 60);
/// How long the menu bar and Settings say what it was updated to.
const SAID_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// When Splashdown was last doing something for the user (Unix
/// milliseconds): see `active`.
static LAST_ACTIVE: AtomicU64 = AtomicU64::new(0);

/// Something is going on (the overlay showed, sound is coming in, a hook
/// is running): not the time for an update to restart the app. Cheap, and
/// called from any thread.
pub fn active() {
    LAST_ACTIVE.store(now_millis(), Ordering::Relaxed);
}

/// What an update that installed itself leaves for the next launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Installed {
    /// The version it was before.
    from: String,
    /// The version it installed.
    to: String,
    /// When, in Unix milliseconds.
    at: u64,
}

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
    /// The version an update installed by itself in the last day, to say
    /// so.
    updated: Option<String>,
}

pub struct Updates {
    status: Mutex<Status>,
    pending: Mutex<Option<Update>>,
    /// One check or install at a time.
    busy: AtomicBool,
    /// When a check last got an answer. Failed checks (offline) retry at
    /// soon instead of waiting a day.
    last_success: Mutex<Option<SystemTime>>,
    /// When a check was last tried, whether or not it got an answer.
    last_attempt: Mutex<Option<SystemTime>>,
    tray: TrayMenu,
}

impl Updates {
    pub fn new(tray: TrayMenu) -> Self {
        Self {
            status: Mutex::default(),
            pending: Mutex::default(),
            busy: AtomicBool::new(false),
            last_success: Mutex::default(),
            last_attempt: Mutex::default(),
            tray,
        }
    }

    pub fn status(&self) -> Status {
        self.status.locked().clone()
    }

    pub fn available(&self) -> Option<String> {
        self.status().available
    }

    fn update(&self, app: &AppHandle, change: impl FnOnce(&mut Status)) {
        let status = {
            let mut s = self.status.locked();
            change(&mut s);
            s.clone()
        };
        self.tray.set_update(menu_line(&status).as_deref());
        let _ = app.emit(UPDATE_STATUS, status);
    }
}

/// The menu bar's line about updates: one to install, or one that
/// installed itself.
fn menu_line(status: &Status) -> Option<String> {
    let v = |version: &str| version.trim_start_matches('v').to_string();
    match (&status.available, &status.updated) {
        (Some(available), _) => Some(format!("Update to v{}…", v(available))),
        (None, Some(updated)) => Some(format!("Updated to v{}: what changed…", v(updated))),
        (None, None) => None,
    }
}

/// Starts the background thread that checks on launch and then daily.
/// Debug builds only check when asked, so a source build is never offered
/// a release over itself.
pub fn start(app: &AppHandle) {
    // Being launched counts as being used.
    active();
    say_updated(app);
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
                say_updated(&app);
                if check_due(&app) {
                    let _ = tauri::async_runtime::block_on(check(&app, false));
                }
                if install_due(&app) {
                    tracing::info!("nothing is going on: installing the update by itself");
                    if let Err(e) = tauri::async_runtime::block_on(install(&app, true)) {
                        tracing::warn!("the update didn't install by itself: {e}");
                    }
                }
                std::thread::sleep(TICK);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("couldn't start update checks: {e}");
    }
}

/// Automatic checks are on, the last answer was a day ago (or never), and
/// a check that got none wasn't tried in the last hour.
fn check_due(app: &AppHandle) -> bool {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return false;
    };
    if !state.settings().general.check_for_updates() {
        return false;
    }
    let since = |at: Option<SystemTime>, least: Duration| {
        at.is_none_or(|at| at.elapsed().unwrap_or(least) >= least)
    };
    since(*state.updates.last_success.locked(), CHECK_INTERVAL)
        && since(*state.updates.last_attempt.locked(), RETRY)
}

/// Whether the update that was found should install now, unasked: the
/// user chose that, the release has been out long enough, and nothing is
/// going on.
fn install_due(app: &AppHandle) -> bool {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return false;
    };
    if !state.settings().general.install_updates() {
        return false;
    }
    let Some(update) = state.updates.pending.locked().clone() else {
        return false;
    };
    let published = update.date.map(|d| d.unix_timestamp());
    settled(published, (now_millis() / 1000) as i64) && idle(app)
}

/// Whether a release published at `published` (Unix seconds) has been out
/// long enough to install unasked. One that doesn't say when it was
/// published waits for a click.
fn settled(published: Option<i64>, now: i64) -> bool {
    published.is_some_and(|at| now.saturating_sub(at) >= SETTLED.as_secs() as i64)
}

/// Whether `last_active` (Unix milliseconds) is long enough ago.
fn quiet(last_active: u64, now: u64) -> bool {
    now.saturating_sub(last_active) >= QUIET.as_millis() as u64
}

/// Nothing is going on that a restart would cut off or get in the way of:
/// no dictation or hook lately, no meeting, no file being transcribed, no
/// model downloading, and none of Splashdown's windows open.
fn idle(app: &AppHandle) -> bool {
    quiet(LAST_ACTIVE.load(Ordering::Relaxed), now_millis())
        && !crate::meeting::is_running()
        && crate::notes::files::notes_transcribing().is_none()
        && !crate::setup::working()
        && crate::models::downloads().is_empty()
        && !app
            .webview_windows()
            .values()
            .any(|w| w.is_visible().unwrap_or(false))
}

/// Whether to still say that `installed` happened: this is the version it
/// installed, and it was within the last day.
fn still_said(installed: &Installed, version: &str, now: u64) -> bool {
    installed.to.trim_start_matches('v') == version.trim_start_matches('v')
        && now.saturating_sub(installed.at) < SAID_FOR.as_millis() as u64
}

/// Says in the menu bar and Settings what an update installed by itself
/// (for a day), and forgets it after that or when it didn't take.
fn say_updated(app: &AppHandle) {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    let Ok(path) = crate::engine::paths().map(|p| p.updated_file()) else {
        return;
    };
    let installed: Option<Installed> = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let version = app.package_info().version.to_string();
    let said = installed
        .as_ref()
        .filter(|i| still_said(i, &version, now_millis()))
        .map(|i| i.to.clone());
    if said.is_none() && path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    if state.updates.status().updated != said {
        state.updates.update(app, |s| s.updated = said);
    }
}

/// Leaves word for the next launch that this version installed `to` by
/// itself.
fn leave_word(app: &AppHandle, to: &str) {
    let installed = Installed {
        from: app.package_info().version.to_string(),
        to: to.to_string(),
        at: now_millis(),
    };
    let saved =
        crate::engine::paths().and_then(|p| crate::data::write(&p.updated_file(), &installed));
    if let Err(e) = saved {
        tracing::warn!("couldn't note the update for the next launch: {e}");
    }
}

/// Asks the release server for a newer version and returns it, if there is
/// one. Only a check the user asked for (`manual`) reports a failure in
/// Settings; automatic ones just log it.
pub async fn check(app: &AppHandle, manual: bool) -> Result<Option<String>, String> {
    let updates = &app.state::<crate::state::AppState>().inner().updates;
    if updates.busy.swap(true, Ordering::SeqCst) {
        return Ok(updates.available());
    }
    *updates.last_attempt.locked() = Some(SystemTime::now());
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
            *updates.pending.locked() = update;
            *updates.last_success.locked() = Some(SystemTime::now());
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
/// relaunches. From the user's click, or `unasked` when they chose to have
/// updates install by themselves and `install_due` says now is the time.
pub async fn install(app: &AppHandle, unasked: bool) -> Result<(), String> {
    let updates = &app.state::<crate::state::AppState>().inner().updates;
    let update = updates
        .pending
        .locked()
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
                // Nobody clicked, so the next launch says what happened.
                if unasked {
                    leave_word(app, &update.version);
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_installs_unasked_only_once_it_has_been_out_two_days() {
        let now = 1_800_000_000;
        let day = 24 * 60 * 60;
        assert!(!settled(Some(now - day), now));
        assert!(!settled(Some(now - 2 * day + 1), now));
        assert!(settled(Some(now - 2 * day), now));
        // A date in the future (a wrong clock) isn't "long ago".
        assert!(!settled(Some(now + day), now));
        // No date: it waits for a click.
        assert!(!settled(None, now));
    }

    #[test]
    fn idle_means_left_alone_for_ten_minutes() {
        let now = 1_800_000_000_000;
        assert!(!quiet(now, now));
        assert!(!quiet(now - 9 * 60 * 1000, now));
        assert!(quiet(now - 10 * 60 * 1000, now));
        // A time in the future (the clock was changed) isn't long ago.
        assert!(!quiet(now + 5, now));
    }

    #[test]
    fn what_was_updated_is_said_for_a_day_and_only_if_it_took() {
        let now = 1_800_000_000_000;
        let hour = 60 * 60 * 1000;
        let installed = Installed {
            from: "0.12.0".into(),
            to: "0.12.1".into(),
            at: now - hour,
        };
        assert!(still_said(&installed, "0.12.1", now));
        assert!(still_said(&installed, "v0.12.1", now));
        // Still the old version: the install didn't take.
        assert!(!still_said(&installed, "0.12.0", now));
        assert!(!still_said(&installed, "0.12.1", now + 24 * hour));
    }

    #[test]
    fn the_menu_bar_says_what_to_install_or_what_was() {
        let line = |available: Option<&str>, updated: Option<&str>| {
            menu_line(&Status {
                available: available.map(String::from),
                updated: updated.map(String::from),
                ..Status::default()
            })
        };
        assert_eq!(line(None, None), None);
        assert_eq!(
            line(Some("v0.12.2"), None).as_deref(),
            Some("Update to v0.12.2…")
        );
        assert_eq!(
            line(None, Some("0.12.1")).as_deref(),
            Some("Updated to v0.12.1: what changed…")
        );
        // A newer one to install matters more than the last one.
        assert_eq!(
            line(Some("0.12.2"), Some("0.12.1")).as_deref(),
            Some("Update to v0.12.2…")
        );
    }
}
