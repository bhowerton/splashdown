//! The Compose panel. Compose (the user says what they want to write after
//! the instruction key: "draft a reply…") writes the draft straight into
//! the field by default (`show_draft` in session/assist.rs); this panel is used when
//! Settings → Words & language → Check drafts in a panel first is on, text is
//! selected, or there's no field. The draft is streamed into a small
//! window as it's written. A draft can also be dictated (the draft
//! letter): its first version is what was said, cleaned up as any
//! dictation is. While the panel is open, a dictation goes to the draft
//! instead of being typed: a plain one adds its words, one with the
//! instruction key tapped changes it ("shorter", "add that I'm free
//! Thursday"), and so do the quick changes and a typed change. A change
//! that only replaces words is made exactly. Every version is kept, to go
//! back to. Insert pastes the draft into the app it's for; nothing is ever
//! sent.

use splashdown_core::sync::Locked;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use splashdown_core::TextModel;
use splashdown_core::compose::{self, Changed, ComposeContext};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const WINDOW: &str = "compose";
/// App event with the panel's new state (`Shown`).
const CHANGED: &str = "compose-changed";

/// A draft to write.
pub struct Draft {
    /// What the user said they want to write.
    pub asked: String,
    pub context: ComposeContext,
    pub model: Arc<dyn TextModel>,
    /// The app the draft is for (pid and name), for Insert.
    pub target: Option<(i32, String)>,
}

/// A draft the user dictated (the draft letter): it's shown as said, and
/// there's no request behind it.
pub struct Dictated {
    /// What was said, cleaned up as any dictation.
    pub text: String,
    pub context: ComposeContext,
    /// For changes; with none, words can still be added.
    pub model: Option<Arc<dyn TextModel>>,
    /// The app the draft is for (pid and name), for Insert.
    pub target: Option<(i32, String)>,
}

/// What made a version, for the panel to say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Made {
    /// Written from what the user asked for (`said`).
    #[default]
    Asked,
    /// Dictated as it is.
    Dictated,
    /// Words added by a dictation (`said`).
    Added,
    /// Rewritten as asked (`said`, the change).
    Changed,
    /// Words replaced exactly (`said`, what changed: "Changed 2 · Tuesday
    /// → Thursday").
    Replaced,
}

/// One version of the draft: what made it, and the text.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Version {
    pub said: String,
    pub text: String,
    pub made: Made,
}

/// What the panel shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Shown {
    pub versions: Vec<Version>,
    /// Which version is shown.
    pub at: usize,
    /// A version is being written.
    pub writing: bool,
    pub error: Option<String>,
    /// The app Insert pastes into.
    pub app: Option<String>,
    /// What it's replying to came from a selection.
    pub replying: bool,
    /// The conversation around the field was read from the window.
    pub read_window: bool,
    /// Instructions to an AI were left out of the material.
    pub removed: bool,
}

#[derive(Default)]
struct Panel {
    /// Which version is being written; an older one still streaming in
    /// stops showing once another starts.
    writing: u64,
    shown: Shown,
    /// What the draft was asked for; `None` for a dictated one.
    asked: Option<String>,
    target: Option<(i32, String)>,
    context: Option<ComposeContext>,
    model: Option<Arc<dyn TextModel>>,
}

fn panel() -> std::sync::MutexGuard<'static, Panel> {
    static PANEL: OnceLock<Mutex<Panel>> = OnceLock::new();
    PANEL.get_or_init(Mutex::default).locked()
}

/// Whether the panel is on screen (a dictation then changes the draft).
pub fn is_open(app: &AppHandle) -> bool {
    app.get_webview_window(WINDOW)
        .is_some_and(|w| w.is_visible().unwrap_or(false))
}

/// Opens the panel and writes a first draft of `draft.asked`.
pub fn start(app: &AppHandle, draft: Draft) {
    let (context, removed) = draft.context.defanged();
    let request = compose::request(&draft.asked, &context);
    {
        let mut panel = panel();
        panel.shown = Shown {
            versions: vec![Version {
                said: draft.asked.clone(),
                text: String::new(),
                made: Made::Asked,
            }],
            app: draft.target.as_ref().map(|(_, name)| name.clone()),
            replying: context.replying_to.is_some(),
            read_window: context.window_text.is_some(),
            removed,
            ..Shown::default()
        };
        panel.asked = Some(draft.asked);
        panel.target = draft.target;
        panel.context = Some(context);
        panel.model = Some(draft.model);
    }
    open(app);
    write(app, request, None);
}

