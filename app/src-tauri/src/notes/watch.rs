//! Watched folders (Settings → Notes): new audio and video files in a
//! folder the user chose become transcript notes. The folders are scanned
//! every few seconds rather than watched for events, which iCloud, Dropbox
//! and network folders don't report reliably; a file is taken once it
//! stops changing, so one still copying or recording isn't taken early.
//! What was taken (or was already there when the folder was added) is kept
//! in `watched.json`, so a restart does nothing twice.

use std::collections::HashMap;

use serde::Deserialize;
use splashdown_config::WatchedFolder;

use super::files::{Item, add_items, is_media};
use super::*;

/// How often the folders are looked at.
const EVERY: Duration = Duration::from_secs(10);

/// What's been seen, kept between runs.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Record {
    /// Folders looked at already: their files when first seen are known.
    folders: Vec<PathBuf>,
    files: HashMap<PathBuf, Seen>,
}

/// A file as it was when taken, or left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Seen {
    size: u64,
    modified: u64,
    /// Already in the folder when it was added: left alone unless asked.
    left: bool,
}

/// A media file in a folder, as it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    path: PathBuf,
    size: u64,
    modified: u64,
}

fn record_file() -> Result<PathBuf, String> {
    Ok(engine::paths()?.config_dir.join("watched.json"))
}

/// The record, held while it's read and changed (the scanner and Settings
/// both change it).
fn record() -> std::sync::MutexGuard<'static, Option<Record>> {
    static RECORD: Mutex<Option<Record>> = Mutex::new(None);
    let mut guard = RECORD.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let loaded = record_file()
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        *guard = Some(loaded);
    }
    guard
}

fn save(record: &Record) {
    let saved = record_file().and_then(|path| {
        let json = serde_json::to_string(record).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    });
    if let Err(e) = saved {
        tracing::warn!("couldn't keep the watched folders' record: {e}");
    }
}

/// The audio and video files directly in `folder` (not its subfolders),
/// without hidden ones, empty ones or Office's lock files.
fn media_in(folder: &Path) -> Vec<Found> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.starts_with("~$") || !is_media(&path) {
                return None;
            }
            let meta = entry
                .metadata()
                .ok()
                .filter(|m| m.is_file() && m.len() > 0)?;
            let modified = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            Some(Found {
                path,
                size: meta.len(),
                modified,
            })
        })
        .collect()
}

/// What a scan does: the files to transcribe now. `pending` holds files
/// seen changing, as they were at the last scan.
fn scan(
    record: &mut Record,
    watched: &[WatchedFolder],
    pending: &mut HashMap<PathBuf, (u64, u64)>,
    listed: impl Fn(&Path) -> Vec<Found>,
) -> Vec<(Found, WatchedFolder)> {
    // A folder taken off the list is looked at afresh if it comes back.
    record
        .folders
        .retain(|f| watched.iter().any(|w| &w.path == f));
    let mut take = Vec::new();
    let mut still = HashMap::new();
    for folder in watched {
        let found = listed(&folder.path);
        if !record.folders.contains(&folder.path) {
            remember_as_left(record, &folder.path, &found);
            continue;
        }
        for file in found {
            let now = (file.size, file.modified);
            if record
                .files
                .get(&file.path)
                .is_some_and(|s| (s.size, s.modified) == now)
            {
                continue;
            }
            if pending.get(&file.path) == Some(&now) {
                record.files.insert(
                    file.path.clone(),
                    Seen {
                        size: file.size,
                        modified: file.modified,
                        left: false,
                    },
                );
                take.push((file, folder.clone()));
            } else {
                still.insert(file.path.clone(), now);
            }
        }
    }
    *pending = still;
    take
}

/// A folder seen for the first time: what's in it already is left alone.
fn remember_as_left(record: &mut Record, folder: &Path, found: &[Found]) {
    for file in found {
        record.files.insert(
            file.path.clone(),
            Seen {
                size: file.size,
                modified: file.modified,
                left: true,
            },
        );
    }
    record.folders.push(folder.to_path_buf());
}

/// Starts looking at the watched folders, for as long as the app runs.
pub fn watch_folders(app: &AppHandle) {
    let app = app.clone();
    let started = std::thread::Builder::new()
        .name("watched-folders".into())
        .spawn(move || {
            let mut pending = HashMap::new();
            loop {
                // Not before the app's state is set up, a moment after start.
                let Some(state) = app.try_state::<AppState>() else {
                    std::thread::sleep(EVERY);
                    continue;
                };
                let watched = state.settings().notes.watched;
                let take = {
                    let mut guard = record();
                    let record = guard.get_or_insert_with(Record::default);
                    let before = record.clone();
                    let take = scan(record, &watched, &mut pending, media_in);
                    if *record != before {
                        save(record);
                    }
                    take
                };
                if !take.is_empty() {
                    tracing::info!("{} new file(s) in watched folders", take.len());
                    add_items(&app, take.into_iter().map(item).collect());
                }
                std::thread::sleep(EVERY);
            }
        });
    if let Err(e) = started {
        tracing::error!("couldn't start watching folders: {e}");
    }
}

fn item((file, folder): (Found, WatchedFolder)) -> Item {
    Item {
        path: file.path,
        link: None,
        again: None,
        watched: Some(folder),
        reply: None,
    }
}

