//! Where notes live: the folder the user picked (or Documents/Splashdown),
//! a fallback when it's out of reach, moving notes to a new folder, and
//! naming cloud folders (iCloud, Dropbox, …).

use super::*;

/// `Documents/Splashdown Notes`.
pub(super) fn default_folder() -> Option<PathBuf> {
    let documents = directories::UserDirs::new()
        .and_then(|d| d.document_dir().map(Path::to_path_buf))
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join("Documents")))?;
    Some(documents.join("Splashdown Notes"))
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
    let mut slot = current_slot().lock().unwrap_or_else(|e| e.into_inner());
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

/// "Documents › Splashdown Notes", or the full path outside the home folder.
pub(super) fn label(path: &Path) -> String {
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
