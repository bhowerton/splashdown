//! The user's writing style per kind of app (`splashdown_core::style`):
//! the drafts Compose wrote and what was sent instead, kept on this
//! computer in `style.json` (the last few per kind), and the profile the AI
//! writes from them, which Settings shows and the user can edit. Compose
//! follows the profile for the app it's writing into.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use splashdown_core::style::{self, Edit, Kind};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

const FILE: &str = "style.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    kinds: HashMap<String, KindStyle>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct KindStyle {
    /// What the AI wrote, or the user changed it to.
    #[serde(default)]
    profile: String,
    #[serde(default)]
    edits: Vec<StoredEdit>,
    /// Edits since the profile was last written.
    #[serde(default)]
    new: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEdit {
    draft: String,
    sent: String,
    /// Milliseconds since the Unix epoch.
    at: u64,
}

static STORED: Mutex<Option<Stored>> = Mutex::new(None);

fn path() -> Option<PathBuf> {
    crate::engine::paths().ok().map(|p| p.config_dir.join(FILE))
}

/// Runs `f` on the stored styles (read from disk the first time), and
/// saves them if it says so.
fn with_stored<T>(f: impl FnOnce(&mut Stored) -> (T, bool)) -> T {
    let mut slot = STORED.lock().unwrap_or_else(|e| e.into_inner());
    let stored = slot.get_or_insert_with(|| {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    });
    let (out, changed) = f(stored);
    if changed
        && let Some(path) = path()
        && let Ok(text) = serde_json::to_string_pretty(stored)
        && let Err(e) = std::fs::write(&path, text)
    {
        tracing::warn!("couldn't save the writing style: {e}");
    }
    out
}

/// The profile Compose follows in `kind` of app, if there is one.
pub fn profile(kind: Kind) -> Option<String> {
    with_stored(|s| {
        let profile = s
            .kinds
            .get(kind.key())
            .map(|k| k.profile.trim().to_string())
            .filter(|p| !p.is_empty());
        (profile, false)
    })
}

/// A draft Compose wrote into `kind` of app, and what was sent: kept if it
/// says something about how the user writes, and the profile is written
/// again once there are a few new ones.
pub fn record(app: &AppHandle, kind: Kind, draft: &str, sent: &str) {
    if !style::worth_keeping(draft, sent) {
        return;
    }
    let due = with_stored(|s| {
        let k = s.kinds.entry(kind.key().to_string()).or_default();
        k.edits.push(StoredEdit {
            draft: draft.trim().to_string(),
            sent: sent.trim().to_string(),
            at: crate::history::Entry::now_ms(),
        });
        let over = k.edits.len().saturating_sub(style::KEEP_EDITS);
        k.edits.drain(..over);
        k.new += 1;
        (k.new >= style::UPDATE_AFTER, true)
    });
    tracing::info!("kept an edited draft for the {} style", kind.key());
    if due {
        let app = app.clone();
        std::thread::spawn(move || update(&app, kind));
    }
}

/// Writes `kind`'s profile again from its edits, with the AI that answers
/// questions (it runs off the session thread, so this one waits for it).
fn update(app: &AppHandle, kind: Kind) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let model = match state.session.answer_model() {
        Ok(Some(m)) => m.model,
        Ok(None) => return,
        Err(e) => return tracing::warn!("no model for the writing style: {e}"),
    };
    let (current, edits) = with_stored(|s| {
        let k = s.kinds.get(kind.key()).cloned().unwrap_or_default();
        let edits: Vec<Edit> = k
            .edits
            .iter()
            .map(|e| Edit {
                draft: e.draft.clone(),
                sent: e.sent.clone(),
            })
            .collect();
        ((k.profile, edits), false)
    });
    let request = style::profile_request(kind, &current, &edits);
    match tauri::async_runtime::block_on(model.complete(&request)) {
        Ok(done) => {
            crate::spend::shared().record(&[(model.name(), done.usage)], chrono::Local::now());
            let profile = style::clean_profile(&done.text);
            if profile.is_empty() {
                return;
            }
            with_stored(|s| {
                let k = s.kinds.entry(kind.key().to_string()).or_default();
                k.profile = profile;
                k.new = 0;
                ((), true)
            });
            tracing::info!("wrote the {} style again", kind.key());
            crate::state::settings_changed_elsewhere(app);
        }
        Err(e) => tracing::warn!("couldn't write the writing style: {e}"),
    }
}

// ---------------------------------------------------------------- commands

/// One kind of app's style, as Settings shows it.
#[derive(Serialize)]
pub struct StyleView {
    kind: &'static str,
    label: &'static str,
    profile: String,
    /// How many edited drafts it was learned from.
    edits: usize,
}

#[tauri::command]
pub fn style_profiles() -> Vec<StyleView> {
    with_stored(|s| {
        let views = Kind::ALL
            .iter()
            .map(|kind| {
                let k = s.kinds.get(kind.key());
                StyleView {
                    kind: kind.key(),
                    label: kind.label(),
                    profile: k.map(|k| k.profile.clone()).unwrap_or_default(),
                    edits: k.map_or(0, |k| k.edits.len()),
                }
            })
            .collect();
        (views, false)
    })
}

/// The user's own wording for a kind's profile; the next update starts
/// from it.
#[tauri::command]
pub fn style_set_profile(kind: String, profile: String) -> Result<(), String> {
    let kind = Kind::from_key(&kind).ok_or("Unknown kind of app")?;
    with_stored(|s| {
        s.kinds.entry(kind.key().to_string()).or_default().profile = profile.trim().to_string();
        ((), true)
    });
    Ok(())
}

/// Forgets every style and every kept draft.
#[tauri::command]
pub fn style_forget() {
    with_stored(|s| {
        *s = Stored::default();
        ((), true)
    });
    tracing::info!("forgot the writing style");
}
