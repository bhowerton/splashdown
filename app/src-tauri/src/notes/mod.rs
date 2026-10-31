//! The Notes window (spec §9e), which replaced the scratchpad: notes are
//! Markdown files in a folder the user picks (`store`), listed and edited
//! here. ⌃⌥N or the menu bar opens it and remembers the app that was in
//! front, so "Send to {app}" can hand focus back and paste the note there.

pub mod store;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use splashdown_platform::InsertMethod;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::DialogExt;

use crate::commands::AppState;
use crate::engine;
use store::{Note, Origin, Saved, Store, Summary, Version};

pub const WINDOW: &str = "notes";
/// Shown in the menu; registered in `shortcuts.rs`.
pub const ACCELERATOR: &str = "Ctrl+Alt+N";
/// App event telling the page it was shown again, with the app "Send to"
/// would paste into (or null).
const SHOWN: &str = "notes-shown";
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

/// `Documents/Splashdown Notes`.
fn default_folder() -> Option<PathBuf> {
    let documents = directories::UserDirs::new()
        .and_then(|d| d.document_dir().map(Path::to_path_buf))
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join("Documents")))?;
    Some(documents.join("Splashdown Notes"))
}

/// The folder in settings, or the default.
fn wanted_folder(state: &AppState) -> Option<PathBuf> {
    state.settings().notes.folder.or_else(default_folder)
}

/// Where notes go while the chosen folder can't be used (macOS refused
/// Documents access, a removed drive), so nothing typed is lost.
fn fallback_folder() -> Result<PathBuf, String> {
    Ok(engine::paths()?.config_dir.join("notes"))
}

struct Current {
    /// The folder asked for.
    wanted: PathBuf,
    store: Arc<Store>,
    /// Why notes are in the fallback folder instead.
    problem: Option<String>,
}

fn current_slot() -> &'static Mutex<Option<Current>> {
    static CURRENT: OnceLock<Mutex<Option<Current>>> = OnceLock::new();
    CURRENT.get_or_init(Mutex::default)
}

/// The store for the notes folder, set up the first time: created
/// lazily (never at launch, so the Documents prompt only appears when
/// Notes is used), with the old scratchpad's notes moved in.
fn store(state: &AppState) -> Result<(Arc<Store>, Option<String>), String> {
    let wanted = wanted_folder(state).ok_or("Couldn't find your Documents folder.")?;
    let mut slot = current_slot().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = slot
        .as_ref()
        .filter(|c| c.wanted == wanted && c.problem.is_none())
    {
        return Ok((c.store.clone(), None));
    }
    let chosen = Store::new(wanted.clone());
    let (store, problem) = match chosen.ensure() {
        Ok(()) => (chosen, None),
        Err(e) => {
            tracing::warn!("notes folder {} unusable: {e}", wanted.display());
            let fallback = Store::new(fallback_folder()?);
            fallback.ensure().map_err(|e| e.to_string())?;
            let why = if e.kind() == std::io::ErrorKind::PermissionDenied {
                format!(
                    "Splashdown isn't allowed to use {}. Notes are kept in its own folder until you choose another one or allow it in System Settings → Privacy & Security → Files and Folders.",
                    label(&wanted)
                )
            } else {
                format!(
                    "Couldn't use {}: {e}. Notes are kept in Splashdown's own folder until you choose another one.",
                    label(&wanted)
                )
            };
            (fallback, Some(why))
        }
    };
    let store = Arc::new(store);
    migrate_scratchpad(&store);
    *slot = Some(Current {
        wanted,
        store: store.clone(),
        problem: problem.clone(),
    });
    Ok((store, problem))
}

/// Moves the scratchpad's notes (`scratchpad.json`) into the folder, once:
/// the file is renamed `scratchpad.json.migrated` afterwards, not deleted.
fn migrate_scratchpad(store: &Store) {
    let Ok(paths) = engine::paths() else {
        return;
    };
    let old = paths.config_dir.join("scratchpad.json");
    let Ok(json) = std::fs::read_to_string(&old) else {
        return;
    };
    match store.import_scratchpad(&json) {
        Ok(n) => {
            tracing::info!("moved {n} scratchpad notes into the notes folder");
            if let Err(e) = std::fs::rename(&old, old.with_extension("json.migrated")) {
                tracing::warn!("couldn't rename scratchpad.json: {e}");
            }
        }
        Err(e) => tracing::warn!("couldn't move the scratchpad's notes: {e}"),
    }
}

/// "Documents › Splashdown Notes", or the full path outside the home folder.
fn label(path: &Path) -> String {
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rel) if rel.components().count() > 0 => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" › "),
        _ => path.display().to_string(),
    }
}

