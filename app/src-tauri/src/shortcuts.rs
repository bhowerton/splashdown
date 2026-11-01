//! Global shortcuts: ⌃⌥V (paste the last raw transcript), ⌃⌥Z (undo the
//! AI's edit of the last dictation), ⌃⌥N (show or hide Notes), each
//! preset's own shortcut, which starts a hands-free dictation with that
//! preset (or finishes the one in progress), and each transform's, which
//! runs it on the selected text.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use splashdown_config::{Preset, Transform};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::actions;
use crate::history::History;

/// What a preset or transform shortcut does.
#[derive(Clone)]
enum Target {
    Preset(String),
    Transform(String),
}

/// The presets' and transforms' shortcuts as registered now.
#[derive(Default)]
struct Registered {
    /// Shortcut id → what it does.
    targets: HashMap<u32, Target>,
    shortcuts: Vec<Shortcut>,
    /// Preset id, or `transform:` and a transform id → why its shortcut
    /// couldn't be registered.
    errors: HashMap<String, String>,
}

fn registered() -> &'static Mutex<Registered> {
    static REGISTERED: OnceLock<Mutex<Registered>> = OnceLock::new();
    REGISTERED.get_or_init(Mutex::default)
}

fn lock() -> std::sync::MutexGuard<'static, Registered> {
    registered().lock().unwrap_or_else(|e| e.into_inner())
}

/// Installs the plugin, ⌃⌥V, ⌃⌥Z, ⌃⌥N and ⌃⌥M. Failing to register (another app owns the
/// shortcut) only loses the shortcut, not the menu item.
pub fn install(app: &tauri::App, history: Arc<History>) {
    let paste_raw = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyV);
    let paste_id = paste_raw.id();
    let undo_ai = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyZ);
    let undo_id = undo_ai.id();
    let notes = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyN);
    let notes_id = notes.id();
    let meeting = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyM);
    let meeting_id = meeting.id();
    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(move |app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            if shortcut.id() == paste_id {
                actions::paste_last_raw(&history);
                return;
            }
            if shortcut.id() == undo_id {
                if let Some(state) = app.try_state::<crate::commands::AppState>() {
                    state.session.undo_ai_edit();
                }
                return;
            }
            if shortcut.id() == notes_id {
                crate::notes::toggle(app);
                return;
            }
            if shortcut.id() == meeting_id {
                crate::meeting::toggle(app);
                return;
            }
            let target = lock().targets.get(&shortcut.id()).cloned();
            let Some(state) = app.try_state::<crate::commands::AppState>() else {
                return;
            };
            match target {
                Some(Target::Preset(id)) => state.session.preset_shortcut(id),
                // In Notes the transform runs on the note there.
                Some(Target::Transform(id)) if crate::notes::is_focused(app) => {
                    let _ = app.emit_to(crate::notes::WINDOW, crate::notes::RUN_TRANSFORM, id);
                }
                Some(Target::Transform(id)) => state.session.transform_shortcut(id),
                None => {}
            }
        })
        .build();
    if let Err(e) = app.handle().plugin(plugin) {
        tracing::warn!("global shortcuts unavailable: {e}");
        return;
    }
    for (shortcut, name) in [
        (paste_raw, "Ctrl+Alt+V"),
        (undo_ai, "Ctrl+Alt+Z"),
        (notes, crate::notes::ACCELERATOR),
        (meeting, crate::meeting::ACCELERATOR),
    ] {
        if let Err(e) = app.global_shortcut().register(shortcut) {
            tracing::warn!("couldn't register {name}: {e}");
        }
    }
}

/// Registers the presets' and transforms' shortcuts, replacing the
/// previous ones.
pub fn sync(app: &AppHandle, presets: &[Preset], transforms: &[Transform]) {
    let mut reg = lock();
    let manager = app.global_shortcut();
    for old in reg.shortcuts.drain(..) {
        let _ = manager.unregister(old);
    }
    reg.targets.clear();
    reg.errors.clear();
    let wanted = presets
        .iter()
        .map(|p| {
            (
                p.shortcut.as_deref(),
                p.id.clone(),
                Target::Preset(p.id.clone()),
            )
        })
        .chain(transforms.iter().map(|t| {
            (
                t.shortcut.as_deref(),
                format!("transform:{}", t.id),
                Target::Transform(t.id.clone()),
            )
        }));
    for (text, key, target) in wanted {
        let Some(text) = text.filter(|s| !s.is_empty()) else {
            continue;
        };
        let result = text
            .parse::<Shortcut>()
            .map_err(|e| format!("isn't a shortcut Splashdown understands ({e})"))
            .and_then(|shortcut| {
                if reg.targets.contains_key(&shortcut.id()) {
                    return Err("is already used by another preset or transform".to_string());
                }
                manager
                    .register(shortcut)
                    .map(|()| shortcut)
                    .map_err(|_| "is already taken by Splashdown or another app".to_string())
            });
        match result {
            Ok(shortcut) => {
                reg.targets.insert(shortcut.id(), target);
                reg.shortcuts.push(shortcut);
            }
            Err(e) => {
                tracing::warn!("{key} shortcut {text} {e}");
                reg.errors.insert(key, e);
            }
        }
    }
}

/// Why each shortcut isn't working, by preset id or `transform:` id (for
/// Settings).
pub fn errors() -> HashMap<String, String> {
    lock().errors.clone()
}
