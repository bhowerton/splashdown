//! Notes: Markdown files in a folder the user picks, listed and edited in
//! the Notes window. ⌃⌥N or the menu bar opens it and remembers the app
//! that was in front, so "Paste into {app}" can hand focus back and paste
//! the note there.
//!
//! This file has the Notes window's commands. The rest:
//!
//! | File | What it does |
//! |---|---|
//! | `store.rs` | Reading and writing note files, versions, the trash |
//! | `markdown.rs` | Front matter, tags, sections and titles, as text |
//! | `folder.rs` | Which folder, and moving notes to another |
//! | `window.rs` | The window, and pasting back into the app in front |
//! | `capture.rs` | A dictation into a note (the note letter) |
//! | `search.rs` | Finding notes for Ask, with excerpts |
//! | `meaning.rs` | Finding notes by meaning |
//! | `agents.rs` | Coding agents' access |
//! | `files.rs`, `media.rs` | Transcribing a file or an app's audio into a note |
//! | `links.rs`, `link_helper.rs` | Transcribing a link, and the helper that downloads pages |

mod agents;
mod capture;
pub mod files;
mod folder;
mod link_helper;
pub mod links;
pub mod markdown;
pub mod meaning;
pub mod media;
mod search;
pub mod store;
mod window;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use splashdown_platform::InsertMethod;
use tauri::{
    AppHandle, DragDropEvent, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_dialog::DialogExt;

use crate::engine;
use crate::state::AppState;
use splashdown_config::{CaptureTo, NotesSettings};
use store::{Appended, Note, Origin, Saved, Store, Summary, Target, Version};

pub use self::agents::*;
pub use self::capture::*;
pub use self::folder::*;
pub use self::search::*;
pub use self::window::*;

pub const WINDOW: &str = "notes";
/// Shown in the menu; registered in `shortcuts.rs`.
pub const ACCELERATOR: &str = "Ctrl+Alt+N";
/// App event telling the page it was shown again, with the app "Paste into"
/// would paste into (or null).
const SHOWN: &str = "notes-shown";
/// App event: open a new note ("new") or today's note ("daily").
const THEN: &str = "notes-then";
/// App event with a new app for "Paste into" (you used another app).
const TARGET: &str = "notes-send-target";
/// App event asking the page to run a transform (its shortcut was pressed
/// while Notes had focus), with the transform's id.
pub const RUN_TRANSFORM: &str = "notes-run-transform";
/// Reopened after this long, the window starts a new note rather than
/// showing the last one.
const FRESH_AFTER: Duration = Duration::from_secs(6 * 3600);
/// How long to wait for the other app to come to the front before pasting.
const ACTIVATE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Once it's in front, a moment for its window to take keyboard focus.
const SETTLE: Duration = Duration::from_millis(120);

// ---------------------------------------------------------------- folder

// ---------------------------------------------------------------- capture

// ---------------------------------------------------------------- coding agents

// ---------------------------------------------------------------- ask

// ---------------------------------------------------------------- window

// ---------------------------------------------------------------- commands

/// The folder, as Settings and the Notes window show it.
#[derive(Serialize)]
pub struct Folder {
    path: String,
    /// "Documents › Splashdown Notes".
    label: String,
    /// The sync service with a copy, e.g. "iCloud Drive".
    synced: Option<&'static str>,
    /// Why notes are somewhere else for now.
    problem: Option<String>,
}

fn folder_of(root: &Path, problem: Option<String>) -> Folder {
    Folder {
        path: root.display().to_string(),
        label: label(root),
        synced: cloud_service(root),
        problem,
    }
}

/// What the page shows when it opens.
#[derive(Serialize)]
pub struct View {
    folder: Folder,
    notes: Vec<Summary>,
    /// The note to show, unless it's time for a fresh one.
    open: Option<Note>,
    /// The app "Paste into" pastes into.
    target: Option<String>,
    /// The window stays above other apps' windows.
    on_top: bool,
}

fn view(state: &AppState) -> Result<View, String> {
    let (store, problem) = store(state)?;
    let fresh_after = FRESH_AFTER.as_millis() as u64;
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let open = store
        .last()
        .filter(|(_, at)| now.saturating_sub(*at) < fresh_after)
        .and_then(|(path, _)| store.open(&path).ok());
    Ok(View {
        folder: folder_of(store.root(), problem),
        notes: store.list(),
        open,
        target: target_name(),
        on_top: state.settings().notes.keep_on_top,
    })
}

/// Keeps the window above other apps' windows, or not, and remembers it.
#[tauri::command]
pub fn notes_keep_on_top(
    on: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WINDOW) {
        window.set_always_on_top(on).map_err(|e| e.to_string())?;
    }
    state.update(&app, |s| s.notes.keep_on_top = on)?;
    Ok(())
}

