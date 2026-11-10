//! Watched folders (Settings → Notes): new audio and video files in a
//! folder the user chose become transcript notes. The folders are scanned
//! every few seconds rather than watched for events, which iCloud, Dropbox
//! and network folders don't report reliably; a file is taken once it
//! stops changing, so one still copying or recording isn't taken early.
//! What was taken (or was already there when the folder was added) is kept
//! in `watched.json`, so a restart does nothing twice, and a file whose
//! transcription the app quit before finishing is taken again.

use splashdown_core::sync::Locked;
use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use splashdown_config::WatchedFolder;

use super::files::{Item, add_items, is_media};
use super::*;

/// How often the folders are looked at.
const EVERY: Duration = Duration::from_secs(10);

/// What's been seen, kept between runs (`watched.json`).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Ledger {
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
    /// Taken, but no note has come of it yet: it's in the queue or being
    /// transcribed, or the app quit (or it failed) before one did.
    #[serde(default)]
    waiting: bool,
}

/// A media file in a folder, as it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    path: PathBuf,
    size: u64,
    modified: u64,
}

/// The ledger, held while it's read and changed (the scanner and Settings
/// both change it). Read from its file the first time.
fn ledger() -> std::sync::MutexGuard<'static, Ledger> {
    static LEDGER: OnceLock<Mutex<Ledger>> = OnceLock::new();
    LEDGER
        .get_or_init(|| {
            Mutex::new(
                crate::data::path("watched.json")
                    .map(|p| crate::data::read(&p))
                    .unwrap_or_default(),
            )
        })
        .locked()
}

/// The files this run of the app has taken: in the queue, being
/// transcribed, or tried with nothing to show for it. A file the ledger
/// still has as `waiting` that isn't here was left by an earlier run, and
/// is taken again.
fn taken() -> std::sync::MutexGuard<'static, HashSet<PathBuf>> {
    static TAKEN: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    TAKEN.get_or_init(Mutex::default).locked()
}

/// These watched files are done with: a note came of each (whole, or as
/// far as it got), or Stop dropped it from the queue. One that failed
/// before any note isn't among them, and is tried again when the app next
/// starts.
pub(super) fn settled(paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    let mut ledger = ledger();
    let mut changed = false;
    for path in paths {
        if let Some(seen) = ledger.files.get_mut(path)
            && seen.waiting
        {
            seen.waiting = false;
            changed = true;
        }
    }
    if changed {
        save(&ledger);
    }
}

