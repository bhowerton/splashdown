//! Where notes live: the folder the user picked (or Documents/Splashdown),
//! a fallback when it's out of reach, moving notes to a new folder, and
//! naming cloud folders (iCloud, Dropbox, …).

use super::*;
use splashdown_core::sync::Locked;

/// `Documents/Splashdown Notes`.
pub(super) fn default_folder() -> Option<PathBuf> {
    let documents = directories::UserDirs::new()
        .and_then(|d| d.document_dir().map(Path::to_path_buf))
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join("Documents")))?;
    Some(documents.join(NOTES_FOLDER_NAME))
}

/// The folder in settings, or the default.
pub(super) fn wanted_folder(state: &AppState) -> Option<PathBuf> {
    state.settings().notes.folder.or_else(default_folder)
}

/// Where notes go while the chosen folder can't be used (macOS refused
/// Documents access, a removed drive), so nothing typed is lost.
pub(super) fn fallback_folder() -> Result<PathBuf, String> {
    Ok(engine::paths()?.config_dir.join("notes"))
}

pub(super) struct Current {
    /// The folder asked for.
    pub(super) wanted: PathBuf,
    pub(super) store: Arc<Store>,
    /// Why notes are in the fallback folder instead.
    pub(super) problem: Option<String>,
    /// When the folder asked for was last tried.
    pub(super) tried: Instant,
}

/// While the fallback folder is in use, how often the chosen folder is
/// tried again (the user may have allowed access meanwhile).
pub(super) const RETRY_FOLDER: Duration = Duration::from_secs(30);

pub(super) fn current_slot() -> &'static Mutex<Option<Current>> {
    static CURRENT: OnceLock<Mutex<Option<Current>>> = OnceLock::new();
    CURRENT.get_or_init(Mutex::default)
}

/// The store for the notes folder, set up the first time: created
/// lazily (never at launch, so the Documents prompt only appears when
/// Notes is used), with the old scratchpad's notes moved in.
pub(super) fn store(state: &AppState) -> Result<(Arc<Store>, Option<String>), String> {
    store_for(state.settings().notes.folder)
}

