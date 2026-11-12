//! Notes said on an iPhone. The Splashdown Note shortcut turns //! what was
//! said into text and saves it as a file in iCloud Drive's Shortcuts
//! folder, named "Splashdown" and when it was said. Here each one is added to the
//! capture note (Inbox or today's note) at that time, then deleted.
//!
//! The shortcut can only save quietly to iCloud Drive, so that's where
//! notes wait, whichever folder or sync service the notes are kept in.
//!
//! Only while Settings → Notes → From your iPhone is on: looking in iCloud
//! Drive makes macOS ask, so it starts with the user's own click.

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use std::collections::HashSet;
use std::sync::Condvar;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// How often the folder is looked at.
const EVERY: Duration = Duration::from_secs(10);
/// A file is left until it's been still this long, in case it's still
/// arriving.
const SETTLED: Duration = Duration::from_secs(3);
/// How the shortcut's files are named: "Splashdown 2026-11-08
/// 14-03-22.txt". Other files in the Shortcuts folder are left alone.
const PREFIX: &str = "Splashdown";
/// The Shortcuts app's folder in iCloud Drive.
const SHORTCUTS_CONTAINER: &str = "iCloud~is~workflow~my~workflows";

/// Whether a note from the iPhone has been added since the app started.
static ARRIVED: AtomicBool = AtomicBool::new(false);
/// Whether macOS refused the last look in the folder.
static REFUSED: AtomicBool = AtomicBool::new(false);
/// Wakes the watcher for a look now rather than at its next turn.
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Where the shortcut's files can arrive on this computer, whether or not
/// they exist yet: the Shortcuts app's folder in iCloud Drive.
fn folders() -> Vec<PathBuf> {
    let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) else {
        return Vec::new();
    };
    if cfg!(target_os = "macos") {
        vec![
            home.join("Library/Mobile Documents")
                .join(SHORTCUTS_CONTAINER)
                .join("Documents"),
        ]
    } else {
        // iCloud for Windows: iCloudDrive (from the Microsoft Store) or
        // iCloud Drive, with the Shortcuts folder by either name.
        icloud_roots(&home)
            .into_iter()
            .flat_map(|root| {
                [
                    root.join("Shortcuts"),
                    root.join(SHORTCUTS_CONTAINER),
                    root.join(SHORTCUTS_CONTAINER).join("Documents"),
                ]
            })
            .collect()
    }
}

/// iCloud Drive's folder on this computer, by each name it can have.
fn icloud_roots(home: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![home.join("Library/Mobile Documents/com~apple~CloudDocs")]
    } else {
        vec![home.join("iCloudDrive"), home.join("iCloud Drive")]
    }
}

/// Whether `name` is one of the shortcut's notes.
fn is_note(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(PREFIX) else {
        return false;
    };
    // "Splashdown 2026-…", "Splashdown.txt", "Splashdown 2.txt", not
    // "Splashdowns.txt".
    let after = rest.chars().next();
    let plain = matches!(after, None | Some(' ' | '.'));
    let text = Path::new(name)
        .extension()
        .is_none_or(|e| e.eq_ignore_ascii_case("txt"));
    plain && text
}

/// When a note was said: from its name ("Splashdown 2026-11-08
/// 14-03-22.txt", or "… 2.txt" when two came the same second), else the
/// file's time.
fn said_at(path: &Path, modified: std::time::SystemTime) -> DateTime<Local> {
    path.file_stem()
        .map(|s| s.to_string_lossy())
        .and_then(|stem| {
            let stamp = stem.strip_prefix(PREFIX)?.trim_start().get(..19)?;
            let naive = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H-%M-%S").ok()?;
            Local.from_local_datetime(&naive).earliest()
        })
        .unwrap_or_else(|| modified.into())
}

/// The notes waiting in `folder`, oldest first, leaving any still
/// arriving. Files iCloud hasn't downloaded yet (`.name.txt.icloud`, before
/// macOS 14) are asked for, to be taken on a later look.
fn waiting(
    folder: &Path,
    now: std::time::SystemTime,
    asked: &mut HashSet<PathBuf>,
) -> Vec<(PathBuf, DateTime<Local>)> {
    let mut found = Vec::new();
    for entry in list(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(real) = name
            .strip_prefix('.')
            .and_then(|n| n.strip_suffix(".icloud"))
            .filter(|n| is_note(n))
        {
            let real = folder.join(real);
            if asked.insert(real.clone()) {
                download(&real);
            }
            continue;
        }
        if !is_note(&name) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if now.duration_since(modified).unwrap_or_default() < SETTLED {
            continue;
        }
        found.push((path.clone(), said_at(&path, modified)));
    }
    found.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    found
}

