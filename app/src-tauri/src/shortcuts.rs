//! Global shortcuts: ⌃⌥V (paste the last raw transcript), ⌃⌥Z (undo the
//! AI's edit of the last dictation), ⌃⌥N (show or hide Notes) and
//! each preset's own shortcut, which
//! starts a hands-free dictation with that preset (or finishes the one in
//! progress).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use splashdown_config::Preset;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::actions;
use crate::history::History;

/// The presets' shortcuts as registered now.
#[derive(Default)]
struct Registered {
    /// Shortcut id → preset id.
    presets: HashMap<u32, String>,
    shortcuts: Vec<Shortcut>,
    /// Preset id → why its shortcut couldn't be registered.
    errors: HashMap<String, String>,
}

fn registered() -> &'static Mutex<Registered> {
    static REGISTERED: OnceLock<Mutex<Registered>> = OnceLock::new();
    REGISTERED.get_or_init(Mutex::default)
}

fn lock() -> std::sync::MutexGuard<'static, Registered> {
    registered().lock().unwrap_or_else(|e| e.into_inner())
}

/// Installs the plugin, ⌃⌥V, ⌃⌥Z and ⌃⌥N. Failing to register (another app owns the
/// shortcut) only loses the shortcut, not the menu item.
pub fn install(app: &tauri::App, history: Arc<History>) {
    let paste_raw = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyV);
    let paste_id = paste_raw.id();
    let undo_ai = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyZ);
    let undo_id = undo_ai.id();
    let notes = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyN);
    let notes_id = notes.id();
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
            let preset = lock().presets.get(&shortcut.id()).cloned();
            if let (Some(preset), Some(state)) =
                (preset, app.try_state::<crate::commands::AppState>())
            {
                state.session.preset_shortcut(preset);
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
    ] {
        if let Err(e) = app.global_shortcut().register(shortcut) {
            tracing::warn!("couldn't register {name}: {e}");
        }
    }
}

/// Registers the presets' shortcuts, replacing the previous ones.
pub fn sync(app: &AppHandle, presets: &[Preset]) {
    let mut reg = lock();
    let manager = app.global_shortcut();
    for old in reg.shortcuts.drain(..) {
        let _ = manager.unregister(old);
    }
    reg.presets.clear();
    reg.errors.clear();
    for preset in presets {
        let Some(text) = preset.shortcut.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        let result = text
            .parse::<Shortcut>()
            .map_err(|e| format!("isn't a shortcut Splashdown understands ({e})"))
            .and_then(|shortcut| {
                if reg.presets.contains_key(&shortcut.id()) {
                    return Err("is already used by another preset".to_string());
                }
                manager
                    .register(shortcut)
                    .map(|()| shortcut)
                    .map_err(|_| "is already taken by Splashdown or another app".to_string())
            });
        match result {
            Ok(shortcut) => {
                reg.presets.insert(shortcut.id(), preset.id.clone());
                reg.shortcuts.push(shortcut);
            }
            Err(e) => {
                tracing::warn!("preset {} shortcut {text} {e}", preset.id);
                reg.errors.insert(preset.id.clone(), e);
            }
        }
    }
}

/// Why each preset's shortcut isn't working, by preset id (for Settings).
pub fn errors() -> HashMap<String, String> {
    lock().errors.clone()
}
