//! The scratchpad (spec §9d): a small floating notepad for dictating when
//! there's no text field. One current note plus the last `KEEP_NOTES`
//! earlier ones, in `scratchpad.json` next to the settings. Never sent
//! anywhere.
//!
//! ⌃⌥N or the menu bar opens it and remembers the app that was in front,
//! so "Send to {app}" can hand focus back and paste the note there.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use splashdown_platform::InsertMethod;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::history::Entry;

pub const WINDOW: &str = "scratchpad";
/// Shown in the menu; registered in `shortcuts.rs`.
pub const ACCELERATOR: &str = "Ctrl+Alt+N";
/// Earlier notes to keep.
const KEEP_NOTES: usize = 20;
/// App event telling the page it was shown again, with the app "Send to"
/// would paste into (or null).
const SHOWN: &str = "scratchpad-shown";
/// How long to wait for the other app to come to the front before pasting.
const ACTIVATE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Once it's in front, a moment for its window to take keyboard focus.
const SETTLE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub text: String,
    /// When it was last changed (Unix ms). Also identifies an earlier note.
    pub at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notes {
    pub current: Note,
    /// Newest first, at most `KEEP_NOTES`.
    pub earlier: Vec<Note>,
}

pub struct Scratchpad {
    path: PathBuf,
    notes: Mutex<Notes>,
}

/// The app's scratchpad (one per user).
pub fn shared() -> &'static Scratchpad {
    static SCRATCHPAD: OnceLock<Scratchpad> = OnceLock::new();
    SCRATCHPAD.get_or_init(|| {
        Scratchpad::open(
            splashdown_config::Paths::resolve()
                .map(|p| p.config_dir.join("scratchpad.json"))
                .unwrap_or_else(|| std::env::temp_dir().join("splashdown-scratchpad.json")),
        )
    })
}

impl Scratchpad {
    pub fn open(path: PathBuf) -> Self {
        let notes = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            notes: Mutex::new(notes),
        }
    }

    fn notes(&self) -> std::sync::MutexGuard<'_, Notes> {
        self.notes.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self) -> Notes {
        self.notes().clone()
    }

    /// Replaces the current note's text (the page saves as you type).
    pub fn set_current(&self, text: String, now: u64) {
        let mut notes = self.notes();
        if notes.current.text == text {
            return;
        }
        notes.current = Note { text, at: now };
        self.save(&notes);
    }

    /// Moves the current note to the top of the earlier ones and starts an
    /// empty one. A blank note is dropped instead.
    pub fn new_note(&self, now: u64) -> Notes {
        let mut notes = self.notes();
        let current = std::mem::take(&mut notes.current);
        file_away(&mut notes, current, now);
        self.save(&notes);
        notes.clone()
    }

    /// Opens an earlier note in the editor; the current one (if it has
    /// text) takes its place in the list.
    pub fn open_note(&self, at: u64, now: u64) -> Notes {
        let mut notes = self.notes();
        if let Some(i) = notes.earlier.iter().position(|n| n.at == at) {
            let opened = notes.earlier.remove(i);
            let current = std::mem::replace(&mut notes.current, opened);
            file_away(&mut notes, current, now);
            self.save(&notes);
        }
        notes.clone()
    }

    pub fn delete_note(&self, at: u64) -> Notes {
        let mut notes = self.notes();
        let before = notes.earlier.len();
        notes.earlier.retain(|n| n.at != at);
        if notes.earlier.len() != before {
            self.save(&notes);
        }
        notes.clone()
    }

    /// Writes to a temporary file first, so a crash mid-write can't lose
    /// the notes.
    fn save(&self, notes: &Notes) {
        let json = serde_json::to_string_pretty(notes).expect("scratchpad serialize");
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("json.tmp");
        let result = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(e) = result {
            tracing::warn!("couldn't save the scratchpad: {e}");
        }
    }
}

/// Puts `note` at the top of the earlier notes (unless it's blank) with an
/// `at` no other note has, and drops the oldest past `KEEP_NOTES`.
fn file_away(notes: &mut Notes, mut note: Note, now: u64) {
    if note.text.trim().is_empty() {
        return;
    }
    if note.at == 0 {
        note.at = now;
    }
    while notes.earlier.iter().any(|n| n.at == note.at) {
        note.at += 1;
    }
    notes.earlier.insert(0, note);
    notes.earlier.truncate(KEEP_NOTES);
}

// ---------------------------------------------------------------- window

/// The app that was in front when the scratchpad opened: (pid, name).
fn target() -> &'static Mutex<Option<(i32, String)>> {
    static TARGET: OnceLock<Mutex<Option<(i32, String)>>> = OnceLock::new();
    TARGET.get_or_init(Mutex::default)
}

fn target_lock() -> std::sync::MutexGuard<'static, Option<(i32, String)>> {
    target().lock().unwrap_or_else(|e| e.into_inner())
}

/// Remembers the app in front, unless it's us (Settings, or the
/// scratchpad itself): then the previous one stays.
fn remember_front_app() {
    if let Some((pid, name)) = splashdown_platform::frontmost_app()
        && u32::try_from(pid).ok() != Some(std::process::id())
    {
        *target_lock() = Some((pid, name));
    }
}

fn target_name() -> Option<String> {
    target_lock()
        .as_ref()
        .map(|(_, name)| name.clone())
        .filter(|n| !n.is_empty())
}

/// ⌃⌥N: hides the scratchpad if it has focus, otherwise opens it.
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

