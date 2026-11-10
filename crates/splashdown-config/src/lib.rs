//! Configuration file, secrets and on-disk locations.

pub mod import;
mod presets;
mod secrets;
mod settings;
mod transforms;

pub use presets::{
    AppContext, AppRule, CleanupLevel, DEFAULT_PRESET_ID, InsertMode, Preset, PresetModel, Tone,
    WORD_FOR_WORD_ID, builtin_presets, match_preset, preset_from_file, preset_to_file, style_kind,
    translate_instruction,
};
pub use secrets::{
    KeyName, KeySource, Secrets, delete_key, key_source, migrate_legacy_keys, store_key,
};
pub use settings::{
    ActionSettings, AiChoice, AiProvider, AiSettings, Budget, CaptureTo, Covers,
    DEFAULT_SOUND_VOLUME, Day, Destination, GeneralSettings, Hook, HookEvent, HookTarget,
    HotkeySettings, LOCAL_FAST_MS, LOCAL_SLOW_MS, LoadError, MOST_TRANSLATE_TARGETS,
    MeetingSummary, MyAction, MyActionKind, NotesSettings, OtherAudio, PutIn, Replacement,
    ResolveError, Schedule, SearchEngine, Settings, Situation, SpeechChoice, SpeechProvider,
    SpeechSettings, TEXT_SLOT, WatchedFolder, defaults, is_local_url, replacement_problem,
};
pub use transforms::{
    NOTHING_TO_DO, Place, Transform, builtin_transforms, is_nothing_to_do, tidy_answer,
};

use std::path::PathBuf;

const APP_DIR: &str = "Splashdown";
/// The folder name before the app was renamed to Splashdown.
const LEGACY_APP_DIR: &str = "just-say-it";

/// Where Splashdown keeps its files on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `config.toml` and `history.jsonl`.
    pub config_dir: PathBuf,
    /// Downloaded models: speech, the built-in text model, voices.
    pub models_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl Paths {
    /// Resolves the per-user locations, or `None` if the home directory
    /// can't be determined.
    pub fn resolve() -> Option<Self> {
        Self::for_app(APP_DIR)
    }

    fn for_app(app_dir: &str) -> Option<Self> {
        let base = directories::BaseDirs::new()?;

        // macOS: ~/Library/Application Support, ~/Library/Caches, ~/Library/Logs
        // Windows: %APPDATA%, %LOCALAPPDATA%, %LOCALAPPDATA%\Splashdown\logs
        let config_dir = base.config_dir().join(app_dir);
        let models_dir = base.cache_dir().join(app_dir).join("models");
        let logs_dir = if cfg!(target_os = "macos") {
            base.home_dir().join("Library/Logs").join(app_dir)
        } else {
            base.data_local_dir().join(app_dir).join("logs")
        };

        Some(Self {
            config_dir,
            models_dir,
            logs_dir,
        })
    }

    /// Moves what the app kept under its former name ("just-say-it") to
    /// today's folders: settings and history, downloaded models, logs. A
    /// folder moves only while the new one doesn't exist, so this runs once.
    /// Returns the folders that moved.
    pub fn migrate_legacy(&self) -> Vec<PathBuf> {
        let Some(old) = Self::for_app(LEGACY_APP_DIR) else {
            return Vec::new();
        };
        let cache = |p: &Paths| p.models_dir.parent().map(PathBuf::from);
        let mut moves = vec![
            (old.config_dir.clone(), self.config_dir.clone()),
            (old.logs_dir.clone(), self.logs_dir.clone()),
        ];
        if let (Some(from), Some(to)) = (cache(&old), cache(self)) {
            moves.push((from, to));
        }
        move_folders(&moves)
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// What cloud providers have cost, per day and month.
    pub fn spend_file(&self) -> PathBuf {
        self.config_dir.join("spend.json")
    }

    pub fn history_file(&self) -> PathBuf {
        self.config_dir.join("history.jsonl")
    }

    /// Where the mini bar was last dragged to.
    pub fn mini_bar_file(&self) -> PathBuf {
        self.config_dir.join("mini-bar.json")
    }

    /// The running app's control socket (macOS), which `splashdown-cli` and
    /// `splashdown-mcp` talk to. Windows uses a named pipe instead.
    pub fn control_socket(&self) -> PathBuf {
        self.config_dir.join("control.sock")
    }
}

/// Renames each `from` to `to` when `from` exists and `to` doesn't.
fn move_folders(moves: &[(PathBuf, PathBuf)]) -> Vec<PathBuf> {
    moves
        .iter()
        .filter(|(from, to)| from.is_dir() && !to.exists())
        .filter(|(from, to)| {
            to.parent()
                .is_none_or(|p| std::fs::create_dir_all(p).is_ok())
                && std::fs::rename(from, to).is_ok()
        })
        .map(|(_, to)| to.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_folders_move_once() {
        let root = std::env::temp_dir().join(format!("splashdown-migrate-{}", std::process::id()));
        let old = root.join("old");
        let new = root.join("nested/new");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("config.toml"), "x").unwrap();

        let moved = move_folders(&[(old.clone(), new.clone())]);
        assert_eq!(moved, vec![new.clone()]);
        assert!(new.join("config.toml").exists() && !old.exists());

        // An existing new folder is never overwritten.
        std::fs::create_dir_all(&old).unwrap();
        assert!(move_folders(&[(old.clone(), new.clone())]).is_empty());
        assert!(old.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolves_app_specific_dirs() {
        let paths = Paths::resolve().expect("home dir");
        for dir in [&paths.config_dir, &paths.models_dir, &paths.logs_dir] {
            assert!(
                dir.components().any(|c| c.as_os_str() == APP_DIR),
                "{dir:?} should contain {APP_DIR}"
            );
        }
        assert!(paths.config_file().ends_with("config.toml"));
    }
}
