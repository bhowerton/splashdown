//! The Ask panel: a spoken question (dictation key + A, or a spoken
//! command that's a question) answered in a small window instead of typed.
//! The answer streams in; Insert pastes it into the app the question was
//! asked from, Copy puts it on the clipboard. A question typed into the
//! panel, or asked again while it's open, follows on from the ones before.
//! It also shows what was asked of selected text that can't be changed
//! where it is (`show`).

use splashdown_core::sync::Locked;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use splashdown_core::TextModel;
use splashdown_core::ask::AskContext;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const WINDOW: &str = "ask";
/// App event with the panel's new state (`Shown`).
const CHANGED: &str = "ask-changed";

/// Earlier questions a follow-up carries along, at most.
const EARLIER: usize = 6;

/// A question to answer.
pub struct Question {
    /// As it was said.
    pub question: String,
    /// What it may be answered from.
    pub context: AskContext,
    pub model: Arc<dyn TextModel>,
    /// The app it was asked from (pid and name), for Insert.
    pub target: Option<(i32, String)>,
    /// Read the answer aloud as it's written ("read me a summary of
    /// this"); `context.spoken` should be set too.
    pub speak: bool,
}

/// An earlier question and its answer, shown above the current one.
#[derive(Debug, Clone, Serialize)]
pub struct Turn {
    pub question: String,
    pub answer: String,
}

/// What the panel shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Shown {
    /// Earlier turns of this conversation.
    pub earlier: Vec<Turn>,
    pub question: String,
    /// The answer so far.
    pub answer: String,
    /// The answer is complete (or failed).
    pub done: bool,
    pub error: Option<String>,
    /// The app Insert pastes into.
    pub app: Option<String>,
    pub notes: usize,
}

#[derive(Default)]
struct Panel {
    /// Which question is being answered; an older answer still streaming
    /// in stops showing once a new question is asked.
    asked: u64,
    shown: Shown,
    target: Option<(i32, String)>,
    /// The last question's context and model, for a typed follow-up.
    context: Option<AskContext>,
    model: Option<Arc<dyn TextModel>>,
}

fn panel() -> std::sync::MutexGuard<'static, Panel> {
    static PANEL: OnceLock<Mutex<Panel>> = OnceLock::new();
    PANEL.get_or_init(Mutex::default).locked()
}

/// Shows the panel with `q` and answers it in the background. While the
/// panel is open with an answer, `q` follows on from it.
pub fn ask(app: &AppHandle, mut q: Question) {
    let open_now = app
        .get_webview_window(WINDOW)
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    let asked = {
        let mut panel = panel();
        panel.asked += 1;
        let before = &panel.shown;
        let mut earlier = Vec::new();
        if open_now {
            // The conversation so far, and the last question if it was
            // answered.
            earlier = before.earlier.clone();
            if before.done && before.error.is_none() && !before.answer.is_empty() {
                earlier.push(Turn {
                    question: before.question.clone(),
                    answer: before.answer.clone(),
                });
            }
            let drop = earlier.len().saturating_sub(EARLIER);
            earlier.drain(..drop);
        }
        q.context.earlier = earlier
            .iter()
            .map(|t| (t.question.clone(), t.answer.clone()))
            .collect();
        panel.shown = Shown {
            earlier,
            question: q.question.clone(),
            app: q.target.as_ref().map(|(_, name)| name.clone()),
            notes: q.context.notes.len(),
            ..Shown::default()
        };
        panel.target = q.target.clone();
        panel.context = Some(q.context.clone());
        panel.model = Some(q.model.clone());
        panel.asked
    };
    let request = splashdown_core::ask::request(&q.question, &q.context);
    open(app);
    changed(app);
    // A new question ends the reading of an earlier answer.
    let reader = if q.speak {
        Some(crate::reading::start("Reading the answer"))
    } else {
        crate::reading::stop();
        None
    };

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // How much of the answer has gone to be read: whole sentences, as
        // they're written.
        let mut read_to = 0;
        let mut on_text = |so_far: &str| {
            if let Some(reader) = &reader
                && let Some(rest) = so_far.get(read_to..)
                && let Some(end) = splashdown_core::aloud::sentence_end(rest)
            {
                reader.push(&rest[..end]);
                read_to += end;
            }
            let mut panel = panel();
            if panel.asked == asked {
                panel.shown.answer = so_far.to_string();
                drop(panel);
                changed(&app);
            }
        };
        let result = q.model.complete_streaming(&request, &mut on_text).await;
        if let Some(reader) = reader {
            match &result {
                Ok(completion) => {
                    if let Some(rest) = completion.text.get(read_to..) {
                        reader.push(rest);
                    }
                    reader.close();
                }
                Err(_) => reader.cancel(),
            }
        }
        let usage = match &result {
            Ok(completion) => Some(completion.usage),
            Err(splashdown_core::ProviderError::Interrupted { usage, .. }) => Some(*usage),
            Err(_) => None,
        };
        if let Some(usage) = usage {
            crate::spend::shared().record(&[(q.model.name(), usage)], chrono::Local::now());
        }
        {
            let mut panel = panel();
            if panel.asked != asked {
                return;
            }
            panel.shown.done = true;
            match result {
                Ok(completion) => panel.shown.answer = completion.text.trim().to_string(),
                Err(e) => {
                    tracing::warn!("ask failed: {e}");
                    panel.shown.error = Some(crate::errors::pipeline(
                        &splashdown_core::PipelineError::Llm(e),
                        None,
                        Some(&q.model.name()),
                    ));
                }
            }
        }
        changed(&app);
    });
}

