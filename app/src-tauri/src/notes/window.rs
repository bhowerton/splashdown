//! The Notes window: opening it, remembering the app that was in front
//! (for "Paste into {app}"), and pasting a note back into that app.

use super::*;

/// The app that was in front when the window opened: (pid, name).
pub(super) fn target() -> &'static Mutex<Option<(i32, String)>> {
    static TARGET: OnceLock<Mutex<Option<(i32, String)>>> = OnceLock::new();
    TARGET.get_or_init(Mutex::default)
}

pub(super) fn target_lock() -> std::sync::MutexGuard<'static, Option<(i32, String)>> {
    target().lock().unwrap_or_else(|e| e.into_inner())
}

/// Apps that take no text, so Send never goes to them: the file manager,
/// the system's settings, the desktop and the like (bundle IDs on macOS,
/// executable names on Windows).
pub(super) const NO_TEXT: &[&str] = &[
    "com.apple.finder",
    "com.apple.systempreferences",
    "com.apple.dock",
    "com.apple.loginwindow",
    "com.apple.controlcenter",
    "com.apple.notificationcenterui",
    "com.apple.Spotlight",
    "explorer.exe",
    "SystemSettings.exe",
    "SearchHost.exe",
    "StartMenuExperienceHost.exe",
    "ShellExperienceHost.exe",
    "LockApp.exe",
];

/// Remembers the app in front as where Send goes, unless it's us
/// (Settings, Notes, Ask) or takes no text: then the one before stays.
/// Returns whether it changed.
pub(super) fn remember_front_app() -> bool {
    let Some((pid, name, id)) = splashdown_platform::frontmost_app() else {
        return false;
    };
    if u32::try_from(pid).ok() == Some(std::process::id())
        || NO_TEXT.iter().any(|n| n.eq_ignore_ascii_case(&id))
    {
        return false;
    }
    let mut target = target_lock();
    let changed = target.as_ref().is_none_or(|(p, _)| *p != pid);
    *target = Some((pid, name));
    changed
}

/// Keeps track of the last app used, so Send goes where you just were even
/// when Notes stayed open while you worked elsewhere. Twice a second; it's
/// only asking which app is in front.
pub fn watch_front_app(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("front-app".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(500));
                if remember_front_app()
                    && let Some(window) = app.get_webview_window(WINDOW)
                    && window.is_visible().unwrap_or(false)
                {
                    let _ = app.emit_to(WINDOW, TARGET, target_name());
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("couldn't follow the app in front: {e}");
    }
}

pub(super) fn target_name() -> Option<String> {
    target_lock()
        .as_ref()
        .map(|(_, name)| name.clone())
        .filter(|n| !n.is_empty())
}

/// Whether the Notes window is in front with focus.
pub fn is_focused(app: &AppHandle) -> bool {
    app.get_webview_window(WINDOW)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

/// ⌃⌥N: hides Notes if it has focus, otherwise opens it.
pub fn toggle(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW)
        && window.is_visible().unwrap_or(false)
        && window.is_focused().unwrap_or(false)
    {
        let _ = window.hide();
        // Hand focus back rather than leaving no app in front.
        if let Some((pid, _)) = *target_lock() {
            let _ = splashdown_platform::activate_app(pid);
        }
        return;
    }
    open(app);
}

/// Shows the Notes window (creating it the first time) with the note
/// focused.
pub fn open(app: &AppHandle) {
    open_to(app, None);
}

/// `open`, then a new note or today's note (`then`: "new" or "daily"),
/// for voice actions.
pub fn open_to(app: &AppHandle, then: Option<&str>) {
    remember_front_app();
    read_for_meaning(app);
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to(WINDOW, SHOWN, target_name());
        if let Some(then) = then {
            let _ = app.emit_to(WINDOW, THEN, then);
        }
        return;
    }
    let page = match then {
        Some(then) => format!("notes.html?then={then}"),
        None => "notes.html".into(),
    };
    let result = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App(page.into()))
        .title("Notes")
        .inner_size(1100.0, 680.0)
        .min_inner_size(420.0, 300.0)
        .resizable(true)
        // A window of its own, unless the user keeps it on top to dictate
        // into while they look at another app.
        .always_on_top(
            app.try_state::<AppState>()
                .is_some_and(|s| s.settings().notes.keep_on_top),
        )
        // Comes up on the Space (desktop) the user is on, not the one it was
        // first opened on.
        .visible_on_all_workspaces(true)
        .center()
        .focused(true)
        .build();
    match result {
        Ok(window) => {
            // Closing only hides it, so the page (and anything not yet
            // saved) stays.
            let handle = window.clone();
            let app = app.clone();
            window.on_window_event(move |event| match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = handle.hide();
                }
                // Audio and video files dropped on the window are
                // transcribed.
                // Files only: dragging text or a link isn't a transcription.
                WindowEvent::DragDrop(DragDropEvent::Enter { paths, .. }) if !paths.is_empty() => {
                    let _ = app.emit_to(WINDOW, files::DRAGGING, true);
                }
                WindowEvent::DragDrop(DragDropEvent::Leave) => {
                    let _ = app.emit_to(WINDOW, files::DRAGGING, false);
                }
                WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
                    let _ = app.emit_to(WINDOW, files::DRAGGING, false);
                    files::dropped(&app, paths.clone());
                }
                _ => {}
            });
        }
        Err(e) => tracing::error!("failed to open Notes: {e}"),
    }
}

/// Hides Notes, brings back the app that was in front before it opened,
/// and pastes `text` there. On failure the note is left on the clipboard
/// and the window comes back.
pub(super) fn send(app: &AppHandle, text: &str) -> Result<(), String> {
    let Some((pid, name)) = target_lock().clone() else {
        return Err("No app to send to. Copy the note instead.".into());
    };
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.hide();
    }
    if let Err(e) = paste_into(pid, text) {
        tracing::warn!("notes send failed: {e}");
        let _ = splashdown_platform::copy_to_clipboard(text);
        if let Some(window) = app.get_webview_window(WINDOW) {
            let _ = window.show();
            let _ = window.set_focus();
        }
        return Err(format!(
            "Couldn't paste into {name}. It's on the clipboard."
        ));
    }
    Ok(())
}

/// Brings the app `pid` to the front and pastes `text` there. Blocks for
/// up to a couple of seconds.
pub(crate) fn paste_into(pid: i32, text: &str) -> Result<(), String> {
    splashdown_platform::activate_app(pid)
        .map_err(|e| e.to_string())
        .and_then(|()| wait_for_front(pid))
        .and_then(|()| {
            std::thread::sleep(SETTLE);
            splashdown_platform::insert_text(text, InsertMethod::Paste).map_err(|e| e.to_string())
        })
}

pub(super) fn wait_for_front(pid: i32) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < ACTIVATE_TIMEOUT {
        if splashdown_platform::frontmost_app().is_some_and(|(p, _, _)| p == pid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err("the app didn't come to the front".into())
}
