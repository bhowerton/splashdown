//! Bringing a window the user just asked for to the very front.

use tauri::{AppHandle, WebviewWindow};

/// How long after the first try to do it again: long enough for the menu
/// bar menu to have closed and macOS to have handed the front back to the
/// app that was there before.
#[cfg(target_os = "macos")]
const AGAIN_AFTER: std::time::Duration = std::time::Duration::from_millis(150);

/// Shows `window`, un-minimises it, makes this app the active one and
/// gives the window the keyboard. A menu bar app isn't active, so without
/// the activation a window ends up behind other apps' windows; and the
/// menu closing can return the front to the previous app just after, so on
/// a Mac it's done once more a moment later.
pub fn bring_to_front(app: &AppHandle, window: &WebviewWindow) {
    raise_on_main(app, window);
    #[cfg(target_os = "macos")]
    {
        let (app, window) = (app.clone(), window.clone());
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(AGAIN_AFTER).await;
            // Only if it's still there for them to look at.
            if window.is_visible().unwrap_or(false) {
                raise_on_main(&app, &window);
            }
        });
    }
}

fn raise_on_main(app: &AppHandle, window: &WebviewWindow) {
    let window = window.clone();
    let _ = app.run_on_main_thread(move || {
        splashdown_platform::activate_self();
        let _ = window.show();
        // A minimised window ignores being focused.
        #[cfg(target_os = "macos")]
        if window.is_minimized().unwrap_or(false) {
            let _ = window.unminimize();
        }
        let _ = window.set_focus();
    });
}