#[tauri::command]
pub fn notes_load(state: State<'_, AppState>) -> Result<View, String> {
    view(&state)
}

/// Polled every few seconds; a big folder (or files a sync service has to
/// download first) takes a while to read, so not on the main thread.
#[tauri::command]
pub async fn notes_list(state: State<'_, AppState>) -> Result<Vec<Summary>, String> {
    let folder = state.settings().notes.folder;
    tauri::async_runtime::spawn_blocking(move || Ok(store_for(folder)?.0.list()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn notes_open(path: String, state: State<'_, AppState>) -> Result<Note, String> {
    store(&state)?.0.open(&path).map_err(|e| e.to_string())
}

/// Saves the editor's text: a new file the first time a note has any,
/// otherwise the note's own file. `None` for a new note that's still blank.
#[tauri::command]
pub fn notes_save(
    path: Option<String>,
    text: String,
    base: Option<u64>,
    folder: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<Saved>, String> {
    let store = store(&state)?.0;
    let saved = match path {
        Some(path) => store.save(&path, &text, base),
        None if text.trim().is_empty() => return Ok(None),
        None => {
            let app = target_name();
            // A new note goes in the folder being looked at.
            store
                .create_in(
                    folder.as_deref().unwrap_or(""),
                    &text,
                    chrono::Local::now(),
                    &Origin {
                        app: app.as_deref(),
                        ..Origin::default()
                    },
                )
                .map(|note| Saved {
                    note,
                    conflict: false,
                })
        }
    };
    saved.map(Some).map_err(|e| e.to_string())
}

/// Starts a new note: the next save creates its file.
#[tauri::command]
pub fn notes_new(state: State<'_, AppState>) -> Result<(), String> {
    store(&state)?.0.set_last(None);
    Ok(())
}

#[tauri::command]
pub fn notes_delete(path: String, state: State<'_, AppState>) -> Result<(), String> {
    not_recording(&path)?;
    store(&state)?.0.delete(&path)
}

/// Today's daily note (in the folder and with the name set for captures
/// in Settings → Notes), made if it isn't there yet.
#[tauri::command]
pub fn notes_daily(state: State<'_, AppState>) -> Result<Note, String> {
    let settings = state.settings().notes;
    let (store, _) = store(&state)?;
    store
        .daily(
            &Target::Daily {
                folder: settings.daily_folder().to_string(),
                format: settings.daily_format().to_string(),
            },
            chrono::Local::now(),
        )
        .map_err(|e| e.to_string())
}

/// Neither a meeting nor a file's transcript is being written into a note
/// in `folder`, so it can be moved or trashed.
fn nothing_written_in(folder: &str) -> Result<(), String> {
    let inside = |note: &str| note.starts_with(&format!("{folder}/"));
    if crate::meeting::status().is_some_and(|m| inside(&m.note)) {
        return Err(
            "A recording is being written into a note in this folder. Stop it first.".into(),
        );
    }
    if files::notes_transcribing().is_some_and(|j| inside(&j.note)) {
        return Err("A file is being transcribed into a note in this folder. Wait for it, or stop it first.".into());
    }
    Ok(())
}

/// The subfolders of the notes folder, for the sidebar.
#[tauri::command]
pub async fn notes_folders(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let (store, _) = store(&state)?;
    tauri::async_runtime::spawn_blocking(move || store.folders())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn notes_create_folder(
    parent: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    store(&state)?
        .0
        .create_folder(&parent, &name)
        .map_err(|e| e.to_string())
}

/// Renames a folder (`parent` unchanged) or moves it into another.
#[tauri::command]
pub fn notes_move_folder(
    path: String,
    parent: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    nothing_written_in(&path)?;
    store(&state)?
        .0
        .move_folder(&path, &parent, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn notes_delete_folder(path: String, state: State<'_, AppState>) -> Result<(), String> {
    nothing_written_in(&path)?;
    store(&state)?.0.delete_folder(&path)
}

/// Moves a note into `folder` ("" for the top). Returns its new path.
#[tauri::command]
pub fn notes_move(
    path: String,
    folder: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    not_recording(&path)?;
    if files::notes_transcribing().is_some_and(|j| j.note == path) {
        return Err("This note is being transcribed into. Wait for it, or stop it first.".into());
    }
    store(&state)?
        .0
        .move_note(&path, &folder)
        .map_err(|e| e.to_string())
}

/// Sets the tags in a note's front matter.
#[tauri::command]
pub fn notes_set_tags(
    path: String,
    tags: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Note, String> {
    store(&state)?
        .0
        .set_front_tags(&path, &tags)
        .map_err(|e| e.to_string())
}

/// Keeps a note from coding agents (`agents: false` in its front matter),
/// or lets them reach it again.
#[tauri::command]
pub fn notes_set_hidden_from_agents(
    path: String,
    hidden: bool,
    state: State<'_, AppState>,
) -> Result<Note, String> {
    store(&state)?
        .0
        .set_front_value(&path, "agents", hidden.then_some("false"))
        .map_err(|e| e.to_string())
}

/// A meeting writes into its note as it goes: it can't be moved meanwhile.
fn not_recording(path: &str) -> Result<(), String> {
    match crate::meeting::status() {
        Some(meeting) if meeting.note == path => {
            Err("A meeting is being written into this note. Stop the meeting first.".into())
        }
        _ => Ok(()),
    }
}

#[tauri::command]
pub fn notes_pin(path: String, pinned: bool, state: State<'_, AppState>) -> Result<(), String> {
    store(&state)?.0.set_pinned(&path, pinned);
    Ok(())
}

#[tauri::command]
pub fn notes_rename(
    path: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    not_recording(&path)?;
    store(&state)?
        .0
        .rename(&path, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn notes_versions(path: String, state: State<'_, AppState>) -> Result<Vec<Version>, String> {
    store(&state)?.0.versions(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn notes_restore(path: String, at: u64, state: State<'_, AppState>) -> Result<Note, String> {
    store(&state)?
        .0
        .restore(&path, at)
        .map_err(|e| e.to_string())
}

/// Shows a note, or the notes folder, in Finder / File Explorer.
#[tauri::command]
pub fn notes_reveal(path: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    let store = store(&state)?.0;
    let target = match path {
        Some(p) => store.root().join(p),
        None => store.root().to_path_buf(),
    };
    splashdown_platform::reveal_in_file_manager(&target).map_err(|e| e.to_string())
}

/// The folder Settings shows. Doesn't create it, so opening Settings never
/// triggers the Documents prompt.
#[tauri::command]
pub fn notes_folder(state: State<'_, AppState>) -> Result<Folder, String> {
    let slot = current_slot().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = slot.as_ref() {
        return Ok(folder_of(c.store.root(), c.problem.clone()));
    }
    drop(slot);
    let wanted = wanted_folder(&state).ok_or("Couldn't find your Documents folder.")?;
    Ok(folder_of(&wanted, None))
}

/// A native folder picker, starting at the current folder. `None` if the
/// user cancelled.
#[tauri::command]
pub async fn notes_pick_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let start = wanted_folder(&state);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app
            .dialog()
            .file()
            .set_title("Choose a folder for your notes");
        if let Some(dir) = start.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        dialog.blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

/// Uses `folder` for notes from now on, moving the current notes there
/// first if asked. Returns the new view.
#[tauri::command]
pub fn notes_set_folder(
    folder: String,
    move_notes_too: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<View, String> {
    let folder = PathBuf::from(folder);
    Store::new(folder.clone())
        .ensure()
        .map_err(|e| format!("Can't use {}: {e}", label(&folder)))?;
    if move_notes_too {
        let (old, _) = store(&state)?;
        if old.root() != folder {
            let moved = move_notes(&old, &folder)?;
            tracing::info!("moved {moved} notes to the new folder");
        }
    }
    let kept = (Some(&folder) != default_folder().as_ref()).then_some(folder);
    state.update(&app, |s| s.notes.folder = kept)?;
    *current_slot().lock().unwrap_or_else(|e| e.into_inner()) = None;
    view(&state)
}

/// A transform as the Notes menu shows it.
#[derive(Serialize)]
pub struct TransformItem {
    id: String,
    name: String,
    place: splashdown_config::Place,
    shortcut: Option<String>,
}

#[tauri::command]
pub fn notes_transforms(state: State<'_, AppState>) -> Vec<TransformItem> {
    state
        .settings()
        .transforms()
        .into_iter()
        .map(|t| TransformItem {
            id: t.id,
            name: t.name,
            place: t.place,
            shortcut: t.shortcut,
        })
        .collect()
}

/// Runs transform `id` on `text` (the note, or the selection in it). The
/// note as it is on disk is kept as a version first, so the change can
/// always be undone from History. The page puts the result in place.
#[tauri::command]
pub async fn notes_transform(
    path: Option<String>,
    id: String,
    text: String,
    extra: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::session::TransformOutcome, String> {
    let session = state.session.clone();
    let out = tauri::async_runtime::spawn_blocking(move || session.transform_with(id, text, extra))
        .await
        .map_err(|e| e.to_string())??;
    if out.unchanged.is_none()
        && let Some(path) = path
    {
        let (store, _) = store(&state)?;
        if let Err(e) = store.keep_current(&path) {
            tracing::warn!("couldn't keep a version before the transform: {e}");
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- ask

/// App event to the page with a question's answer so far (`AskUpdate`).
const ASKED: &str = "notes-ask";
/// Most notes a question about all of them is answered from, and how much
/// of each.
const NOTES_ASK_NOTES: usize = 5;
const NOTES_ASK_EXCERPT: usize = 2000;
/// How much of one note a question about it may use.
const ONE_NOTE_CHARS: usize = 24_000;

/// A note an answer may cite, as `[n]`: `n` is its place in the list, from 1.
#[derive(Debug, Clone, Serialize)]
pub struct Source {
    title: String,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct AskUpdate {
    /// The page's number for the question, so an older answer still coming
    /// in is told apart.
    asked: u64,
    answer: String,
    done: bool,
    error: Option<String>,
    sources: Vec<Source>,
}

/// A question asked in the Notes window, about the note at `path` or (with
/// none) the notes that match it. The answer streams to the page as
/// `notes-ask` events. `recent` asks about the end of a long note (a
/// meeting still going); `earlier` holds this conversation's questions and
/// answers so far.
#[tauri::command]
pub async fn notes_ask(
    question: String,
    path: Option<String>,
    recent: bool,
    earlier: Vec<(String, String)>,
    asked: u64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Ok(());
    }
    let session = state.session.clone();
    let setup = tauri::async_runtime::spawn_blocking(move || session.answer_model())
        .await
        .map_err(|e| e.to_string())??
        .ok_or("Asking needs an AI model: choose one in Settings → AI Models.")?;
    let (store, _) = store(&state)?;
    let (notes, sources): (Vec<_>, Vec<_>) = match &path {
        Some(path) => {
            let note = store.read(path).map_err(|e| e.to_string())?;
            let title = markdown::title_of(&note.body);
            let excerpt = splashdown_core::ask::NoteExcerpt {
                title: title.clone(),
                date: day_of(note.modified),
                text: clip(&note.body, ONE_NOTE_CHARS, recent),
            };
            let source = Source {
                title,
                path: path.clone(),
            };
            (vec![excerpt], vec![source])
        }
        None => {
            // Finding notes by meaning can take a moment.
            let by_meaning = state.settings().notes.ask_by_meaning;
            let asked = question.clone();
            tauri::async_runtime::spawn_blocking(move || {
                matching(
                    &store,
                    &asked,
                    NOTES_ASK_NOTES,
                    NOTES_ASK_EXCERPT,
                    by_meaning,
                )
            })
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(path, excerpt)| {
                let source = Source {
                    title: excerpt.title.clone(),
                    path,
                };
                (excerpt, source)
            })
            .unzip()
        }
    };
    tracing::info!("asking in Notes, with {} note(s)", notes.len());
    let context = splashdown_core::ask::AskContext {
        notes,
        user_name: setup.user_name,
        today: splashdown_core::prompt::today(chrono::Local::now().date_naive()),
        earlier,
        cite_numbers: true,
        spoken: false,
        ..Default::default()
    };
    let request = splashdown_core::ask::request(&question, &context);
    let model = setup.model;
    tauri::async_runtime::spawn(async move {
        let emit = |answer: &str, done: bool, error: Option<String>| {
            let _ = app.emit_to(
                WINDOW,
                ASKED,
                AskUpdate {
                    asked,
                    answer: answer.to_string(),
                    done,
                    error,
                    sources: sources.clone(),
                },
            );
        };
        let mut on_text = |so_far: &str| emit(so_far, false, None);
        let result = model.complete_streaming(&request, &mut on_text).await;
        let usage = match &result {
            Ok(completion) => Some(completion.usage),
            Err(splashdown_core::ProviderError::Interrupted { usage, .. }) => Some(*usage),
            Err(_) => None,
        };
        if let Some(usage) = usage {
            crate::spend::shared().record(&[(model.name(), usage)], chrono::Local::now());
        }
        match result {
            Ok(completion) => emit(completion.text.trim(), true, None),
            Err(e) => {
                tracing::warn!("asking in Notes failed: {e}");
                let message = crate::errors::pipeline(
                    &splashdown_core::PipelineError::Llm(e),
                    None,
                    Some(&model.name()),
                );
                emit("", true, Some(message));
            }
        }
    });
    Ok(())
}

/// Pastes can block for a few hundred ms, so this runs off the main thread.
#[tauri::command]
pub async fn notes_send(text: String, app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || send(&app, &text))
        .await
        .map_err(|e| e.to_string())?
}

/// History → "Save to notes": adds the text to the capture note. Returns
/// the note's name.
#[tauri::command]
pub fn notes_add(text: String, state: State<'_, AppState>) -> Result<String, String> {
    capture(&state.settings().notes, None, &text, None).map(|c| c.label)
}

/// "Edit transforms…": Settings, on the tab that lists them.
#[tauri::command]
// Async: a synchronous command runs on the main thread, and building a
// window from there deadlocks on Windows (here: Settings).
pub async fn notes_edit_transforms(app: AppHandle) {
    crate::tray::open_settings_tab(&app, "words");
}

/// From Settings → Privacy.
/// Brings the notes' vectors up to date in the background, when Ask finds
/// notes by meaning: changed notes are then ready by the time they're
/// asked about.
fn read_for_meaning(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let settings = state.settings().notes;
    if !settings.ask_by_meaning || !meaning::available() {
        return;
    }
    if let Ok((store, _)) = store_for(settings.folder) {
        meaning::update_soon(store);
    }
}

/// How far along reading the notes for meaning is, for Settings: notes
/// read, of all of them (and starts reading any that aren't).
#[tauri::command]
pub async fn notes_meaning_progress(app: AppHandle) -> Result<(usize, usize), String> {
    read_for_meaning(&app);
    let state = app.state::<AppState>();
    let (store, _) = store(&state)?;
    tauri::async_runtime::spawn_blocking(move || meaning::progress(store.root(), &store.list()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
// Async: a synchronous command runs on the main thread, and building a
// window from there deadlocks on Windows (here: Notes).
pub async fn open_notes(app: AppHandle) {
    open(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(title: &str, text: &str, modified: u64) -> Summary {
        Summary {
            path: format!("{title}.md"),
            title: title.into(),
            text: text.into(),
            modified,
            pinned: false,
            folder: String::new(),
            source: String::new(),
            tags: Vec::new(),
            created: None,
            hidden_from_agents: false,
        }
    }

    #[test]
    fn questions_find_the_notes_that_match() {
        let notes = [
            summary(
                "Launch plan",
                "We moved the launch to Thursday. The docs are late.",
                1,
            ),
            summary("Groceries", "Milk, eggs and the bread from the bakery.", 2),
            summary(
                "Standup",
                "The onboarding bug is fixed. The launch is close.",
                3,
            ),
            summary("Ideas", "The app could read the clipboard.", 4),
            summary("Recipes", "The best bread needs time.", 5),
        ];
        let found = best_matches(&notes, "When is the launch? What about the docs?", 3);
        let titles: Vec<&str> = found.iter().map(|(n, _)| n.title.as_str()).collect();
        // "the" is in every note and counts for nothing; "docs" is rarest.
        assert_eq!(titles, ["Launch plan", "Standup"]);
        assert_eq!(found[0].1, "docs");
        assert!(best_matches(&notes, "the", 3).is_empty());
        // Whole words: "stand" isn't in "Standup".
        assert!(best_matches(&notes, "stand", 3).is_empty());
        let found = best_matches(&notes, "eggs", 3);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.title, "Groceries");
        assert!(best_matches(&notes, "", 3).is_empty());
        // A single note still matches.
        let one = [summary("Launch plan", "Thursday.", 1)];
        assert_eq!(best_matches(&one, "launch day", 3).len(), 1);
    }

    #[test]
    fn common_words_and_unspaced_scripts() {
        let w = words_of("What did we decide about the Launch?");
        assert!(w.contains("decide") && w.contains("launch"));
        assert!(!w.contains("what") && !w.contains("the") && !w.contains("about"));
        // Japanese: pairs of characters.
        let w = words_of("発売日はいつ");
        assert!(w.contains("発売") && w.contains("売日"));
        // With a single note, common words don't make it match.
        let one = [summary("Weather", "What is the forecast", 1)];
        assert!(best_matches(&one, "what's the time", 3).is_empty());
    }

    #[test]
    fn excerpts_are_cut_around_the_word() {
        let text = format!("{} needle {}", "a".repeat(2000), "b".repeat(2000));
        let cut = around(&text, "needle", 300);
        assert!(cut.starts_with('…') && cut.ends_with('…'));
        assert!(cut.contains("needle"));
        assert_eq!(cut.chars().count(), 302);
        assert_eq!(around("short", "x", 300), "short");
        // The whole word, not part of a longer one.
        let text = format!("start {} the art show", "x".repeat(2000));
        assert!(around(&text, "art", 300).contains("art show"));
    }

    #[test]
    fn moving_notes_never_overwrites() {
        let base =
            std::env::temp_dir().join(format!("splashdown-notes-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let from = Store::new(base.join("from"));
        from.ensure().unwrap();
        std::fs::create_dir_all(base.join("from/Work")).unwrap();
        std::fs::write(base.join("from/Plan.md"), "Plan").unwrap();
        std::fs::write(base.join("from/Work/Standup.md"), "Standup").unwrap();
        std::fs::create_dir_all(base.join("from/.splashdown/versions/1-ab")).unwrap();
        std::fs::write(base.join("from/.splashdown/versions/1-ab/5.md"), "old").unwrap();
        std::fs::write(base.join("from/.splashdown/state.json"), "{}").unwrap();
        std::fs::create_dir_all(base.join("from/Attachments")).unwrap();
        std::fs::write(base.join("from/Attachments/a.wav"), "RIFF").unwrap();
        // The new folder is set up before the move, as Notes does.
        Store::new(base.join("to")).ensure().unwrap();
        std::fs::write(base.join("to/Plan.md"), "Theirs").unwrap();

        assert_eq!(move_notes(&from, &base.join("to")).unwrap(), 2);
        assert!(base.join("to/.splashdown/versions/1-ab/5.md").exists());
        assert!(base.join("to/.splashdown/state.json").exists());
        assert!(base.join("to/Attachments/a.wav").exists());
        assert_eq!(
            std::fs::read_to_string(base.join("to/Plan.md")).unwrap(),
            "Theirs"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("to/Plan 2.md")).unwrap(),
            "Plan"
        );
        assert!(base.join("to/Work/Standup.md").exists());
        assert!(base.join("to/.splashdown").is_dir());
        assert!(from.list().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn labels_inside_home_use_arrows() {
        let home = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf();
        assert_eq!(
            label(&home.join("Documents/Splashdown Notes")),
            "Documents › Splashdown Notes"
        );
        assert_eq!(
            cloud_service(&home.join("Library/Mobile Documents/x")).is_some(),
            cfg!(target_os = "macos")
        );
    }
}