/// What's in `folder`, remembering whether macOS refused to say (as
/// opposed to the folder not being there yet).
fn list(folder: &Path) -> Option<std::fs::ReadDir> {
    match std::fs::read_dir(folder) {
        Ok(entries) => {
            REFUSED.store(false, Ordering::Relaxed);
            Some(entries)
        }
        Err(e) => {
            let refused = e.kind() == std::io::ErrorKind::PermissionDenied;
            REFUSED.store(refused, Ordering::Relaxed);
            if refused {
                tracing::warn!("not allowed to look for notes from the iPhone: {e}");
            }
            None
        }
    }
}

/// Waits for the next look: `EVERY`, or sooner when woken.
fn wait() {
    let (woken, wake) = &WAKE;
    let mut woken = woken.locked();
    if !*woken {
        woken = wake
            .wait_timeout(woken, EVERY)
            .map(|(guard, _)| guard)
            .unwrap_or_else(|e| e.into_inner().0);
    }
    *woken = false;
}

/// Asks iCloud to download a file it's only keeping a placeholder for.
fn download(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let asked = std::process::Command::new("/usr/bin/brctl")
            .arg("download")
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if let Err(e) = asked {
            tracing::warn!("couldn't ask iCloud for a note from the iPhone: {e}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// What a note file says: its text, without a byte order mark.
fn text_of(raw: &str) -> &str {
    raw.trim_start_matches('\u{feff}').trim()
}

/// Adds the note in `file` to the capture note, at `when`, and deletes the
/// file: what it said. A note that's empty is only deleted.
fn take(settings: &NotesSettings, file: &Path, when: DateTime<Local>) -> Result<String, String> {
    let raw = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let text = text_of(&raw);
    if !text.is_empty() {
        let (store, _) = store_for(settings.folder.clone())?;
        // The last note opened here means nothing to someone on their
        // phone: those go to the Inbox.
        let target = match super::capture::capture_target(settings) {
            Target::Last => Target::Inbox,
            target => target,
        };
        store
            .append(&target, text, when)
            .map_err(|e| e.to_string())?;
    }
    std::fs::remove_file(file).map_err(|e| e.to_string())?;
    Ok(text.to_string())
}

/// Starts looking for notes from the iPhone, for as long as the app runs.
pub fn watch_phone(app: &AppHandle) {
    let app = app.clone();
    let started = std::thread::Builder::new()
        .name("phone-notes".into())
        .spawn(move || {
            let mut asked = HashSet::new();
            // Added but not deleted (a file iCloud wouldn't let go of):
            // never added twice while the app runs.
            let mut taken = HashSet::new();
            loop {
                wait();
                let Some(state) = app.try_state::<AppState>() else {
                    continue;
                };
                let settings = state.settings().notes;
                // Off: iCloud Drive isn't touched at all.
                if !settings.from_phone() {
                    REFUSED.store(false, Ordering::Relaxed);
                    continue;
                }
                let now = std::time::SystemTime::now();
                let mut added = 0;
                for folder in folders() {
                    for (file, when) in waiting(&folder, now, &mut asked) {
                        if taken.contains(&file) {
                            continue;
                        }
                        match take(&settings, &file, when) {
                            Ok(text) => {
                                added += 1;
                                if !text.is_empty() {
                                    crate::hooks::fire(
                                        &app,
                                        crate::hooks::Happened::phone(&text, when),
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!("a note from the iPhone: {e}");
                                // Added, but still there: don't add it again.
                                if file.exists() {
                                    taken.insert(file);
                                }
                            }
                        }
                    }
                }
                if added > 0 {
                    ARRIVED.store(true, Ordering::Relaxed);
                    tracing::info!("{added} note(s) from the iPhone added");
                }
            }
        });
    if let Err(e) = started {
        tracing::error!("couldn't start looking for notes from the iPhone: {e}");
    }
}

/// Settings: whether this computer has iCloud Drive, whether a note from
/// the iPhone has come since the app started, and whether macOS refused
/// the last look for them.
#[derive(Debug, Serialize)]
pub struct Phone {
    icloud: bool,
    used: bool,
    refused: bool,
}

/// Only looks whether iCloud Drive's folder is there, which macOS doesn't
/// ask about; nothing in it is read.
#[tauri::command]
pub fn notes_phone() -> Phone {
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    Phone {
        icloud: home.is_some_and(|h| icloud_roots(&h).iter().any(|r| r.is_dir())),
        used: ARRIVED.load(Ordering::Relaxed),
        refused: REFUSED.load(Ordering::Relaxed),
    }
}

/// Settings → From your iPhone was just turned on: looks in the folder now,
/// so macOS asks right after the click, and wakes the watcher. Waits for
/// the answer, off the main thread.
#[tauri::command]
pub async fn notes_phone_look() -> Phone {
    let _ = tauri::async_runtime::spawn_blocking(|| {
        for folder in folders() {
            let _ = list(&folder);
        }
    })
    .await;
    let (woken, wake) = &WAKE;
    *woken.locked() = true;
    wake.notify_one();
    notes_phone()
}

/// Settings → Get the shortcut: the page with it and how to set it up.
#[tauri::command]
pub fn notes_phone_page() -> Result<(), String> {
    splashdown_platform::open_url("https://splashdown.app/iphone/").map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_dated_by_its_name_or_else_its_file() {
        let file_time = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let at = said_at(Path::new("x/Splashdown 2026-11-08 14-03-22.txt"), file_time);
        assert_eq!(
            at.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-11-08 14:03:22"
        );
        // Two in the same second: Shortcuts numbers the second.
        let at = said_at(
            Path::new("x/Splashdown 2026-11-08 14-03-22 2.txt"),
            file_time,
        );
        assert_eq!(at.format("%H:%M").to_string(), "14:03");
        let at = said_at(Path::new("x/Splashdown.txt"), file_time);
        assert_eq!(at, DateTime::<Local>::from(file_time));
        assert!(is_note("Splashdown 2026-11-08 14-03-22.txt"));
        assert!(is_note("Splashdown.txt"));
        assert!(is_note("Splashdown 2"));
        assert!(!is_note("Splashdowns.txt"));
        assert!(!is_note("Splashdown.jpg"));
        assert!(!is_note("Groceries.txt"));
        assert!(!is_note(".Splashdown.txt"));
        assert_eq!(text_of("\u{feff} Call Sam back. \n"), "Call Sam back.");
    }

    #[test]
    fn notes_are_taken_oldest_first_once_settled_and_added_with_their_time() {
        let base = std::env::temp_dir().join(format!("sd-phone-{}", std::process::id()));
        let inbox = base.join("phone");
        let notes = base.join("notes");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("Splashdown 2026-11-08 09-15-00.txt"), "Second.").unwrap();
        std::fs::write(inbox.join("Splashdown 2026-11-08 08-05-00.txt"), "First.").unwrap();
        std::fs::write(inbox.join("Splashdown 2026-11-08 10-00-00.txt"), "  ").unwrap();
        std::fs::write(inbox.join("Splashdown.jpg"), "x").unwrap();
        std::fs::write(inbox.join("Groceries.txt"), "x").unwrap();
        std::fs::write(inbox.join(".Splashdown.txt"), "x").unwrap();
        let mut asked = HashSet::new();
        // Just written: still arriving.
        let now = std::time::SystemTime::now();
        assert!(waiting(&inbox, now, &mut asked).is_empty());
        let later = now + Duration::from_secs(10);
        let found = waiting(&inbox, later, &mut asked);
        let names: Vec<_> = found
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "Splashdown 2026-11-08 08-05-00.txt",
                "Splashdown 2026-11-08 09-15-00.txt",
                "Splashdown 2026-11-08 10-00-00.txt"
            ]
        );
        let settings = NotesSettings {
            folder: Some(notes.clone()),
            ..Default::default()
        };
        for (file, when) in found {
            take(&settings, &file, when).unwrap();
        }
        let inbox_note = std::fs::read_to_string(notes.join("Inbox.md")).unwrap();
        assert_eq!(
            inbox_note,
            "# Inbox\n\n## 2026-11-08\n\n**08:05** First.\n\n**09:15** Second.\n"
        );
        let left: Vec<_> = std::fs::read_dir(&inbox)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 3, "{left:?}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