/// The sync service that has a copy of `path`, best effort (spec §9e.8).
pub fn cloud_service(path: &Path) -> Option<&'static str> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    if cfg!(target_os = "macos") {
        if path.starts_with(home.join("Library/Mobile Documents")) {
            return Some("iCloud Drive");
        }
        if let Ok(rest) = path.strip_prefix(home.join("Library/CloudStorage")) {
            let provider = rest.components().next()?.as_os_str().to_string_lossy();
            return Some(match provider.split('-').next().unwrap_or("") {
                "Dropbox" => "Dropbox",
                "OneDrive" => "OneDrive",
                "GoogleDrive" => "Google Drive",
                "Box" => "Box",
                _ => "a cloud storage service",
            });
        }
        if path.starts_with(home.join("Dropbox")) {
            return Some("Dropbox");
        }
        // "Desktop & Documents Folders" in iCloud Drive.
        if path.starts_with(home.join("Documents"))
            && home
                .join("Library/Mobile Documents/com~apple~CloudDocs/Documents")
                .exists()
        {
            return Some("iCloud Drive");
        }
    } else {
        // Windows 11 often backs up Documents to OneDrive by default.
        for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if std::env::var_os(var).is_some_and(|dir| path.starts_with(dir)) {
                return Some("OneDrive");
            }
        }
        if path.starts_with(home.join("Dropbox")) {
            return Some("Dropbox");
        }
    }
    None
}

/// Moves every note (and `.splashdown/`) from one folder to another,
/// never overwriting: a name that's taken gets a number. Returns how many
/// moved.
fn move_notes(from: &Store, to: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let mut moved = 0;
    for note in from.list() {
        let source = from.root().join(&note.path);
        let mut target = to.join(&note.path);
        let mut n = 2;
        while target.exists() {
            let stem = Path::new(&note.path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            target.set_file_name(format!("{stem} {n}.md"));
            n += 1;
        }
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        if std::fs::rename(&source, &target).is_err() {
            // Another volume: copy, then remove the original.
            std::fs::copy(&source, &target).map_err(|e| e.to_string())?;
            std::fs::remove_file(&source).map_err(|e| e.to_string())?;
        }
        moved += 1;
    }
    let meta = from.root().join(".splashdown");
    if meta.is_dir()
        && !to.join(".splashdown").exists()
        && let Err(e) = std::fs::rename(&meta, to.join(".splashdown"))
    {
        tracing::warn!("versions stayed in the old notes folder: {e}");
    }
    Ok(moved)
}

// ---------------------------------------------------------------- window

/// The app that was in front when the window opened: (pid, name).
fn target() -> &'static Mutex<Option<(i32, String)>> {
    static TARGET: OnceLock<Mutex<Option<(i32, String)>>> = OnceLock::new();
    TARGET.get_or_init(Mutex::default)
}

fn target_lock() -> std::sync::MutexGuard<'static, Option<(i32, String)>> {
    target().lock().unwrap_or_else(|e| e.into_inner())
}

/// Remembers the app in front, unless it's us (Settings, or Notes itself):
/// then the previous one stays.
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

/// Whether the Notes window is in front with focus.
pub fn is_focused(app: &AppHandle) -> bool {
    app.get_webview_window(WINDOW)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

/// ⌃⌥N: hides Notes if it has focus, otherwise opens it.
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

/// Shows the Notes window (creating it the first time) with the note
/// focused.
pub fn open(app: &AppHandle) {
    remember_front_app();
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to(WINDOW, SHOWN, target_name());
        return;
    }
    let result = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("notes.html".into()))
        .title("Notes")
        .inner_size(640.0, 420.0)
        .min_inner_size(360.0, 240.0)
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
        Err(e) => tracing::error!("failed to open Notes: {e}"),
    }
}

/// Hides Notes, brings back the app that was in front before it opened,
/// and pastes `text` there. On failure the note is left on the clipboard
/// and the window comes back.
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
        tracing::warn!("notes send failed: {e}");
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
    /// The app "Send to" pastes into.
    target: Option<String>,
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
    })
}

#[tauri::command]
pub fn notes_load(state: State<'_, AppState>) -> Result<View, String> {
    view(&state)
}

#[tauri::command]
pub fn notes_list(state: State<'_, AppState>) -> Result<Vec<Summary>, String> {
    Ok(store(&state)?.0.list())
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
    state: State<'_, AppState>,
) -> Result<Option<Saved>, String> {
    let store = store(&state)?.0;
    let saved = match path {
        Some(path) => store.save(&path, &text, base),
        None if text.trim().is_empty() => return Ok(None),
        None => {
            let app = target_name();
            store
                .create(
                    &text,
                    chrono::Local::now(),
                    &Origin {
                        app: app.as_deref(),
                        source: None,
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
    store(&state)?.0.delete(&path)
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
    let mut settings = state.settings();
    settings.notes.folder = (Some(&folder) != default_folder().as_ref()).then_some(folder);
    settings.save(&engine::paths()?.config_file())?;
    *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
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
    state: State<'_, AppState>,
) -> Result<crate::session::TransformOutcome, String> {
    let session = state.session.clone();
    let out = tauri::async_runtime::spawn_blocking(move || session.transform(id, text))
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

/// Pastes can block for a few hundred ms, so this runs off the main thread.
#[tauri::command]
pub async fn notes_send(text: String, app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || send(&app, &text))
        .await
        .map_err(|e| e.to_string())?
}

/// "Edit transforms…": Settings, on the tab that lists them.
#[tauri::command]
pub fn notes_edit_transforms(app: AppHandle) {
    crate::tray::open_settings_tab(&app, "words");
}

/// From Settings → Privacy.
#[tauri::command]
pub fn open_notes(app: AppHandle) {
    open(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        std::fs::create_dir_all(base.join("to")).unwrap();
        std::fs::write(base.join("to/Plan.md"), "Theirs").unwrap();

        assert_eq!(move_notes(&from, &base.join("to")).unwrap(), 2);
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
