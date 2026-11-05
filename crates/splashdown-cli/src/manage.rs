//! Looking at and changing Splashdown from scripts: the app's status,
//! settings by dotted key, which model is used, and searching history.
//! Changes go to `config.toml`, and a running app is told to read it again.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value as Json;
use splashdown_config::Settings;
#[cfg(feature = "full")]
use splashdown_config::{AiProvider, SpeechProvider};
use splashdown_control::{ClientError, Request};
#[cfg(feature = "full")]
use splashdown_providers::{LocalModel, SpeechModel};

/// A failure with its own exit code (see `exit_code`).
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// A setting, model or other name that doesn't exist.
    #[error("{0}")]
    NotFound(String),
}

/// Exit codes scripts can rely on: 0 worked, 1 failed, 2 wrong usage
/// (clap), 3 the app isn't running (for commands that need it), 4 no such
/// setting or model.
/// Where the app keeps its settings, history and models.
pub fn paths() -> Result<splashdown_config::Paths> {
    splashdown_config::Paths::resolve().context("couldn't find the home directory")
}

pub fn exit_code(e: &anyhow::Error) -> u8 {
    if matches!(e.downcast_ref(), Some(ClientError::NotRunning)) {
        3
    } else if e.downcast_ref::<CliError>().is_some() {
        4
    } else {
        1
    }
}

fn config_path(config: Option<PathBuf>) -> Result<PathBuf> {
    Ok(match config {
        Some(p) => p,
        None => paths()?.config_file(),
    })
}

/// Saves `settings` and, when it's the app's own file, asks the running
/// app to use it now.
fn save(settings: &Settings, path: &Path, own: bool) -> Result<()> {
    settings.save(path).map_err(anyhow::Error::msg)?;
    if own {
        // Not running: it reads the file when it starts.
        let _ = splashdown_control::send(&Request::ReloadSettings);
    }
    Ok(())
}

// ---------------------------------------------------------------- status