/// A result that's already written, for the panel to show as the answer
/// to what was asked.
pub struct Written {
    /// What was asked for, as it was said.
    pub asked: String,
    pub text: String,
    /// What a typed follow-up may be answered from.
    pub context: AskContext,
    /// For a typed follow-up; without one, there's none.
    pub model: Option<Arc<dyn TextModel>>,
    /// The app it was asked from (pid and name), for Insert.
    pub target: Option<(i32, String)>,
}

/// Shows the panel with `w` as a finished answer: a new version of text
/// that couldn't be replaced where it was selected (a message received, a
/// page). Copy, Insert and Read aloud work on it as on any answer, and a
/// typed follow-up carries on from it.
pub fn show(app: &AppHandle, w: Written) {
    {
        let mut panel = panel();
        // An earlier answer still streaming in stops showing.
        panel.asked += 1;
        panel.shown = Shown {
            question: w.asked,
            answer: w.text.trim().to_string(),
            done: true,
            app: w.target.as_ref().map(|(_, name)| name.clone()),
            ..Shown::default()
        };
        panel.target = w.target;
        panel.context = Some(w.context);
        panel.model = w.model;
    }
    crate::reading::stop();
    open(app);
    changed(app);
}

fn changed(app: &AppHandle) {
    let shown = panel().shown.clone();
    let _ = app.emit_to(WINDOW, CHANGED, shown);
}

/// Shows the panel, creating it the first time.
fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let result = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("ask.html".into()))
        .title("Ask")
        .inner_size(460.0, 300.0)
        .min_inner_size(320.0, 180.0)
        .resizable(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .center()
        .focused(true)
        .build();
    match result {
        Ok(window) => {
            // Closing only hides it, ready for the next question.
            let handle = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = handle.hide();
                }
            });
        }
        Err(e) => tracing::warn!("couldn't open the Ask panel: {e}"),
    }
}

/// Hides the panel (`ask_close` also gives focus back to the app the
/// question came from).
fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.hide();
    }
}

// ---------------------------------------------------------------- commands

/// What the panel shows now (for a page that just loaded).
#[tauri::command]
pub fn ask_state() -> Shown {
    panel().shown.clone()
}

/// Insert: pastes the answer into the app the question was asked from.
/// On failure it's left on the clipboard and the panel stays.
#[tauri::command]
pub async fn ask_insert(text: String, app: AppHandle) -> Result<(), String> {
    let target = panel().target.clone();
    let Some((pid, name)) = target else {
        return Err("No app to insert into. Copy the answer instead.".into());
    };
    hide(&app);
    let pasted = tauri::async_runtime::spawn_blocking(move || {
        crate::notes::paste_into(pid, &text).inspect_err(|_| {
            let _ = splashdown_platform::copy_to_clipboard(&text);
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Err(e) = pasted {
        tracing::warn!("ask insert failed: {e}");
        open(&app);
        return Err(format!(
            "Couldn't paste into {name}. It's on the clipboard."
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn ask_copy(text: String) -> Result<(), String> {
    splashdown_platform::copy_to_clipboard(&text).map_err(|e| e.to_string())
}

/// A question typed into the panel: follows on from the ones before, with
/// the same selection and context, and notes that match it.
#[tauri::command]
pub fn ask_followup(question: String, app: AppHandle) -> Result<(), String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Ok(());
    }
    let (context, model, target) = {
        let panel = panel();
        (
            panel.context.clone(),
            panel.model.clone(),
            panel.target.clone(),
        )
    };
    let (Some(mut context), Some(model)) = (context, model) else {
        return Err("Ask a question with the dictation key and A first.".into());
    };
    let context_spoken = context.spoken;
    context.notes = app
        .try_state::<crate::state::AppState>()
        .map(|s| crate::notes::excerpts_for(&s.settings().notes, &question))
        .unwrap_or_default();
    ask(
        &app,
        Question {
            question,
            context,
            model,
            target,
            // Read too, when the conversation's answers are being read.
            speak: context_spoken,
        },
    );
    Ok(())
}

/// Esc or Close: hides the panel and gives focus back.
#[tauri::command]
pub fn ask_close(app: AppHandle) {
    hide(&app);
    if let Some((pid, _)) = panel().target.clone() {
        let _ = splashdown_platform::activate_app(pid);
    }
}
