//! The app's own data files, kept in the settings folder beside
//! `settings.toml` and never in the notes folder: watched folders'
//! ledger, subtitle timings, saved voices. JSON, read leniently (a missing
//! or damaged file reads as empty) and written whole.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// `name` in the settings folder.
pub fn path(name: &str) -> Result<PathBuf, String> {
    Ok(crate::engine::paths()?.config_dir.join(name))
}

/// `<folder>/<id>.json` in the settings folder, for a note's data. The id
/// comes from a note the user can edit, so it's checked to be one
/// Splashdown wrote ("1730000000-ab12"): nothing that leaves the folder.
pub fn for_note(folder: &str, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("This note's id isn't one Splashdown wrote.".into());
    }
    Ok(path(folder)?.join(format!("{id}.json")))
}

/// The file's value, or the default when it's missing or unreadable.
pub fn read<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Writes `value` to `path`, making its folder if needed.
pub fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_note_id_cant_leave_the_folder() {
        assert!(super::for_note("speakers", "../settings").is_err());
        assert!(super::for_note("speakers", "").is_err());
    }
}