/// The store for `folder` (unset: the default), as `store`.
pub(crate) fn store_for(folder: Option<PathBuf>) -> Result<(Arc<Store>, Option<String>), String> {
    let wanted = folder
        .or_else(default_folder)
        .ok_or("Couldn't find your Documents folder.")?;
    let mut slot = current_slot().locked();
    // The same store each time (it remembers which notes need a version
    // kept), trying the chosen folder again now and then while it can't
    // be used.
    if let Some(c) = slot
        .as_ref()
        .filter(|c| c.wanted == wanted && (c.problem.is_none() || c.tried.elapsed() < RETRY_FOLDER))
    {
        return Ok((c.store.clone(), c.problem.clone()));
    }
    let chosen = Store::new(wanted.clone());
    let (store, problem) = match chosen.ensure() {
        Ok(()) => {
            // Usable again: notes written to the fallback meanwhile join
            // the others.
            if let Some(old) = slot
                .as_ref()
                .filter(|c| c.wanted == wanted && c.problem.is_some())
            {
                match move_notes(&old.store, &wanted) {
                    Ok(n) => tracing::info!("moved {n} notes in from the fallback folder"),
                    Err(e) => tracing::warn!("notes stayed in the fallback folder: {e}"),
                }
            }
            (chosen, None)
        }
        Err(e) => {
            tracing::warn!("notes folder {} unusable: {e}", wanted.display());
            let fallback = Store::new(fallback_folder()?);
            fallback.ensure().map_err(|e| e.to_string())?;
            let allow = if cfg!(target_os = "macos") {
                "System Settings → Privacy & Security → Files and Folders"
            } else {
                "Windows Security → Ransomware protection → Allow an app through Controlled folder access"
            };
            let why = if e.kind() == std::io::ErrorKind::PermissionDenied {
                format!(
                    "Splashdown isn't allowed to use {}. Notes are kept in its own folder until you choose another one or allow it in {allow}.",
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
        tried: Instant::now(),
    });
    Ok((store, problem))
}

/// Moves the scratchpad's notes (`scratchpad.json`) into the folder, once:
/// the file is renamed `scratchpad.json.migrated` afterwards, not deleted.
pub(super) fn migrate_scratchpad(store: &Store) {
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

/// "Documents › Splashdown Notes", "Dropbox › Splashdown Notes", or the
/// full path outside the home folder.
pub(super) fn label(path: &Path) -> String {
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rel) if rel.components().count() > 0 => {
            let mut parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            // macOS keeps sync services' folders deep in Library, under
            // names nobody sees in Finder.
            if cfg!(target_os = "macos") && parts.len() >= 3 && parts[0] == "Library" {
                let named = match (parts[1].as_str(), parts[2].as_str()) {
                    ("Mobile Documents", "com~apple~CloudDocs") => Some((3, "iCloud Drive")),
                    ("CloudStorage", p) if p.starts_with("GoogleDrive") => {
                        let skip = if parts.get(3).is_some_and(|d| d == "My Drive") {
                            4
                        } else {
                            3
                        };
                        Some((skip, "Google Drive"))
                    }
                    ("CloudStorage", p) if p.starts_with("Dropbox") => Some((3, "Dropbox")),
                    ("CloudStorage", p) if p.starts_with("OneDrive") => Some((3, "OneDrive")),
                    ("CloudStorage", p) if p.starts_with("Box") => Some((3, "Box")),
                    _ => None,
                };
                if let Some((skip, name)) = named {
                    parts.splice(..skip, [name.to_string()]);
                }
            }
            parts.join(" › ")
        }
        _ => path.display().to_string(),
    }
}

/// The sync service that has a copy of `path`, best effort.
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

/// Moves every note, with `.splashdown/` (versions and pins) and
/// `Attachments/`, from one folder to another, never overwriting: a note
/// name that's taken gets a number. Returns how many notes moved.
pub(super) fn move_notes(from: &Store, to: &Path) -> Result<usize, String> {
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
    for dir in [".splashdown", store::ATTACHMENTS_DIR] {
        let source = from.root().join(dir);
        if source.is_dir()
            && let Err(e) = merge_into(&source, &to.join(dir))
        {
            tracing::warn!("some of {dir} stayed in the old notes folder: {e}");
        }
    }
    Ok(moved)
}

/// Moves `from` to `to`, merging into a folder that's already there. A
/// file that exists on both sides keeps the one at `to`.
pub(super) fn merge_into(from: &Path, to: &Path) -> std::io::Result<()> {
    if !to.exists() && std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)?.flatten() {
            merge_into(&entry.path(), &to.join(entry.file_name()))?;
        }
        // Left behind only if something in it was kept at `to`.
        let _ = std::fs::remove_dir(from);
    } else if !to.exists() {
        // Another volume: copy, then remove the original.
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)?;
    }
    Ok(())
}

/// A sync service on this computer that notes could be kept in, for
/// Settings' "Keep notes in Dropbox" buttons.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloudPlace {
    /// "Dropbox".
    pub service: String,
    /// The account, when there's more than one of a service ("ben@example.com").
    pub account: Option<String>,
    /// Where the notes would go: `Splashdown Notes` at the top of it.
    pub folder: String,
    pub label: String,
}

/// The sync services this computer has (best effort), each once.
pub fn cloud_places() -> Vec<CloudPlace> {
    let mut places: Vec<CloudPlace> = sync_roots()
        .into_iter()
        .map(|(service, account, root)| {
            let folder = root.join(NOTES_FOLDER_NAME);
            CloudPlace {
                service,
                account,
                label: label(&folder),
                folder: folder.display().to_string(),
            }
        })
        .collect();
    // An account is only worth showing when there's more than one.
    let counts = |service: &str| places.iter().filter(|p| p.service == service).count();
    let single: Vec<bool> = places.iter().map(|p| counts(&p.service) == 1).collect();
    for (place, single) in places.iter_mut().zip(single) {
        if single {
            place.account = None;
        }
    }
    places
}

