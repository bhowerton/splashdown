//! Exporting the history, and moving settings between computers. There's
//! no save dialog (no dialog plugin): files go to Downloads
//! and are shown in Finder / File Explorer. Imports are read by the
//! Settings window (a file input) and arrive here as text, so the app needs
//! no file access for them.

use std::path::PathBuf;

use serde::Serialize;
use splashdown_config::Settings;
use tauri::{AppHandle, State};

use crate::history::{self, ExportFormat};
use crate::snapshot::Saved;
use crate::state::AppState;

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
    /// What the file would run on this computer or send text to, a line
    /// each: the part worth reading before importing someone else's file.
    outside: Vec<String>,
}

/// What `settings` would run on this computer, or send what's said to.
fn outside(settings: &Settings) -> Vec<String> {
    use splashdown_config::MyActionKind;
    let mut lines: Vec<String> = settings.hooks.iter().map(crate::hooks::summary).collect();
    for action in &settings.actions.mine {
        match &action.does {
            MyActionKind::Program { path, args } => lines.push(format!(
                "Runs {} when you say “{}”",
                std::iter::once(path.as_str())
                    .chain(args.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" "),
                action.name
            )),
            MyActionKind::Keys { keys } => {
                lines.push(format!("Presses {keys} when you say “{}”", action.name))
            }
            MyActionKind::Link { .. } => {}
        }
    }
    if settings.ai.provider == splashdown_config::AiProvider::Custom
        && let Some(url) = settings.ai.base_url.as_deref()
        && !splashdown_config::is_local_url(url)
    {
        lines.push(format!(
            "Sends what you dictate, and your custom server's key if you saved one, to {url}"
        ));
    }
    lines
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
        outside: outside(&settings),
    })
}

/// Saves one preset to Downloads as a file to share. Returns the path.
#[tauri::command]
pub fn export_preset(preset: splashdown_config::Preset) -> Result<String, String> {
    let text = splashdown_config::preset_to_file(&preset)?;
    let name: String = preset
        .name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .collect();
    save_to_downloads(&format!("Splashdown preset {}", name.trim()), "toml", &text)
}

/// Reads a shared preset file (from the Settings window's file input).
/// The page gives it an id and adds it.
#[tauri::command]
pub fn read_preset_file(text: String) -> Result<splashdown_config::Preset, String> {
    splashdown_config::preset_from_file(&text)
}

/// What importing words from another app's file would add.
#[derive(Serialize)]
pub struct WordsImport {
    /// The settings with the file merged in, to save if the user agrees.
    settings: Settings,
    /// What kind of file it was: "dictionary export".
    kind: &'static str,
    vocabulary: usize,
    replacements: usize,
    snippets: usize,
    duplicates: usize,
    conflicts: usize,
    skipped: usize,
}

/// Merges another dictation app's export (vocabulary, corrections,
/// snippets) into `settings`, the Settings window's current ones, without
/// changing anything already there. Nothing is saved here.
#[tauri::command]
pub fn import_words(text: String, mut settings: Settings) -> Result<WordsImport, String> {
    let imported = splashdown_config::import::parse(&text).map_err(|e| {
        format!("Splashdown couldn't use that file: {e}. It takes a dictionary or snippets export, or a word list.")
    })?;
    let merged = splashdown_config::import::merge(&mut settings, &imported);
    Ok(WordsImport {
        settings,
        kind: imported.kind,
        vocabulary: merged.vocabulary,
        replacements: merged.replacements,
        snippets: merged.snippets,
        duplicates: merged.duplicates,
        conflicts: merged.conflicts,
        skipped: imported.skipped,
    })
}

/// Replaces the settings with an exported file's and applies them, the same
/// way as saving in Settings. Whether this computer's setup guide was
/// finished stays as it is: permissions are per computer. Nothing in the
/// file runs without asking: a file can come from anyone, so its hooks,
/// actions and Shortcuts each ask before their first run here.
#[tauri::command]
pub async fn import_settings(
    text: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Saved, String> {
    let mut settings = parse(&text)?;
    settings.general.onboarded = state.settings().general.onboarded;
    settings.forget_trust();
    crate::commands::save_settings(settings, state, app).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARED: &str = r#"
[ai]
provider = "custom"
base_url = "https://example.com/v1"

[actions]
trusted_shortcuts = ["Log water"]

[[actions.mine]]
name = "Deploy"
kind = "program"
path = "/bin/sh"
args = ["-c", "true"]
trusted = true

[[hooks]]
when = "dictation"
kind = "program"
path = "/bin/sh"
args = ["-c", "true"]
trusted = true

[[hooks]]
when = "meeting"
kind = "web"
url = "https://example.com/hook"
trusted = true
"#;

    #[test]
    fn an_imported_file_says_what_it_runs_and_nothing_in_it_is_trusted() {
        let mut settings = parse(SHARED).unwrap();
        let lines = outside(&settings);
        assert!(
            lines.contains(&"Runs /bin/sh -c true after each dictation".to_string()),
            "{lines:?}"
        );
        assert!(
            lines.contains(
                &"Sends the text to https://example.com/hook after each meeting".to_string()
            ),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("when you say “Deploy”")));
        assert!(
            lines
                .iter()
                .any(|l| l.ends_with("to https://example.com/v1"))
        );

        settings.forget_trust();
        assert!(settings.hooks.iter().all(|h| !h.trusted));
        assert!(settings.actions.mine.iter().all(|a| !a.trusted));
        assert!(settings.actions.trusted_shortcuts.is_empty());
        // A local server is the user's own computer: nothing to warn of.
        let local =
            parse("[ai]\nprovider = \"custom\"\nbase_url = \"http://localhost:11434/v1\"\n")
                .unwrap();
        assert!(outside(&local).is_empty());
    }
}
