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

/// The file's value, or the default when it's missing or unreadable. A
/// file that's there but can't be read is logged: what it held is about to
/// be replaced by what's saved next.
pub fn read<T: DeserializeOwned + Default>(path: &Path) -> T {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            tracing::warn!("couldn't read {}: {e}", file_name(path));
            return T::default();
        }
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!("{} is damaged, starting it afresh: {e}", file_name(path));
        T::default()
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// Writes `value` to `path`, making its folder if needed.
pub fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
    write_text(path, &json).map_err(|e| e.to_string())
}

/// Writes `text` to `path`, making its folder if needed, through a file
/// beside it that then takes its place: a crash or a full disk part way
/// leaves what was there, never half of the new.
pub fn write_text(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let beside = path.with_file_name(name);
    let written = std::fs::File::create(&beside).and_then(|mut file| {
        file.write_all(text.as_bytes())?;
        file.sync_all()
    });
    match written.and_then(|()| std::fs::rename(&beside, path)) {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = std::fs::remove_file(&beside);
            // Something holds the file for a moment (a virus scanner, on
            // Windows): written in place instead.
            std::fs::write(path, text)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_file_is_replaced_whole_and_a_damaged_one_reads_as_empty() {
        let dir = std::env::temp_dir().join(format!("splashdown-data-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("deep").join("ledger.json");
        super::write(&path, &vec![1, 2, 3]).unwrap();
        super::write(&path, &vec![4]).unwrap();
        assert_eq!(super::read::<Vec<u32>>(&path), [4]);
        assert!(!path.with_file_name("ledger.json.tmp").exists());
        std::fs::write(&path, "[4, ").unwrap();
        assert!(super::read::<Vec<u32>>(&path).is_empty());
        assert!(super::read::<Vec<u32>>(&dir.join("missing.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_note_id_cant_leave_the_folder() {
        assert!(super::for_note("speakers", "../settings").is_err());
        assert!(super::for_note("speakers", "").is_err());
    }
}
