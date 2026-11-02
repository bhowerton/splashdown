//! Questions the app asks on its own, outside any window (a meeting's
//! summary, finishing one after a crash, quitting mid-meeting): a small
//! panel of ours near the top right, like the call offer. The system's own
//! alerts don't show for a menu-bar app on recent macOS.

use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const WINDOW: &str = "prompt";
/// App event with the question to show (`Prompt`).
const CHANGED: &str = "prompt-changed";

/// A question and its answers.
#[derive(Debug, Clone, Serialize)]
pub struct Prompt {
    pub title: String,
    pub text: String,
    /// The answers, the usual one first.
    pub buttons: Vec<String>,
}

type Pending = Option<(Prompt, mpsc::Sender<Option<usize>>)>;

fn pending() -> MutexGuard<'static, Pending> {
    static PENDING: Mutex<Pending> = Mutex::new(None);
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Shows a question and waits for the answer: the index of the button
/// clicked, or `None` if the panel was closed. One at a time; call it off
/// the main thread.
pub fn ask(app: &AppHandle, title: &str, text: &str, buttons: &[&str]) -> Option<usize> {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let prompt = Prompt {
        title: title.into(),
        text: text.into(),
        buttons: buttons.iter().map(|b| b.to_string()).collect(),
    };
    let (tx, rx) = mpsc::channel();
    *pending() = Some((prompt.clone(), tx));
    show(app);
    let _ = app.emit_to(WINDOW, CHANGED, &prompt);
    let answer = rx.recv().ok().flatten();
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.hide();
    }
    answer
}

/// Shows the panel near the top right, below where the call offer goes.
fn show(app: &AppHandle) {
    let window = match app.get_webview_window(WINDOW) {
        Some(window) => window,
        None => {
            let built =
                WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("prompt.html".into()))
                    .title("Splashdown")
                    .inner_size(380.0, 150.0)
                    .resizable(false)
                    .decorations(false)
                    .always_on_top(true)
                    .visible_on_all_workspaces(true)
                    .skip_taskbar(true)
                    .focused(false)
                    // Never focused, even when shown again later: a question
                    // mid-typing mustn't take the keystrokes.
                    .focusable(false)
                    .visible(false)
                    .build();
            match built {
                Ok(window) => {
                    window.on_window_event(|event| {
                        if let WindowEvent::CloseRequested { api, .. } = event {
                            api.prevent_close();
                            answer(None);
                        }
                    });
                    window
                }
                Err(e) => {
                    tracing::warn!("couldn't show a question: {e}");
                    answer(None);
                    return;
                }
            }
        }
    };
    crate::placement::place(
        &window,
        crate::placement::Spot::TopRight {
            top: 170.0,
            right: 16.0,
        },
    );
    let _ = window.show();
}

fn answer(index: Option<usize>) {
    if let Some((_, tx)) = pending().take() {
        let _ = tx.send(index);
    }
}

// ---------------------------------------------------------------- commands

/// The question on screen (for a page that just loaded).
#[tauri::command]
pub fn prompt_state() -> Option<Prompt> {
    pending().as_ref().map(|(prompt, _)| prompt.clone())
}

/// A button was clicked (`None`: closed).
#[tauri::command]
pub fn prompt_answer(index: Option<usize>) {
    answer(index);
}