/// Opens the panel on a dictated draft, as it was said.
pub fn start_dictated(app: &AppHandle, draft: Dictated) {
    {
        let mut panel = panel();
        // A version still being written for the last draft stops showing.
        panel.writing += 1;
        panel.shown = Shown {
            versions: vec![Version {
                said: String::new(),
                text: draft.text,
                made: Made::Dictated,
            }],
            app: draft.target.as_ref().map(|(_, name)| name.clone()),
            ..Shown::default()
        };
        panel.asked = None;
        panel.target = draft.target;
        panel.context = Some(draft.context);
        panel.model = draft.model;
    }
    open(app);
    changed(app);
}

/// Adds dictated words to the draft shown: a new version.
pub fn add(app: &AppHandle, words: &str) -> Result<(), String> {
    let words = words.trim();
    if words.is_empty() {
        return Ok(());
    }
    {
        let mut panel = panel();
        if panel.shown.writing {
            return Err("Wait for this version to finish first.".into());
        }
        let draft = panel
            .shown
            .versions
            .get(panel.shown.at)
            .map(|v| v.text.clone())
            .unwrap_or_default();
        panel.shown.versions.push(Version {
            said: words.to_string(),
            text: compose::add(&draft, words),
            made: Made::Added,
        });
        panel.shown.at = panel.shown.versions.len() - 1;
        panel.shown.error = None;
    }
    open(app);
    changed(app);
    Ok(())
}

/// Changes the draft shown as asked ("shorter"): a new version, written
/// from it, or made exactly when the change only replaces words. Ignored
/// while a version is still being written.
pub fn change(app: &AppHandle, change: &str) -> Result<(), String> {
    change_as(app, change, true)
}

/// As `change`. `may_replace`: the change might only replace words, so
/// that's checked for beside the rewrite. The panel's own quick changes
/// ("Make it shorter.") never do, and skip the check and its request.
fn change_as(app: &AppHandle, change: &str, may_replace: bool) -> Result<(), String> {
    let change = change.trim();
    if change.is_empty() {
        return Ok(());
    }
    let (request, draft) = {
        let mut panel = panel();
        if panel.shown.writing {
            return Err("Wait for this version to finish first.".into());
        }
        if panel.model.is_none() {
            return Err("Changes need an AI model: choose one in Settings → AI Models".into());
        }
        let (Some(context), Some(draft)) = (
            panel.context.clone(),
            panel
                .shown
                .versions
                .get(panel.shown.at)
                .map(|v| v.text.clone()),
        ) else {
            return Err("Say what to write first: tap the instruction key while dictating.".into());
        };
        // A dictated draft has no request behind it to write from.
        let request = match &panel.asked {
            Some(asked) => compose::revise(asked, &draft, change, &context),
            None => compose::revise_dictated(&draft, change, &context),
        };
        panel.shown.versions.push(Version {
            said: change.to_string(),
            text: String::new(),
            made: Made::Changed,
        });
        panel.shown.at = panel.shown.versions.len() - 1;
        (request, draft)
    };
    open(app);
    write(
        app,
        request,
        may_replace.then(|| (draft, change.to_string())),
    );
    Ok(())
}

