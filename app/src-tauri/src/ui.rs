//! Bridges the session thread to UI that must live on the main thread: the
//! status overlay, sounds and the tray status line.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use splashdown_config::Preset;
use splashdown_platform::{Overlay, OverlayState, Sound};
use tauri::AppHandle;

use crate::tray::{TrayIconState, TrayMenu};

thread_local! {
    /// Main-thread only.
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

/// Fade steps (opacity) and their spacing.
const FADE_IN: [f64; 4] = [0.25, 0.5, 0.75, 1.0];
const FADE_OUT: [f64; 3] = [0.7, 0.4, 0.15];
const FADE_STEP: Duration = Duration::from_millis(22);

/// Creates the overlay. Call on the main thread during setup.
pub fn init_overlay() {
    match Overlay::new() {
        Ok(o) => OVERLAY.with(|cell| *cell.borrow_mut() = Some(o)),
        Err(e) => tracing::warn!("no status overlay: {e}"),
    }
}

/// App event: rebuild the engine (see `Ui::rebuild_engine`).
pub const REBUILD_ENGINE: &str = "rebuild-engine";

/// A cheap, cloneable handle the session thread uses to update the UI.
#[derive(Clone)]
pub struct Ui {
    app: AppHandle,
    tray: TrayMenu,
    /// Bumped by every show/hide so an older fade stops early.
    generation: Arc<AtomicU64>,
    /// Whether the overlay is (or is becoming) visible.
    shown: Arc<AtomicBool>,
}

impl Ui {
    pub fn new(app: AppHandle, tray: TrayMenu) -> Self {
        Self {
            app,
            tray,
            generation: Arc::default(),
            shown: Arc::default(),
        }
    }

    pub fn app(&self) -> &AppHandle {
        &self.app
    }

    /// Reads the settings file again (changed by the CLI) and applies it.
    /// An open Settings window is told, so it shows the new settings rather
    /// than saving its old copy over them.
    pub fn reload_settings(&self) {
        use tauri::Manager;
        let Some(state) = self.app.try_state::<crate::commands::AppState>() else {
            return;
        };
        let settings = crate::engine::load_settings();
        crate::commands::show_settings(&self.app, &state, &settings);
        *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<crate::commands::AppState>();
            if let Some(e) = state.apply().await {
                tracing::warn!("settings reloaded, but: {e}");
            }
            crate::commands::settings_changed_elsewhere(&app);
        });
    }

    /// The settings as last saved, for things that don't depend on the
    /// engine being usable (notes).
    pub fn settings(&self) -> Option<splashdown_config::Settings> {
        use tauri::Manager;
        self.app
            .try_state::<crate::commands::AppState>()
            .map(|state| state.settings())
    }

    pub fn show(&self, state: OverlayState) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let appearing = !self.shown.swap(true, Ordering::SeqCst);
        let announcement = announcement(&state);
        crate::minibar::working(matches!(state, OverlayState::Working { .. }));
        let ui = self.clone();
        self.on_overlay(move |o| {
            let fade = appearing && !splashdown_platform::reduce_motion();
            o.set_alpha(if fade { 0.0 } else { 1.0 });
            o.show(&state);
            o.announce(&announcement);
            if fade {
                ui.fade(generation, &FADE_IN, false);
            }
        });
    }

    /// Shows the overlay by the text cursor (`None`: the usual place).
    pub fn anchor(&self, caret: Option<splashdown_platform::CaretRect>) {
        if let Some(c) = &caret {
            tracing::debug!(
                "text cursor at {:.0},{:.0}, {:.0}×{:.0}",
                c.x,
                c.y,
                c.width,
                c.height
            );
        }
        self.on_overlay(move |o| o.set_anchor(caret));
    }

    pub fn level(&self, level: f32) {
        self.on_overlay(move |o| o.set_level(level));
    }

    pub fn live_text(&self, text: Option<String>) {
        self.on_overlay(move |o| o.set_live_text(text.as_deref()));
    }

    /// Shows or clears the "↵ Send" badge, announcing the change.
    pub fn set_send(&self, on: bool) {
        self.on_overlay(move |o| {
            o.set_send(on);
            o.announce(if on {
                "Will press Return"
            } else {
                "Won't press Return"
            });
        });
    }

    pub fn hide(&self) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        crate::minibar::working(false);
        if !self.shown.swap(false, Ordering::SeqCst) {
            return;
        }
        let ui = self.clone();
        self.on_overlay(move |o| {
            if splashdown_platform::reduce_motion() {
                o.hide();
            } else {
                ui.fade(generation, &FADE_OUT, true);
            }
        });
    }

    /// Re-reads the keyboard layout for synthetic shortcuts (main thread).
    pub fn refresh_shortcut_keys(&self) {
        let _ = self
            .app
            .run_on_main_thread(splashdown_platform::refresh_shortcut_keys);
    }

    /// Asks the app to rebuild the engine from current settings (the
    /// spending limit was reached or lifted). Handled in `main.rs`.
    pub fn rebuild_engine(&self) {
        use tauri::Emitter;
        let _ = self.app.emit(REBUILD_ENGINE, ());
    }

    pub fn sound(&self, sound: Sound, volume: f32) {
        let _ = self
            .app
            .run_on_main_thread(move || splashdown_platform::play_sound(sound, volume));
    }

    /// The disabled first line of the tray menu.
    pub fn status(&self, text: &str) {
        if let Err(e) = self.tray.status.set_text(text) {
            tracing::warn!("couldn't update tray status: {e}");
        }
    }

    pub fn tray_icon(&self, state: TrayIconState) {
        self.tray.set_icon(&self.app, state);
        crate::minibar::tray_state(state);
    }

    pub fn tray_presets(&self, presets: &[Preset], pinned: Option<&str>) {
        self.tray.set_presets(&self.app, presets, pinned);
        let name = pinned.and_then(|id| presets.iter().find(|p| p.id == id));
        crate::minibar::pinned_preset(name.map(|p| p.name.clone()));
    }

    /// Steps the overlay's opacity through `steps`, then hides it if
    /// `hide_after` is set. Stops as soon as a newer show/hide happens.
    fn fade(&self, generation: u64, steps: &'static [f64], hide_after: bool) {
        let ui = self.clone();
        std::thread::spawn(move || {
            for &alpha in steps {
                std::thread::sleep(FADE_STEP);
                if ui.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                ui.on_overlay(move |o| o.set_alpha(alpha));
            }
            if hide_after {
                std::thread::sleep(FADE_STEP);
                let current = ui.generation.clone();
                ui.on_overlay(move |o| {
                    if current.load(Ordering::SeqCst) == generation {
                        o.hide();
                        o.set_alpha(1.0);
                    }
                });
            }
        });
    }

    fn on_overlay(&self, f: impl FnOnce(&Overlay) + Send + 'static) {
        let _ = self.app.run_on_main_thread(move || {
            OVERLAY.with(|cell| {
                if let Some(o) = cell.borrow().as_ref() {
                    f(o);
                }
            });
        });
    }
}

/// What VoiceOver says for each state.
fn announcement(state: &OverlayState) -> String {
    match state {
        OverlayState::Listening { preset } => match preset {
            Some(p) => format!("Listening, {p}"),
            None => "Listening".into(),
        },
        OverlayState::Instruction => "Instruction".into(),
        OverlayState::Transform { .. } => "Transform selection".into(),
        OverlayState::HandsFree { .. } => "Hands-free".into(),
        OverlayState::Question { text } => format!("Question: {text}"),
        OverlayState::Working { label } => label.trim_end_matches('…').into(),
        OverlayState::Message { text, .. } => text.clone(),
    }
}