/// A sync service's folder on this computer, for finding the same file
/// on someone else's (a team's vocabulary, `shared_words::invite`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncBase {
    /// "Dropbox".
    pub service: String,
    pub dir: PathBuf,
    /// What this folder is called inside the service, when it's one part
    /// of it kept somewhere of its own: Google Drive's "My Drive" as a
    /// folder in the home folder. A path in the service that starts with
    /// this name is inside `dir`.
    pub inside: Option<String>,
}

/// Each sync service's folders on this computer. Google Drive's is the
/// folder that holds both My Drive and Shared drives, where there is one.
pub fn sync_bases() -> Vec<SyncBase> {
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    let mut bases: Vec<SyncBase> = Vec::new();
    for (service, _, root) in sync_roots() {
        let my_drive = service == "Google Drive" && root.ends_with("My Drive");
        let base = match root.parent() {
            // Kept in the home folder: only My Drive is there.
            Some(parent) if my_drive && Some(parent) == home.as_deref() => SyncBase {
                service,
                dir: root,
                inside: Some("My Drive".into()),
            },
            Some(parent) if my_drive => SyncBase {
                service,
                dir: parent.to_path_buf(),
                inside: None,
            },
            _ => SyncBase {
                service,
                dir: root,
                inside: None,
            },
        };
        bases.push(base);
    }
    // Shared drives are here even when My Drive is kept in the home folder.
    for dir in google_drive_folders() {
        if !bases.iter().any(|b| b.dir == dir) {
            bases.push(SyncBase {
                service: "Google Drive".into(),
                dir,
                inside: None,
            });
        }
    }
    bases
}

/// Google Drive's own folders on a Mac, one per account
/// (`~/Library/CloudStorage/GoogleDrive-ben@example.com`).
fn google_drive_folders() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = std::fs::read_dir(home.join("Library/CloudStorage"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("GoogleDrive-"))
        })
        .collect();
    found.sort();
    found
}

/// Where each sync service keeps its files here: the service, the account
/// where the folder's name gives one, and the folder. Only folders that
/// exist, each once.
fn sync_roots() -> Vec<(String, Option<String>, PathBuf)> {
    let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) else {
        return Vec::new();
    };
    let mut roots: Vec<(String, Option<String>, PathBuf)> = Vec::new();
    if cfg!(target_os = "macos") {
        roots.push((
            "iCloud Drive".into(),
            None,
            home.join("Library/Mobile Documents/com~apple~CloudDocs"),
        ));
        // Dropbox, OneDrive, Google Drive and Box as File Provider folders:
        // `GoogleDrive-ben@example.com`, `OneDrive-Personal`, `Dropbox`.
        let storage = home.join("Library/CloudStorage");
        let mut found: Vec<PathBuf> = std::fs::read_dir(&storage)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        found.sort();
        for dir in found {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (provider, account) = match name.split_once('-') {
                Some((p, a)) => (p.to_string(), Some(a.to_string())),
                None => (name.clone(), None),
            };
            let (service, root) = match provider.as_str() {
                "Dropbox" => ("Dropbox", dir),
                "OneDrive" => ("OneDrive", dir),
                "GoogleDrive" => ("Google Drive", dir.join("My Drive")),
                "Box" => ("Box", dir),
                _ => continue,
            };
            roots.push((service.into(), account, root));
        }
        roots.extend(dropbox_roots(&home.join(".dropbox/info.json")));
        roots.push(("Dropbox".into(), None, home.join("Dropbox")));
        // Google Drive set to keep My Drive on this Mac ("mirror files")
        // puts it in the home folder, not with its other folders.
        if !google_drive_folders().is_empty() {
            roots.push(("Google Drive".into(), None, home.join("My Drive")));
        }
    } else {
        for name in ["iCloudDrive", "iCloud Drive"] {
            roots.push(("iCloud Drive".into(), None, home.join(name)));
        }
        for (var, account) in [
            ("OneDriveConsumer", None),
            ("OneDrive", None),
            ("OneDriveCommercial", Some("work")),
        ] {
            if let Some(dir) = std::env::var_os(var) {
                roots.push(("OneDrive".into(), account.map(String::from), dir.into()));
            }
        }
        for var in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(dir) = std::env::var_os(var) {
                roots.extend(dropbox_roots(&Path::new(&dir).join("Dropbox/info.json")));
            }
        }
        roots.push(("Dropbox".into(), None, home.join("Dropbox")));
        // Google Drive for desktop: a drive letter (G: unless changed) with
        // My Drive in it, or My Drive in the home folder when mirroring.
        let google = std::env::var_os("LOCALAPPDATA")
            .is_some_and(|dir| Path::new(&dir).join("Google/DriveFS").is_dir());
        if google {
            for letter in ('D'..='Z').rev() {
                roots.push((
                    "Google Drive".into(),
                    None,
                    PathBuf::from(format!("{letter}:\\My Drive")),
                ));
            }
            roots.push(("Google Drive".into(), None, home.join("My Drive")));
        }
        roots.push(("Box".into(), None, home.join("Box")));
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    roots.retain(|(_, _, root)| {
        let new = root.is_dir() && !seen.contains(root);
        if new {
            seen.push(root.clone());
        }
        new
    });
    roots
}

