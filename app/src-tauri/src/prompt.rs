//! Questions the app asks on its own, outside any window (a meeting's
//! summary, finishing one after a crash, quitting mid-meeting), and
//! `choose`, a grid of choices (the "Translate into…" languages): a small
//! panel of ours near the top right, below the call offer. The system's own
//! alerts don't show for a menu-bar app on recent macOS.

use splashdown_core::sync::Locked;
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const WINDOW: &str = "prompt";
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 150.0;
/// With a grid of choices (three rows of three).
const TALL_HEIGHT: f64 = 210.0;
/// App event with the question to show (`Prompt`).
const CHANGED: &str = "prompt-changed";

/// A question and its answers.
#[derive(Debug, Clone, Serialize)]
pub struct Prompt {
    pub title: String,
    pub text: String,
    /// The answers, the usual one first.
    pub buttons: Vec<String>,
    /// Options to pick from, shown as a grid above the buttons (a language
    /// to translate into). Their answers come first: choice `i` is `i`,
    /// and button `j` is `choices.len() + j`.
    pub choices: Vec<String>,
}

type Pending = Option<(Prompt, mpsc::Sender<Option<usize>>)>;

fn pending() -> MutexGuard<'static, Pending> {
    static PENDING: Mutex<Pending> = Mutex::new(None);
    PENDING.locked()
}

/// Shows a question and waits for the answer: the index of the button
/// clicked, or `None` if the panel was closed. One at a time; call it off
/// the main thread.
pub fn ask(app: &AppHandle, title: &str, text: &str, buttons: &[&str]) -> Option<usize> {
    let _turn = one_at_a_time().locked();
    let prompt = Prompt {
        title: title.into(),
        text: text.into(),
        buttons: buttons.iter().map(|b| b.to_string()).collect(),
        choices: Vec::new(),
    };
    put(app, prompt, None)
}

/// Shows `choices` to pick one from, with `buttons` below, for up to
/// `timeout`: the answer as for `Prompt::choices`, or `None` if it was
/// closed or ran out of time. `Err(())` when another question is on screen,
/// so the caller can do without rather than wait behind it.
pub fn choose(
    app: &AppHandle,
    title: &str,
    choices: &[&str],
    buttons: &[&str],
    timeout: Duration,
) -> Result<Option<usize>, ()> {
    let _turn = match one_at_a_time().try_lock() {
        Ok(turn) => turn,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Err(()),
    };
    let prompt = Prompt {
        title: title.into(),
        text: String::new(),
        buttons: buttons.iter().map(|b| b.to_string()).collect(),
        choices: choices.iter().map(|c| c.to_string()).collect(),
    };
    Ok(put(app, prompt, Some(timeout)))
}

fn one_at_a_time() -> &'static Mutex<()> {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    &ONE_AT_A_TIME
}

/// Shows `prompt` and waits for its answer (at most `timeout`).
fn put(app: &AppHandle, prompt: Prompt, timeout: Option<Duration>) -> Option<usize> {
    let (tx, rx) = mpsc::channel();
    let tall = !prompt.choices.is_empty();
    *pending() = Some((prompt.clone(), tx));
    show(app, tall);
    let _ = app.emit_to(WINDOW, CHANGED, &prompt);
    let answer = match timeout {
        Some(timeout) => rx.recv_timeout(timeout).ok().flatten(),
        None => rx.recv().ok().flatten(),
    };
    // Ran out of time: it's no longer waiting.
    pending().take();
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.hide();
    }
    answer
}

/// Shows the panel near the top right, below where the call offer goes:
/// `tall` for one with choices.
fn show(app: &AppHandle, tall: bool) {
    let window = match app.get_webview_window(WINDOW) {
        Some(window) => window,
        None => {
            let built =
                WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("prompt.html".into()))
                    .title("Splashdown")
                    .inner_size(WIDTH, HEIGHT)
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
    let height = if tall { TALL_HEIGHT } else { HEIGHT };
    let _ = window.set_size(tauri::LogicalSize::new(WIDTH, height));
    crate::placement::place(
        &window,
        crate::placement::Spot::TopRight {
            top: crate::calls::BELOW,
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
