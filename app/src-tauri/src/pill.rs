//! The meeting pill: a small bar at the top of the screen while a meeting
//! records, with the time, both sides' levels and Stop. Windows can't show
//! the time next to the tray icon, so it's on there by default; on macOS
//! the menu bar shows it (unless the notch hides it), and the pill is a
//! setting (Settings → Notes → Meetings).

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::meeting::Status;
use crate::state::AppState;

pub const WINDOW: &str = "meeting-pill";
/// The same event the Notes window gets, with the same `Status`.
const STATUS: &str = "meeting-status";

/// Shows the pill with `status`, or hides it when no meeting is recording
/// (or the pill is turned off).
pub fn update(app: &AppHandle, status: Option<&Status>) {
    let wanted = status.is_some()
        && app
            .try_state::<AppState>()
            .is_some_and(|s| s.settings().notes.meeting_pill());
    let window = match (app.get_webview_window(WINDOW), wanted) {
        (Some(window), false) => {
            let _ = window.hide();
            return;
        }
        (None, false) => return,
        (Some(window), true) => window,
        (None, true) => match create(app) {
            Some(window) => window,
            None => return,
        },
    };
    if !window.is_visible().unwrap_or(false) {
        place(&window);
        let _ = window.show();
    }
    let _ = app.emit_to(WINDOW, STATUS, status);
}

fn create(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    let built = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("pill.html".into()))
        .title("Meeting")
        .inner_size(300.0, 40.0)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        .visible(false)
        .build();
    match built {
        Ok(window) => {
            // Closing (Alt+F4) only hides it until the next meeting.
            let handle = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = handle.hide();
                }
            });
            Some(window)
        }
        Err(e) => {
            tracing::warn!("couldn't show the meeting pill: {e}");
            None
        }
    }
}

/// Top center of the main screen, just inside the work area.
fn place(window: &tauri::WebviewWindow) {
    crate::placement::place(window, crate::placement::Spot::TopCenter { top: 8.0 });
}