fn save(ledger: &Ledger) {
    let saved =
        crate::data::path("watched.json").and_then(|path| crate::data::write(&path, ledger));
    if let Err(e) = saved {
        tracing::warn!("couldn't keep the watched folders' ledger: {e}");
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
/// seen changing, as they were at the last scan; `taken`, the files this
/// run of the app has taken already.
fn scan(
    ledger: &mut Ledger,
    watched: &[WatchedFolder],
    pending: &mut HashMap<PathBuf, (u64, u64)>,
    taken: &mut HashSet<PathBuf>,
    listed: impl Fn(&Path) -> Vec<Found>,
) -> Vec<(Found, WatchedFolder)> {
    // A folder taken off the list is looked at afresh if it comes back.
    ledger
        .folders
        .retain(|f| watched.iter().any(|w| &w.path == f));
    let mut take = Vec::new();
    let mut still = HashMap::new();
    for folder in watched {
        let found = listed(&folder.path);
        if !ledger.folders.contains(&folder.path) {
            remember_as_left(ledger, &folder.path, &found);
            continue;
        }
        for file in found {
            let now = (file.size, file.modified);
            if let Some(seen) = ledger
                .files
                .get(&file.path)
                .filter(|s| (s.size, s.modified) == now)
            {
                // Taken by an earlier run that didn't get to it (the app
                // quit, or it failed): once more, now.
                if seen.waiting && taken.insert(file.path.clone()) {
                    take.push((file, folder.clone()));
                }
                continue;
            }
            if pending.get(&file.path) == Some(&now) {
                ledger.files.insert(
                    file.path.clone(),
                    Seen {
                        size: file.size,
                        modified: file.modified,
                        left: false,
                        waiting: true,
                    },
                );
                taken.insert(file.path.clone());
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
fn remember_as_left(ledger: &mut Ledger, folder: &Path, found: &[Found]) {
    for file in found {
        ledger.files.insert(
            file.path.clone(),
            Seen {
                size: file.size,
                modified: file.modified,
                left: true,
                waiting: false,
            },
        );
    }
    ledger.folders.push(folder.to_path_buf());
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
                    let mut ledger = ledger();
                    let before = ledger.clone();
                    let take = scan(&mut ledger, &watched, &mut pending, &mut taken(), media_in);
                    if *ledger != before {
                        save(&ledger);
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
fn waiting_in(ledger: &mut Ledger, folder: &Path) -> Vec<Found> {
    let found = media_in(folder);
    if !ledger.folders.iter().any(|f| f == folder) {
        remember_as_left(ledger, folder, &found);
        save(ledger);
    }
    found
        .into_iter()
        .filter(|f| {
            ledger
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
    waiting_in(&mut ledger(), Path::new(&path)).len()
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
        let mut ledger = ledger();
        let waiting = waiting_in(&mut ledger, &folder.path);
        let mut taken = taken();
        for file in &waiting {
            if let Some(seen) = ledger.files.get_mut(&file.path) {
                seen.left = false;
                seen.waiting = true;
                taken.insert(file.path.clone());
            }
        }
        drop(taken);
        save(&ledger);
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
        let mut ledger = Ledger::default();
        let mut pending = HashMap::new();
        let mut taken = HashSet::new();
        let mut files = vec![found("/in/old.m4a", 10, 1)];
        let mut run = |ledger: &mut Ledger, files: &[Found]| {
            let files = files.to_vec();
            scan(ledger, &watched, &mut pending, &mut taken, move |_| {
                files.clone()
            })
            .into_iter()
            .map(|(f, _)| f.path)
            .collect::<Vec<_>>()
        };
        // First look: what's there already is left alone.
        assert!(run(&mut ledger, &files).is_empty());
        assert!(ledger.files[Path::new("/in/old.m4a")].left);
        assert!(run(&mut ledger, &files).is_empty());

        // A file being copied in: growing, then settled.
        files.push(found("/in/talk.mp4", 100, 5));
        assert!(run(&mut ledger, &files).is_empty());
        files[1] = found("/in/talk.mp4", 900, 6);
        assert!(run(&mut ledger, &files).is_empty());
        assert_eq!(run(&mut ledger, &files), [PathBuf::from("/in/talk.mp4")]);
        // Not again.
        assert!(run(&mut ledger, &files).is_empty());
        assert!(run(&mut ledger, &files).is_empty());

        // Replaced by a new version: taken again once settled.
        files[1] = found("/in/talk.mp4", 950, 9);
        assert!(run(&mut ledger, &files).is_empty());
        assert_eq!(run(&mut ledger, &files), [PathBuf::from("/in/talk.mp4")]);
    }

    #[test]
    fn a_folder_taken_off_the_list_starts_afresh() {
        let mut ledger = Ledger::default();
        let mut pending = HashMap::new();
        let mut taken = HashSet::new();
        let files = vec![found("/in/a.wav", 1, 1)];
        let list = |_: &Path| files.clone();
        scan(
            &mut ledger,
            &[folder("/in")],
            &mut pending,
            &mut taken,
            list,
        );
        assert_eq!(ledger.folders, [PathBuf::from("/in")]);
        scan(&mut ledger, &[], &mut pending, &mut taken, list);
        assert!(ledger.folders.is_empty());
        // Back again: a.wav is left alone, not transcribed.
        let more = vec![found("/in/a.wav", 1, 1), found("/in/b.wav", 2, 2)];
        let list = |_: &Path| more.clone();
        let again = [folder("/in")];
        assert!(scan(&mut ledger, &again, &mut pending, &mut taken, list).is_empty());
        assert!(scan(&mut ledger, &again, &mut pending, &mut taken, list).is_empty());
        assert!(ledger.files[Path::new("/in/b.wav")].left);
    }

    #[test]
    fn a_file_the_app_quit_on_is_taken_again_and_a_finished_one_isn_t() {
        let watched = [folder("/in")];
        let mut ledger = Ledger::default();
        let mut pending = HashMap::new();
        let mut taken = HashSet::new();
        let files = vec![found("/in/a.wav", 1, 1), found("/in/b.wav", 2, 2)];
        let list = |_: &Path| files.clone();
        let paths = |take: Vec<(Found, WatchedFolder)>| {
            let mut paths: Vec<PathBuf> = take.into_iter().map(|(f, _)| f.path).collect();
            paths.sort();
            paths
        };
        // The folder is known and empty; two files arrive and settle.
        ledger.folders.push(PathBuf::from("/in"));
        assert!(scan(&mut ledger, &watched, &mut pending, &mut taken, list).is_empty());
        assert_eq!(
            paths(scan(&mut ledger, &watched, &mut pending, &mut taken, list)),
            [PathBuf::from("/in/a.wav"), PathBuf::from("/in/b.wav")]
        );
        // In the queue: not taken twice by the same run.
        assert!(scan(&mut ledger, &watched, &mut pending, &mut taken, list).is_empty());
        // a.wav became a note; the app quit before b.wav did.
        ledger
            .files
            .get_mut(Path::new("/in/a.wav"))
            .unwrap()
            .waiting = false;
        let (mut pending, mut taken) = (HashMap::new(), HashSet::new());
        assert_eq!(
            paths(scan(&mut ledger, &watched, &mut pending, &mut taken, list)),
            [PathBuf::from("/in/b.wav")]
        );
        assert!(scan(&mut ledger, &watched, &mut pending, &mut taken, list).is_empty());
        // A ledger from before this was kept reads as nothing waiting.
        let old: Ledger = serde_json::from_str(
            r#"{"folders":["/in"],"files":{"/in/a.wav":{"size":1,"modified":1,"left":false}}}"#,
        )
        .unwrap();
        assert!(!old.files[Path::new("/in/a.wav")].waiting);
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