/// The folder notes get inside a sync service, as in Documents.
const NOTES_FOLDER_NAME: &str = "Splashdown Notes";

/// Dropbox's own record of where its folders are (`info.json`): personal
/// and business.
fn dropbox_roots(info: &Path) -> Vec<(String, Option<String>, PathBuf)> {
    let Ok(text) = std::fs::read_to_string(info) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    ["personal", "business"]
        .into_iter()
        .filter_map(|kind| {
            let path = json.get(kind)?.get("path")?.as_str()?;
            Some((
                "Dropbox".to_string(),
                (kind == "business").then(|| "work".to_string()),
                PathBuf::from(path),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn sync_folders_are_named_as_people_know_them() {
        let home = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf();
        let at = |rel: &str| label(&home.join(rel));
        assert_eq!(
            at("Documents/Splashdown Notes"),
            "Documents › Splashdown Notes"
        );
        assert_eq!(
            at("Library/Mobile Documents/com~apple~CloudDocs/Splashdown Notes"),
            "iCloud Drive › Splashdown Notes"
        );
        assert_eq!(
            at("Library/CloudStorage/GoogleDrive-a@example.com/My Drive/Splashdown Notes"),
            "Google Drive › Splashdown Notes"
        );
        assert_eq!(
            at("Library/CloudStorage/Dropbox/Work/Notes"),
            "Dropbox › Work › Notes"
        );
        assert_eq!(
            at("Library/Application Support/x"),
            "Library › Application Support › x"
        );
    }

    #[test]
    fn dropbox_says_where_its_folders_are() {
        let dir = std::env::temp_dir().join(format!("sd-dropbox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let info = dir.join("info.json");
        std::fs::write(
            &info,
            r#"{"personal": {"path": "/x/Dropbox", "host": 1}, "business": {"path": "/x/Dropbox (Acme)"}}"#,
        )
        .unwrap();
        let roots = dropbox_roots(&info);
        assert_eq!(
            roots,
            [
                ("Dropbox".to_string(), None, PathBuf::from("/x/Dropbox")),
                (
                    "Dropbox".to_string(),
                    Some("work".to_string()),
                    PathBuf::from("/x/Dropbox (Acme)")
                ),
            ]
        );
        assert!(dropbox_roots(&dir.join("missing.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