/// The running app's state. `None` printed as JSON when it isn't running
/// (exit code 3).
pub fn status(json: bool) -> Result<()> {
    let reply = splashdown_control::send(&Request::Status)?.unwrap_or_default();
    let state: Json = serde_json::from_str(&reply).context("reading the app's status")?;
    if json {
        println!("{state}");
        return Ok(());
    }
    let text = |key: &str| state.get(key).and_then(Json::as_str).unwrap_or("");
    println!("Splashdown {}", text("version"));
    if state["ready"].as_bool() == Some(true) {
        println!("Ready");
    } else {
        println!("Not set up: {}", text("problem"));
    }
    if state["recording"].as_bool() == Some(true) {
        println!("Recording a dictation");
    } else if state["working"].as_bool() == Some(true) {
        println!("Working on a dictation");
    }
    if !text("speech").is_empty() {
        println!("Speech to text: {}", text("speech"));
    }
    println!("AI: {}", state["ai"].as_str().unwrap_or("off"));
    if let Some(meeting) = state.get("meeting").filter(|m| !m.is_null()) {
        let secs = meeting["elapsed"].as_f64().unwrap_or(0.0) as u64;
        println!(
            "Recording a meeting: {}:{:02}, into {}",
            secs / 60,
            secs % 60,
            meeting["note"].as_str().unwrap_or("")
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- settings

/// The settings as a TOML tree, unset values left out.
fn tree(settings: &Settings) -> Result<toml::Value> {
    Ok(toml::Value::try_from(settings)?)
}

fn lookup<'a>(tree: &'a toml::Value, key: &str) -> Option<&'a toml::Value> {
    key.split('.').try_fold(tree, |node, part| node.get(part))
}

/// Every setting as `key = value`, one per line (lists of tables, like
/// presets, as one line each).
fn flatten(prefix: &str, value: &toml::Value, out: &mut Vec<(String, String)>) {
    match value {
        toml::Value::Table(table) => {
            for (k, v) in table {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        other => out.push((prefix.to_string(), other.to_string())),
    }
}

pub fn settings_list(config: Option<PathBuf>) -> Result<()> {
    let settings = Settings::load(&config_path(config)?)?;
    let mut out = Vec::new();
    flatten("", &tree(&settings)?, &mut out);
    for (key, value) in out {
        println!("{key} = {value}");
    }
    Ok(())
}

pub fn settings_get(key: &str, config: Option<PathBuf>) -> Result<()> {
    let settings = Settings::load(&config_path(config)?)?;
    let tree = tree(&settings)?;
    match lookup(&tree, key) {
        Some(toml::Value::String(s)) => println!("{s}"),
        Some(value) => println!("{value}"),
        None => {
            return Err(CliError::NotFound(format!(
                "{key} isn't set (it has its default); `splashdown-cli settings list` shows what is"
            ))
            .into());
        }
    }
    Ok(())
}

/// A value as typed: TOML (`true`, `0.5`, `["a", "b"]`, `"text"`) or, if
/// it isn't valid TOML, plain text.
fn parse_value(text: &str) -> toml::Value {
    toml::from_str::<toml::Table>(&format!("v = {text}"))
        .ok()
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| toml::Value::String(text.to_string()))
}

/// Sets `key` (dotted) to `value`, or removes it (back to its default)
/// when `value` is `None`. Fails for a key Splashdown doesn't have, or a
/// value of the wrong kind.
pub fn settings_set(key: &str, value: Option<&str>, config: Option<PathBuf>) -> Result<()> {
    let own = config.is_none();
    let path = config_path(config)?;
    let settings = Settings::load(&path)?;
    let mut tree = tree(&settings)?;
    let parts: Vec<&str> = key.split('.').collect();
    let (last, parents) = parts.split_last().context("no setting named")?;
    let mut node = &mut tree;
    for part in parents {
        node = node
            .as_table_mut()
            .context("not a group of settings")?
            .entry(part.to_string())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    }
    let table = node
        .as_table_mut()
        .with_context(|| format!("{} isn't a group of settings", parents.join(".")))?;
    let wanted = value.map(parse_value);
    match &wanted {
        Some(v) => {
            table.insert(last.to_string(), v.clone());
        }
        None => {
            table.remove(*last);
        }
    }
    let updated: Settings = tree
        .try_into()
        .with_context(|| format!("{key} can't be {}", value.unwrap_or("unset")))?;
    // A key Splashdown doesn't know is dropped when it reads the settings:
    // say so rather than pretend it was saved. (Only that it's there: a
    // number may come back as 5.0 for 5.)
    if wanted.is_some() && lookup(&self::tree(&updated)?, key).is_none() {
        return Err(CliError::NotFound(format!("Splashdown has no setting {key}")).into());
    }
    if let Some(problem) = updated.hotkey_problem() {
        bail!("{problem}");
    }
    if let Some(problem) = splashdown_config::replacement_problem(&updated.replacements) {
        bail!("{problem}");
    }
    save(&updated, &path, own)?;
    println!("{key} = {}", value.unwrap_or("(default)"));
    Ok(())
}

// ---------------------------------------------------------------- models

/// Uses a downloaded model: a speech model for speech-to-text on this
/// computer, or the built-in text model for the AI.
#[cfg(feature = "full")]
pub fn use_model(id: &str, config: Option<PathBuf>) -> Result<()> {
    let own = config.is_none();
    let path = config_path(config)?;
    let mut settings = Settings::load(&path)?;
    let models_dir = paths()?.models_dir;
    if let Some(model) = SpeechModel::from_id(id) {
        if !model.is_downloaded(&models_dir) {
            bail!("{id} isn't downloaded: splashdown-cli models download {id}");
        }
        settings.speech.provider = SpeechProvider::WhisperLocal;
        settings.speech.model = Some(model.id().to_string());
        save(&settings, &path, own)?;
        println!("Speech to text: {} ({})", model.id(), model.label());
    } else if let Some(model) = LocalModel::from_id(id) {
        if !model.is_downloaded(&models_dir) {
            bail!("{id} isn't downloaded: splashdown-cli models download {id}");
        }
        settings.ai.provider = AiProvider::Local;
        settings.ai.model = Some(model.id().to_string());
        save(&settings, &path, own)?;
        println!("AI: the built-in model {}", model.id());
    } else {
        return Err(CliError::NotFound(format!(
            "no model {id}; `splashdown-cli models list` shows them"
        ))
        .into());
    }
    Ok(())
}

// ---------------------------------------------------------------- history

/// History entries containing `query` (any case) in what was said or
/// inserted, newest first. An empty query lists the latest.
pub fn history(query: &str, limit: usize, json: bool) -> Result<()> {
    let path = paths()?.history_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let needle = query.to_lowercase();
    let mut found: Vec<Json> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Json>(line).ok())
        .filter(|entry| {
            needle.is_empty()
                || ["text", "raw"].iter().any(|k| {
                    entry[k]
                        .as_str()
                        .is_some_and(|s| s.to_lowercase().contains(&needle))
                })
        })
        .collect();
    found.reverse();
    found.truncate(limit);
    if json {
        println!("{}", Json::Array(found));
        return Ok(());
    }
    if found.is_empty() {
        eprintln!(
            "{}",
            if text.is_empty() {
                "No history (Settings → Privacy → Keep history is off, or nothing was dictated yet)."
            } else {
                "Nothing matches."
            }
        );
    }
    for entry in found {
        let when = entry["at"]
            .as_i64()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let app = entry["app"].as_str().unwrap_or("");
        let said = entry["text"].as_str().unwrap_or("").replace('\n', " ");
        println!("{when}  {app:<14}  {said}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("splashdown-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[test]
    fn values_are_read_as_typed() {
        assert_eq!(parse_value("true"), toml::Value::Boolean(true));
        assert_eq!(parse_value("0.5"), toml::Value::Float(0.5));
        assert_eq!(parse_value("\"en\""), toml::Value::String("en".into()));
        assert_eq!(parse_value("en"), toml::Value::String("en".into()));
        assert_eq!(
            parse_value("[\"Tauri\", \"Kubernetes\"]"),
            toml::Value::Array(vec!["Tauri".into(), "Kubernetes".into()])
        );
    }

    #[test]
    fn settings_are_set_checked_and_unset() {
        let path = temp_config("set");
        settings_set("general.language", Some("de"), Some(path.clone())).unwrap();
        settings_set(
            "general.vocabulary",
            Some("[\"Tauri\"]"),
            Some(path.clone()),
        )
        .unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.general.language.as_deref(), Some("de"));
        assert_eq!(s.general.vocabulary, ["Tauri"]);

        // A typo isn't saved quietly, and neither is the wrong kind of value.
        let typo = settings_set("general.langauge", Some("de"), Some(path.clone())).unwrap_err();
        assert_eq!(exit_code(&typo), 4);
        // Numbers that come back written another way are fine.
        settings_set("general.sound_volume", Some("0.3"), Some(path.clone())).unwrap();
        settings_set("general.budget.monthly_usd", Some("5"), Some(path.clone())).unwrap();
        // Letters that clash are refused, and so is a rule that can't work.
        assert!(settings_set("hotkeys.ask", Some("h"), Some(path.clone())).is_err());
        let bad = r#"[{find = "(", replace = "x", regex = true}]"#;
        assert!(settings_set("replacements", Some(bad), Some(path.clone())).is_err());
        assert!(settings_set("general.vocabulary", Some("7"), Some(path.clone())).is_err());

        settings_set("general.language", None, Some(path.clone())).unwrap();
        assert_eq!(Settings::load(&path).unwrap().general.language, None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[cfg(feature = "full")]
    #[test]
    fn unknown_models_are_not_found() {
        let path = temp_config("model");
        let e = use_model("no-such-model", Some(path.clone())).unwrap_err();
        assert_eq!(exit_code(&e), 4);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