/// Writes the newest version from `request`, streaming it into the panel.
/// For a change, `change` is the draft it starts from and what was asked:
/// a change that only replaces words is then made exactly instead.
fn write(app: &AppHandle, request: splashdown_core::TextRequest, change: Option<(String, String)>) {
    let (writing, model, context) = {
        let mut panel = panel();
        panel.writing += 1;
        panel.shown.writing = true;
        panel.shown.error = None;
        (
            panel.writing,
            panel.model.clone(),
            panel.context.clone().unwrap_or_default(),
        )
    };
    changed(app);
    let Some(model) = model else {
        panel().shown.writing = false;
        changed(app);
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut on_text = |so_far: &str| {
            let mut panel = panel();
            if panel.writing == writing
                && let Some(last) = panel.shown.versions.last_mut()
            {
                last.text = compose::clean(so_far);
                drop(panel);
                changed(&app);
            }
        };
        let result = match &change {
            Some((draft, said)) => {
                let (result, checked) = compose::change(
                    &*model,
                    draft,
                    said,
                    &request,
                    context.app_name.as_deref(),
                    &mut on_text,
                )
                .await;
                crate::spend::shared().record(&[(model.name(), checked)], chrono::Local::now());
                match result {
                    Changed::Replaced { text, replaced } => {
                        {
                            let mut panel = panel();
                            if panel.writing != writing {
                                return;
                            }
                            panel.shown.writing = false;
                            if let Some(last) = panel.shown.versions.last_mut() {
                                last.text = text;
                                last.said = replaced.summary();
                                last.made = Made::Replaced;
                            }
                        }
                        tracing::info!(
                            "a change to the draft made {} replacement(s)",
                            replaced.total()
                        );
                        changed(&app);
                        return;
                    }
                    Changed::Rewritten(result) => result,
                }
            }
            None => model.complete_streaming(&request, &mut on_text).await,
        };
        let usage = match &result {
            Ok(completion) => Some(completion.usage),
            Err(splashdown_core::ProviderError::Interrupted { usage, .. }) => Some(*usage),
            Err(_) => None,
        };
        if let Some(usage) = usage {
            crate::spend::shared().record(&[(model.name(), usage)], chrono::Local::now());
        }
        {
            let mut panel = panel();
            if panel.writing != writing {
                return;
            }
            panel.shown.writing = false;
            match result {
                Ok(completion) => {
                    if let Some(last) = panel.shown.versions.last_mut() {
                        last.text = compose::finish(&completion.text, &context);
                    }
                    crate::did::note(crate::did::COMPOSED, "");
                }
                Err(e) => {
                    tracing::warn!("compose failed: {e}");
                    panel.shown.error = Some(crate::errors::pipeline(
                        &splashdown_core::PipelineError::Llm(e),
                        None,
                        Some(&model.name()),
                    ));
                    // A change that failed leaves the draft it started from.
                    if panel.shown.versions.len() > 1 {
                        panel.shown.versions.pop();
                        panel.shown.at = panel.shown.versions.len() - 1;
                    }
                }
            }
        }
        changed(&app);
    });
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
    let result = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("compose.html".into()))
        .title("Compose")
        .inner_size(600.0, 420.0)
        .min_inner_size(360.0, 260.0)
        .resizable(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .center()
        .focused(true)
        .build();
    match result {
        Ok(window) => {
            // Closing only hides it, ready for the next draft.
            let handle = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = handle.hide();
                }
            });
        }
        Err(e) => tracing::warn!("couldn't open the Compose panel: {e}"),
    }
}

fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.hide();
    }
}

// ---------------------------------------------------------------- commands

/// What the panel shows now (for a page that just loaded).
#[tauri::command]
pub fn compose_state() -> Shown {
    panel().shown.clone()
}

/// A quick change ("Shorter") or a typed one.
#[tauri::command]
pub fn compose_change(change: String, quick: Option<bool>, app: AppHandle) -> Result<(), String> {
    change_as(&app, &change, !quick.unwrap_or(false))
}

/// ← and →: shows another version.
#[tauri::command]
pub fn compose_version(at: usize, app: AppHandle) {
    {
        let mut panel = panel();
        if panel.shown.writing || at >= panel.shown.versions.len() {
            return;
        }
        panel.shown.at = at;
    }
    changed(&app);
}

/// Insert: pastes the version shown into the app it's for. On failure
/// it's left on the clipboard and the panel stays.
#[tauri::command]
pub async fn compose_insert(text: String, app: AppHandle) -> Result<(), String> {
    let target = panel().target.clone();
    let Some((pid, name)) = target else {
        return Err("No app to insert into. Copy the draft instead.".into());
    };
    hide(&app);
    crate::reading::stop();
    let pasted = tauri::async_runtime::spawn_blocking(move || {
        crate::notes::paste_into(pid, &text).inspect_err(|_| {
            let _ = splashdown_platform::copy_to_clipboard(&text);
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Err(e) = pasted {
        tracing::warn!("compose insert didn't go in: {e:?}");
        open(&app);
        return Err(e.message(&name));
    }
    Ok(())
}

#[tauri::command]
pub fn compose_copy(text: String) -> Result<(), String> {
    splashdown_platform::copy_to_clipboard(&text).map_err(|e| e.to_string())
}

/// Esc or Close: hides the panel and gives focus back.
#[tauri::command]
pub fn compose_close(app: AppHandle) {
    hide(&app);
    crate::reading::stop();
    if let Some((pid, _)) = panel().target.clone() {
        let _ = splashdown_platform::activate_app(pid);
    }
}