/// The files that were already in `folder` when it was added and are
/// still there unchanged, noting them as such if it's new.
fn waiting_in(record: &mut Record, folder: &Path) -> Vec<Found> {
    let found = media_in(folder);
    if !record.folders.iter().any(|f| f == folder) {
        remember_as_left(record, folder, &found);
        save(record);
    }
    found
        .into_iter()
        .filter(|f| {
            record
                .files
                .get(&f.path)
                .is_some_and(|s| s.left && (s.size, s.modified) == (f.size, f.modified))
        })
        .collect()
}

/// Settings: how many files were in a watched folder before it was, for
/// "Transcribe the 12 files already here".
#[tauri::command]
pub fn notes_watch_waiting(path: String) -> usize {
    let mut guard = record();
    let record = guard.get_or_insert_with(Record::default);
    waiting_in(record, Path::new(&path)).len()
}

/// Settings: transcribes those files too.
#[tauri::command]
pub fn notes_watch_take_waiting(
    path: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let folder = state
        .settings()
        .notes
        .watched
        .into_iter()
        .find(|w| w.path == Path::new(&path))
        .ok_or("Save the folder in Settings first.")?;
    let take: Vec<Found> = {
        let mut guard = record();
        let record = guard.get_or_insert_with(Record::default);
        let waiting = waiting_in(record, &folder.path);
        for file in &waiting {
            if let Some(seen) = record.files.get_mut(&file.path) {
                seen.left = false;
            }
        }
        save(record);
        waiting
    };
    let count = take.len();
    add_items(
        &app,
        take.into_iter()
            .map(|f| item((f, folder.clone())))
            .collect(),
    );
    Ok(count)
}

/// Settings → Add a folder…: a folder picker. Refuses the notes folder or
/// one inside it, whose recordings would be transcribed again.
#[tauri::command]
pub async fn notes_watch_pick(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let notes_root = wanted_folder(&state);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a folder to transcribe new files from")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(folder) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    if notes_root
        .as_deref()
        .is_some_and(|root| folder.starts_with(root))
    {
        return Err("That's your notes folder (or inside it): choose another one.".into());
    }
    Ok(Some(folder.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str, size: u64, modified: u64) -> Found {
        Found {
            path: PathBuf::from(path),
            size,
            modified,
        }
    }

    fn folder(path: &str) -> WatchedFolder {
        WatchedFolder {
            path: PathBuf::from(path),
            ..WatchedFolder::default()
        }
    }

    #[test]
    fn a_new_file_is_taken_once_it_stops_changing_and_only_once() {
        let watched = [folder("/in")];
        let mut record = Record::default();
        let mut pending = HashMap::new();
        let mut files = vec![found("/in/old.m4a", 10, 1)];
        let mut run = |record: &mut Record, files: &[Found]| {
            let files = files.to_vec();
            scan(record, &watched, &mut pending, move |_| files.clone())
                .into_iter()
                .map(|(f, _)| f.path)
                .collect::<Vec<_>>()
        };
        // First look: what's there already is left alone.
        assert!(run(&mut record, &files).is_empty());
        assert!(record.files[Path::new("/in/old.m4a")].left);
        assert!(run(&mut record, &files).is_empty());

        // A file being copied in: growing, then settled.
        files.push(found("/in/talk.mp4", 100, 5));
        assert!(run(&mut record, &files).is_empty());
        files[1] = found("/in/talk.mp4", 900, 6);
        assert!(run(&mut record, &files).is_empty());
        assert_eq!(run(&mut record, &files), [PathBuf::from("/in/talk.mp4")]);
        // Not again.
        assert!(run(&mut record, &files).is_empty());
        assert!(run(&mut record, &files).is_empty());

        // Replaced by a new version: taken again once settled.
        files[1] = found("/in/talk.mp4", 950, 9);
        assert!(run(&mut record, &files).is_empty());
        assert_eq!(run(&mut record, &files), [PathBuf::from("/in/talk.mp4")]);
    }

    #[test]
    fn a_folder_taken_off_the_list_starts_afresh() {
        let mut record = Record::default();
        let mut pending = HashMap::new();
        let files = vec![found("/in/a.wav", 1, 1)];
        let list = |_: &Path| files.clone();
        scan(&mut record, &[folder("/in")], &mut pending, list);
        assert_eq!(record.folders, [PathBuf::from("/in")]);
        scan(&mut record, &[], &mut pending, list);
        assert!(record.folders.is_empty());
        // Back again: a.wav is left alone, not transcribed.
        let more = vec![found("/in/a.wav", 1, 1), found("/in/b.wav", 2, 2)];
        let list = |_: &Path| more.clone();
        assert!(scan(&mut record, &[folder("/in")], &mut pending, list).is_empty());
        assert!(scan(&mut record, &[folder("/in")], &mut pending, list).is_empty());
        assert!(record.files[Path::new("/in/b.wav")].left);
    }

    #[test]
    fn only_audio_and_video_files_directly_inside_are_found() {
        let dir = std::env::temp_dir().join(format!("splashdown-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for (name, bytes) in [
            ("talk.MP4", 3),
            ("memo.m4a", 3),
            ("notes.txt", 3),
            (".hidden.wav", 3),
            ("empty.wav", 0),
            ("talk.srt", 3),
        ] {
            std::fs::write(dir.join(name), vec![0u8; bytes]).unwrap();
        }
        std::fs::write(dir.join("sub/deep.wav"), [0u8; 3]).unwrap();
        let mut names: Vec<String> = media_in(&dir)
            .into_iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["memo.m4a", "talk.MP4"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