/// Shows the scratchpad (creating it the first time) with its text area
/// focused.
pub fn open(app: &AppHandle) {
    remember_front_app();
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to(WINDOW, SHOWN, target_name());
        return;
    }
    let result = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("scratchpad.html".into()))
        .title("Scratchpad")
        .inner_size(420.0, 320.0)
        .min_inner_size(320.0, 220.0)
        .resizable(true)
        .always_on_top(true)
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
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = handle.hide();
                }
            });
        }
        Err(e) => tracing::error!("failed to open the scratchpad: {e}"),
    }
}

/// Hides the scratchpad, brings back the app that was in front before it
/// opened, and pastes `text` there. On failure the note is left on the
/// clipboard and the scratchpad comes back.
fn send(app: &AppHandle, text: &str) -> Result<(), String> {
    let Some((pid, name)) = target_lock().clone() else {
        return Err("No app to send to. Copy the note instead.".into());
    };
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.hide();
    }
    let result = splashdown_platform::activate_app(pid)
        .map_err(|e| e.to_string())
        .and_then(|()| wait_for_front(pid))
        .and_then(|()| {
            std::thread::sleep(SETTLE);
            splashdown_platform::insert_text(text, InsertMethod::Paste).map_err(|e| e.to_string())
        });
    if let Err(e) = result {
        tracing::warn!("scratchpad send failed: {e}");
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

fn wait_for_front(pid: i32) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < ACTIVATE_TIMEOUT {
        if splashdown_platform::frontmost_app().is_some_and(|(p, _)| p == pid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err("the app didn't come to the front".into())
}

// ---------------------------------------------------------------- commands

/// What the page shows.
#[derive(Serialize)]
pub struct View {
    notes: Notes,
    /// The app "Send to" pastes into.
    target: Option<String>,
}

#[tauri::command]
pub fn scratchpad_load() -> View {
    View {
        notes: shared().get(),
        target: target_name(),
    }
}

#[tauri::command]
pub fn scratchpad_save(text: String) {
    shared().set_current(text, Entry::now_ms());
}

#[tauri::command]
pub fn scratchpad_new(text: String) -> Notes {
    let pad = shared();
    pad.set_current(text, Entry::now_ms());
    pad.new_note(Entry::now_ms())
}

#[tauri::command]
pub fn scratchpad_open_note(at: u64, text: String) -> Notes {
    let pad = shared();
    pad.set_current(text, Entry::now_ms());
    pad.open_note(at, Entry::now_ms())
}

#[tauri::command]
pub fn scratchpad_delete_note(at: u64) -> Notes {
    shared().delete_note(at)
}

/// Pastes can block for a few hundred ms, so this runs off the main thread.
#[tauri::command]
pub async fn scratchpad_send(text: String, app: AppHandle) -> Result<(), String> {
    shared().set_current(text.clone(), Entry::now_ms());
    tauri::async_runtime::spawn_blocking(move || send(&app, &text))
        .await
        .map_err(|e| e.to_string())?
}

/// From Settings → Privacy.
#[tauri::command]
pub fn open_scratchpad(app: AppHandle) {
    open(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "splashdown-scratchpad-{name}-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn saves_and_loads_the_current_note() {
        let path = temp("load");
        let pad = Scratchpad::open(path.clone());
        assert_eq!(pad.get(), Notes::default());
        pad.set_current("Buy milk".into(), 10);

        let again = Scratchpad::open(path.clone());
        assert_eq!(
            again.get().current,
            Note {
                text: "Buy milk".into(),
                at: 10
            }
        );
        assert!(again.get().earlier.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_or_broken_file_is_an_empty_scratchpad() {
        let path = temp("broken");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Scratchpad::open(path.clone()).get(), Notes::default());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn new_note_files_the_current_one_and_keeps_twenty() {
        let path = temp("cap");
        let pad = Scratchpad::open(path.clone());
        for i in 0..25u64 {
            pad.set_current(format!("note {i}"), 100 + i);
            pad.new_note(1000 + i);
        }
        let notes = pad.get();
        assert_eq!(notes.current, Note::default());
        assert_eq!(notes.earlier.len(), KEEP_NOTES);
        assert_eq!(notes.earlier[0].text, "note 24");
        assert_eq!(notes.earlier[19].text, "note 5");

        // A blank note isn't filed.
        pad.set_current("   ".into(), 200);
        assert_eq!(pad.new_note(201).earlier.len(), KEEP_NOTES);
        assert_eq!(
            Scratchpad::open(path.clone()).get().earlier.len(),
            KEEP_NOTES
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn opens_and_deletes_earlier_notes() {
        let path = temp("open");
        let pad = Scratchpad::open(path.clone());
        pad.set_current("first".into(), 5);
        pad.new_note(6);
        pad.set_current("second".into(), 5);
        // Same timestamp as "first": it gets a unique one.
        let notes = pad.new_note(7);
        assert_eq!(
            notes.earlier.iter().map(|n| n.at).collect::<Vec<_>>(),
            [6, 5]
        );

        // Opening "first" swaps it with the current note (here: blank).
        let notes = pad.open_note(5, 8);
        assert_eq!(notes.current.text, "first");
        assert_eq!(notes.earlier.len(), 1);

        pad.set_current("first, edited".into(), 9);
        let notes = pad.open_note(6, 10);
        assert_eq!(notes.current.text, "second");
        assert_eq!(notes.earlier[0].text, "first, edited");

        let notes = pad.delete_note(9);
        assert!(notes.earlier.is_empty());
        assert_eq!(Scratchpad::open(path.clone()).get(), notes);
        let _ = std::fs::remove_file(&path);
    }
}
