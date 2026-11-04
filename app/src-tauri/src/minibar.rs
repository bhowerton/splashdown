//! The mini bar (setting `general.mini_bar`): a small button that stays on
//! screen for people who'd rather click than hold a key. Clicking it does
//! what `splash toggle` does: starts a hands-free dictation into the
//! focused field, or finishes the one in progress. It never takes focus
//! (see `splashdown_platform::MiniBar`), shows whether Splashdown is
//! recording or working, and remembers where it was dragged to.
//!
//! The bar lives on the main thread. The session reports its state through
//! `Ui` (`tray_state`, `working`, `pinned_preset`); clicks go back to the
//! session from a spawned thread, because `Handle::control` waits.

use std::cell::RefCell;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use splashdown_control::Request;
use splashdown_platform::{MiniBar, MiniBarEvent, MiniBarPosition, MiniBarState};
use tauri::{AppHandle, Manager};

use crate::state::AppState;
use crate::tray::TrayIconState;

static APP: OnceLock<AppHandle> = OnceLock::new();

/// The right-click menu, in order (see `on_event`).
const MENU: [&str; 2] = ["Settings…", "Hide mini bar"];

#[derive(Default)]
struct Model {
    /// Created the first time the bar is shown.
    bar: Option<MiniBar>,
    /// Creating it failed (logged once; not retried).
    failed: bool,
    visible: bool,
    /// The tray icon's state: recording, idle, or not set up.
    tray: Option<TrayIconState>,
    /// The overlay says "Working…".
    working: bool,
    /// The preset chosen in the menu bar, by name.
    preset: Option<String>,
}

thread_local! {
    /// Main-thread only.
    static MODEL: RefCell<Model> = RefCell::default();
}

impl Model {
    fn state(&self) -> MiniBarState {
        match (self.working, self.tray) {
            (true, _) => MiniBarState::Working,
            (false, Some(TrayIconState::Recording)) => MiniBarState::Recording,
            (false, Some(TrayIconState::Attention)) => MiniBarState::NotReady,
            (false, _) => MiniBarState::Idle,
        }
    }

    fn tooltip(&self, state: MiniBarState) -> String {
        match state {
            MiniBarState::Idle => match &self.preset {
                Some(p) => format!("Click to dictate · {p}"),
                None => "Click to dictate".into(),
            },
            MiniBarState::Recording => "Click to finish".into(),
            MiniBarState::Working => "Working…".into(),
            MiniBarState::NotReady => "Dictation isn't set up yet: see Settings".into(),
        }
    }

    /// Brings the bar in line with the model, creating it if needed.
    fn refresh(&mut self) {
        if self.visible && self.bar.is_none() && !self.failed {
            match MiniBar::new(load_position(), &MENU, Box::new(on_event)) {
                Ok(bar) => self.bar = Some(bar),
                Err(e) => {
                    tracing::warn!("no mini bar: {e}");
                    self.failed = true;
                }
            }
        }
        let Some(bar) = &self.bar else {
            return;
        };
        tracing::debug!(
            "mini bar: {}",
            if self.visible { "shown" } else { "hidden" }
        );
        let state = self.state();
        bar.set_state(state);
        bar.set_tooltip(&self.tooltip(state));
        if self.visible {
            bar.show();
        } else {
            bar.hide();
        }
    }
}

/// Call on the main thread during setup, before the session starts.
pub fn init(app: &AppHandle, visible: bool) {
    let _ = APP.set(app.clone());
    MODEL.with(|m| {
        let mut m = m.borrow_mut();
        m.visible = visible;
        m.refresh();
    });
}

/// Shows or hides the bar (the setting changed). Any thread.
pub fn set_visible(visible: bool) {
    on_main(move |m| m.visible = visible);
}

/// The tray icon's new state, which the bar follows. Any thread.
pub fn tray_state(state: TrayIconState) {
    on_main(move |m| m.tray = Some(state));
}

/// Whether the overlay shows "Working…". Any thread.
pub fn working(on: bool) {
    on_main(move |m| m.working = on);
}

/// The preset chosen in the menu bar (`None`: by app). Any thread.
pub fn pinned_preset(name: Option<String>) {
    on_main(move |m| m.preset = name);
}

fn on_main(f: impl FnOnce(&mut Model) + Send + 'static) {
    let Some(app) = APP.get() else {
        return;
    };
    let _ = app.run_on_main_thread(move || {
        MODEL.with(|m| {
            let mut m = m.borrow_mut();
            f(&mut m);
            m.refresh();
        });
    });
}

/// Runs on the main thread, called by the bar.
fn on_event(event: MiniBarEvent) {
    let Some(app) = APP.get() else {
        return;
    };
    match event {
        MiniBarEvent::Click => {
            let app = app.clone();
            std::thread::spawn(move || {
                let Some(state) = app.try_state::<AppState>() else {
                    return;
                };
                // Failures (not set up, no microphone) are already on the
                // overlay.
                if let Err(e) = state.session.control(Request::Toggle) {
                    tracing::info!("mini bar: {e}");
                }
            });
        }
        MiniBarEvent::Moved(position) => save_position(position),
        MiniBarEvent::Menu(0) => crate::tray::open_settings(app),
        MiniBarEvent::Menu(_) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(state) = app.try_state::<AppState>()
                    && let Err(e) = state.set_mini_bar(&app, false)
                {
                    tracing::warn!("couldn't hide the mini bar: {e}");
                }
            });
        }
    }
}

/// `mini-bar.json`: where the bar was last dropped.
#[derive(Serialize, Deserialize)]
struct Saved {
    x: f64,
    y: f64,
}

fn load_position() -> Option<MiniBarPosition> {
    let path = crate::engine::paths().ok()?.mini_bar_file();
    let saved: Saved = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(MiniBarPosition {
        x: saved.x,
        y: saved.y,
    })
}

fn save_position(position: MiniBarPosition) {
    let result = crate::engine::paths().and_then(|paths| {
        let json = serde_json::to_vec(&Saved {
            x: position.x,
            y: position.y,
        })
        .map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&paths.config_dir).map_err(|e| e.to_string())?;
        std::fs::write(paths.mini_bar_file(), json).map_err(|e| e.to_string())
    });
    if let Err(e) = result {
        tracing::warn!("couldn't save the mini bar's place: {e}");
    }
}
