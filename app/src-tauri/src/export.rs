//! Exporting the history, and moving settings between computers (spec §9,
//! §8.5). There's no save dialog (no dialog plugin): files go to Downloads
//! and are shown in Finder / File Explorer. Imports are read by the
//! Settings window (a file input) and arrive here as text, so the app needs
//! no file access for them.

use std::path::PathBuf;

use serde::Serialize;
use splashdown_config::Settings;
use tauri::{AppHandle, State};

use crate::commands::{AppState, Saved};
use crate::history::{self, ExportFormat};

/// A new file in Downloads named `<name> YYYY-MM-DD.<extension>`, with a
/// number added if one already exists.
fn downloads_file(name: &str, extension: &str) -> Result<PathBuf, String> {
    let dir = directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(PathBuf::from))
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join("Downloads")))
        .ok_or("couldn't find the Downloads folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = format!("{name} {}", chrono::Local::now().format("%Y-%m-%d"));
    let mut path = dir.join(format!("{stem}.{extension}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} {n}.{extension}"));
        n += 1;
    }
    Ok(path)
}

/// Writes `text` to a new file in Downloads and shows it. Returns the path.
fn save_to_downloads(name: &str, extension: &str, text: &str) -> Result<String, String> {
    let path = downloads_file(name, extension)?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    if let Err(e) = splashdown_platform::reveal_in_file_manager(&path) {
        tracing::warn!("couldn't show the exported file: {e}");
    }
    Ok(path.display().to_string())
}

/// Saves every history entry (time, app, mode, what was said, the final
/// text) to Downloads as JSON or Markdown.
#[tauri::command]
pub fn export_history(format: ExportFormat, state: State<'_, AppState>) -> Result<String, String> {
    let entries = state.history.list("");
    if entries.is_empty() {
        return Err("There's nothing in the history to export.".into());
    }
    let text = history::export(&entries, format);
    save_to_downloads("Splashdown history", format.extension(), &text)
}

/// Saves the settings (`config.toml`, with presets, snippets and
/// vocabulary) to Downloads, for importing on another computer. API keys
/// aren't in it: they live in the keychain.
#[tauri::command]
pub fn export_settings(state: State<'_, AppState>) -> Result<String, String> {
    let text = format!(
        "# Splashdown settings, exported {}. API keys aren't included.\n\n{}",
        chrono::Local::now().format("%Y-%m-%d"),
        state.settings().to_toml()?
    );
    save_to_downloads("Splashdown settings", "toml", &text)
}

/// What an imported file holds, for the confirmation.
#[derive(Serialize)]
pub struct ImportSummary {
    presets: Option<usize>,
    snippets: usize,
    vocabulary: usize,
}

fn parse(text: &str) -> Result<Settings, String> {
    Settings::from_toml(text).map_err(|e| format!("That isn't a Splashdown settings file: {e}"))
}

/// Checks an exported settings file before it's imported.
#[tauri::command]
pub fn check_settings_file(text: String) -> Result<ImportSummary, String> {
    let settings = parse(&text)?;
    Ok(ImportSummary {
        presets: settings.presets.as_ref().map(Vec::len),
        snippets: settings.snippets.len(),
        vocabulary: settings.general.vocabulary.len(),
    })
}

/// Replaces the settings with an exported file's and applies them, the same
/// way as saving in Settings. Whether this computer's setup guide was
/// finished stays as it is: permissions are per computer.
#[tauri::command]
pub async fn import_settings(
    text: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Saved, String> {
    let mut settings = parse(&text)?;
    settings.general.onboarded = state.settings().general.onboarded;
    crate::commands::save_settings(settings, state, app).await
}
