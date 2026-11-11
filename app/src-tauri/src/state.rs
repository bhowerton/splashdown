//! The app's shared state (`AppState`): the settings, the dictation
//! session, history, the tray menu and updates, as Tauri manages it for
//! every command. Also the one way to change settings from outside the
//! Settings window (`AppState::update`).

use splashdown_core::sync::Locked;
use std::sync::{Arc, Mutex};

use splashdown_config::Settings;
use tauri::{AppHandle, Emitter, Manager};

use crate::engine;
use crate::history::History;
use crate::session;
use crate::snapshot::*;

/// Shared app state for commands.
pub struct AppState {
    pub settings: Mutex<Settings>,
    pub session: session::Handle,
    /// Why dictation can't run with the current settings, if it can't.
    pub engine_error: Mutex<Option<String>>,
    pub history: Arc<History>,
    /// The menu bar menu, to keep its language ticks in sync.
    pub tray: crate::tray::TrayMenu,
    pub updates: crate::updates::Updates,
}

impl AppState {
    /// Changes the settings from anywhere but the Settings window (which
    /// saves through `save_settings`): the one way to do it. `change` runs
    /// while the settings are locked, so nothing else changes them in
    /// between; the result is saved, kept, reflected in the menu bar, mini
    /// bar and shortcuts, and an open Settings window reads it again rather
    /// than saving an older copy over it. When the change matters to
    /// dictation, rebuild the engine afterwards (`apply`, or
    /// `rebuild_in_background`).
    pub fn update(
        &self,
        app: &AppHandle,
        change: impl FnOnce(&mut Settings),
    ) -> Result<Settings, String> {
        let settings = {
            let mut current = self.settings.locked();
            let mut settings = current.clone();
            change(&mut settings);
            splashdown_config::managed::Managed::current().apply(&mut settings);
            settings.save(&engine::paths()?.config_file())?;
            *current = settings.clone();
            settings
        };
        reflect_settings(app, self, &settings);
        settings_changed_elsewhere(app);
        Ok(settings)
    }

    /// Flips whisper mode (a voice action), saves, and rebuilds.
    pub async fn toggle_whisper_mode(&self, app: &AppHandle) -> Result<(), String> {
        self.update(app, |s| {
            s.general.whisper_mode = Some(!s.general.whisper_mode());
        })?;
        if let Some(e) = self.apply().await {
            tracing::warn!("after changing whisper mode: {e}");
        }
        Ok(())
    }

    /// Shows or hides the mini bar (from the bar's own menu) and saves
    /// the setting. Nothing to rebuild.
    pub fn set_mini_bar(&self, app: &AppHandle, on: bool) -> Result<(), String> {
        self.update(app, |s| s.general.mini_bar = on.then_some(true))
            .map(|_| ())
    }

    /// Sets the spoken language (from the menu bar), saves, and rebuilds.
    pub async fn set_language(
        &self,
        app: &AppHandle,
        language: Option<String>,
    ) -> Result<(), String> {
        self.update(app, |s| s.general.language = language)?;
        if let Some(e) = self.apply().await {
            tracing::warn!("after changing the language: {e}");
        }
        Ok(())
    }

    pub fn settings(&self) -> Settings {
        self.settings.locked().clone()
    }

    /// Rebuilds the engine from current settings and keys, and hands it to
    /// the session. Returns the engine error, if any.
    pub async fn apply(&self) -> Option<String> {
        let settings = self.settings();
        self.history.set_keep_days(settings.general.history_days());
        let keys = engine::keys(&settings);
        let built = tauri::async_runtime::spawn_blocking(move || engine::build(&settings))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        let error = built.as_ref().err().cloned();
        *self.engine_error.locked() = error.clone();
        self.session.reconfigure(built, keys);
        error
    }

    pub fn snapshot(&self) -> Snapshot {
        let error = self.engine_error.locked().clone();
        snapshot(self.settings(), error, self.updates.status())
    }

    /// Opens a new bug report, prefilled with the version, OS and the
    /// providers in use. Never keys, dictated text or file paths.
    pub fn send_feedback(&self) -> Result<(), String> {
        let snap = self.snapshot();
        let speech = snap.resolved_speech.as_deref().unwrap_or("not set up");
        let ai = snap.resolved_ai.as_deref().unwrap_or("not set up");
        let url = crate::feedback::issue_url(
            snap.version,
            splashdown_platform::os_version().as_deref(),
            &format!("Speech to text: {speech} · Cleanup: {ai}"),
        );
        splashdown_platform::open_url(&url).map_err(|e| e.to_string())
    }
}

/// Rebuilds the engine without waiting for it (`AppState::apply`).
pub fn rebuild_in_background(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(e) = app.state::<AppState>().apply().await {
            tracing::warn!("after a settings change: {e}");
        }
    });
}

/// Puts settings to use where they show outside the engine: the menu
/// bar's language, the mini bar and the global shortcuts.
pub fn reflect_settings(app: &AppHandle, state: &AppState, settings: &Settings) {
    state
        .tray
        .show_language(settings.general.language.as_deref());
    state.tray.show_translate(app, settings);
    crate::minibar::set_visible(settings.general.mini_bar());
    crate::shortcuts::sync(app, &settings.presets(), &settings.transforms());
}

/// Settings were saved from somewhere other than the Settings window (the
/// menu bar, a meeting's question, the call offer): an open Settings
/// window reads them again rather than saving its older copy over them.
pub fn settings_changed_elsewhere(app: &AppHandle) {
    let _ = app.emit_to("settings", "settings-reloaded", ());
}
