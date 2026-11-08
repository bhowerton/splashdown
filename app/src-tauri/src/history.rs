//! Dictation history: a JSON-lines file in the app's config
//! folder, newest last. Local only; never sent anywhere.

use splashdown_core::sync::Locked;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use splashdown_core::AudioClip;
use splashdown_core::Mode;

use tauri::State;

use crate::engine;
use crate::state::AppState;

/// Entries kept; older ones are dropped.
const MAX_ENTRIES: usize = 300;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Milliseconds since the Unix epoch; also the entry's id.
    pub at: u64,
    pub app: Option<String>,
    /// Bundle ID / executable, for building preset rules.
    #[serde(default)]
    pub app_id: Option<String>,
    /// Website host, when dictating into a browser.
    #[serde(default)]
    pub site: Option<String>,
    /// Stored as "dictate", "dictate_instruct", "transform" or "raw".
    pub mode: Mode,
    /// What was heard (for a transform, the spoken instruction).
    pub raw: String,
    pub instruction: Option<String>,
    /// What was inserted (or copied).
    pub text: String,
    /// Why the raw text was used instead of the AI output, if it was.
    pub note: Option<String>,
    pub stt: Option<String>,
    pub llm: Option<String>,
    pub millis: u64,
    /// What the cloud providers charged, in US dollars (estimated). `None`
    /// when it ran on this computer or before costs were tracked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// The recording is kept ("Keep the audio"), so it can be transcribed
    /// again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub audio: bool,
    /// The preset used (entries from before presets have none).
    #[serde(default)]
    pub preset: Option<String>,
    /// Pinned: listed first, and never deleted by "Delete after" or the
    /// 300-entry limit (only by hand).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
}

impl Entry {
    pub fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }
}

/// An app (and website) the user recently dictated into, offered when
/// adding a preset rule.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecentApp {
    pub id: String,
    pub name: String,
    pub site: Option<String>,
}

const MAX_RECENT_APPS: usize = 12;

pub struct History {
    path: PathBuf,
    entries: Mutex<Vec<Entry>>,
    /// Kept in memory even when history is off.
    recent_apps: Mutex<Vec<RecentApp>>,
    /// The most recent result, kept in memory even when history is off so
    /// "Paste last raw transcript" still works.
    latest: Mutex<Option<Entry>>,
    /// Entries older than this many days are deleted ("Delete after").
    /// `None`: kept until `MAX_ENTRIES` pushes them out.
    keep_days: Mutex<Option<u32>>,
}

impl History {
    pub fn open(path: PathBuf) -> Self {
        let entries = load(&path).unwrap_or_else(|e| {
            tracing::warn!("couldn't read history: {e}");
            Vec::new()
        });
        let latest = Mutex::new(entries.last().cloned());
        let mut recent = Vec::new();
        for e in entries.iter().rev() {
            push_recent(&mut recent, e);
        }
        recent.reverse();
        Self {
            path,
            entries: Mutex::new(entries),
            latest,
            recent_apps: Mutex::new(recent),
            keep_days: Mutex::new(None),
        }
    }

    /// Sets how many days entries are kept (from the settings, at startup
    /// and whenever they change) and deletes any that are already older.
    pub fn set_keep_days(&self, days: Option<u32>) {
        *self.keep_days.locked() = days;
        self.expire(Entry::now_ms());
    }

    /// Deletes entries older than the "Delete after" period, with their
    /// recordings.
    fn expire(&self, now: u64) {
        let mut entries = self.lock();
        if self.drop_expired(&mut entries, now) > 0
            && let Err(e) = rewrite(&self.path, &entries)
        {
            tracing::warn!("couldn't save history: {e}");
        }
    }

    /// Removes expired entries from `entries` and deletes their recordings.
    /// Returns how many there were.
    fn drop_expired(&self, entries: &mut Vec<Entry>, now: u64) -> usize {
        let Some(days) = *self.keep_days.locked() else {
            return 0;
        };
        let cutoff = now.saturating_sub(u64::from(days) * DAY_MS);
        let before = entries.len();
        entries.retain(|e| {
            let keep = e.pinned || e.at >= cutoff;
            if !keep {
                self.remove_audio(e.at);
            }
            keep
        });
        before - entries.len()
    }

    /// Newest first.
    pub fn recent_apps(&self) -> Vec<RecentApp> {
        let mut v = self.recent_apps.locked().clone();
        v.reverse();
        v
    }

    pub fn find(&self, at: u64) -> Option<Entry> {
        self.lock().iter().find(|e| e.at == at).cloned()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.locked()
    }

    /// Remembers the latest result in memory only (history turned off).
    pub fn remember(&self, entry: Entry) {
        push_recent(&mut self.recent_apps.locked(), &entry);
        *self.latest.locked() = Some(entry);
    }

    pub fn add(&self, entry: Entry) {
        self.remember(entry.clone());
        let mut entries = self.lock();
        entries.push(entry.clone());
        let expired = self.drop_expired(&mut entries, entry.at);
        // The oldest unpinned entries make way, never the one just added.
        let mut excess = entries.len().saturating_sub(MAX_ENTRIES);
        let dropped = excess;
        entries.retain(|e| {
            if excess == 0 || e.pinned || e.at == entry.at {
                return true;
            }
            excess -= 1;
            self.remove_audio(e.at);
            false
        });
        let excess = dropped - excess;
        let result = if expired + excess > 0 {
            rewrite(&self.path, &entries)
        } else {
            append(&self.path, &entry)
        };
        if let Err(e) = result {
            tracing::warn!("couldn't save history: {e}");
        }
    }

    pub fn last(&self) -> Option<Entry> {
        self.latest.locked().clone()
    }

    /// Pinned entries first, then newest first, optionally filtered by a
    /// case-insensitive search. Expired entries are deleted first, so an
    /// app left running for days doesn't show them.
    pub fn list(&self, query: &str) -> Vec<Entry> {
        self.expire(Entry::now_ms());
        let q = query.trim().to_lowercase();
        let entries = self.lock();
        let pinned = entries.iter().rev().filter(|e| e.pinned);
        let rest = entries.iter().rev().filter(|e| !e.pinned);
        pinned
            .chain(rest)
            .filter(|e| {
                q.is_empty()
                    || e.text.to_lowercase().contains(&q)
                    || e.raw.to_lowercase().contains(&q)
                    || e.app
                        .as_deref()
                        .is_some_and(|a| a.to_lowercase().contains(&q))
            })
            .cloned()
            .collect()
    }

    /// Pins or unpins an entry.
    pub fn pin(&self, at: u64, pinned: bool) -> std::io::Result<()> {
        let mut entries = self.lock();
        if let Some(entry) = entries.iter_mut().find(|e| e.at == at) {
            entry.pinned = pinned;
        }
        rewrite(&self.path, &entries)
    }

    pub fn delete(&self, at: u64) -> std::io::Result<()> {
        let mut entries = self.lock();
        entries.retain(|e| e.at != at);
        self.remove_audio(at);
        rewrite(&self.path, &entries)
    }

    pub fn clear(&self) -> std::io::Result<()> {
        let mut entries = self.lock();
        entries.clear();
        let _ = std::fs::remove_dir_all(self.audio_dir());
        rewrite(&self.path, &entries)
    }

    /// Recordings live next to the history file, one or two WAV files per
    /// entry (what was said, and a spoken instruction).
    fn audio_dir(&self) -> PathBuf {
        self.path.with_file_name("audio")
    }

    fn audio_paths(&self, at: u64) -> (PathBuf, PathBuf) {
        let dir = self.audio_dir();
        (
            dir.join(format!("{at}.wav")),
            dir.join(format!("{at}-instruction.wav")),
        )
    }

    /// Saves an entry's recording (16 kHz mono WAV, about 32 KB a second).
    pub fn save_audio(
        &self,
        at: u64,
        speech: &AudioClip,
        instruction: Option<&AudioClip>,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(self.audio_dir())?;
        let (speech_path, instruction_path) = self.audio_paths(at);
        std::fs::write(speech_path, splashdown_audio::wav::encode(speech))?;
        if let Some(clip) = instruction {
            std::fs::write(instruction_path, splashdown_audio::wav::encode(clip))?;
        }
        Ok(())
    }

    /// An entry's recording, if it was kept.
    pub fn audio(&self, at: u64) -> Option<(AudioClip, Option<AudioClip>)> {
        let (speech_path, instruction_path) = self.audio_paths(at);
        let speech = splashdown_audio::wav::read(&speech_path).ok()?;
        let instruction = splashdown_audio::wav::read(&instruction_path).ok();
        Some((speech, instruction))
    }

    fn remove_audio(&self, at: u64) {
        let (speech_path, instruction_path) = self.audio_paths(at);
        let _ = std::fs::remove_file(speech_path);
        let _ = std::fs::remove_file(instruction_path);
    }
}

/// What "Export…" on the History tab writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Markdown,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Markdown => "md",
        }
    }
}

/// One entry as exported: when, where, how, what was said and what was
/// inserted. Leaves out internals (ids, models, timings, costs).
#[derive(Serialize)]
struct Exported<'a> {
    time: String,
    app: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    site: Option<&'a str>,
    mode: Mode,
    #[serde(skip_serializing_if = "Option::is_none")]
    preset: Option<&'a str>,
    said: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    instruction: Option<&'a str>,
    text: &'a str,
}

/// Local time, e.g. "2026-10-29 14:05".
fn local_time(at: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(at as i64)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Dictate => "Dictate",
        Mode::DictateInstruct => "Instruct",
        Mode::Transform => "Transform",
        Mode::Raw => "Raw",
    }
}

/// `entries` (newest first, as `list` returns them) as a file.
pub fn export(entries: &[Entry], format: ExportFormat) -> String {
    match format {
        ExportFormat::Json => {
            let items: Vec<Exported> = entries
                .iter()
                .map(|e| Exported {
                    time: local_time(e.at),
                    app: e.app.as_deref(),
                    site: e.site.as_deref(),
                    mode: e.mode,
                    preset: e.preset.as_deref(),
                    said: &e.raw,
                    instruction: e.instruction.as_deref(),
                    text: &e.text,
                })
                .collect();
            serde_json::to_string_pretty(&items).unwrap_or_default() + "\n"
        }
        ExportFormat::Markdown => {
            let mut out = format!(
                "# Splashdown history\n\n{} dictation{}, newest first. Exported {}.\n",
                entries.len(),
                if entries.len() == 1 { "" } else { "s" },
                local_time(Entry::now_ms())
            );
            for e in entries {
                let app = match (&e.app, &e.site) {
                    (Some(app), Some(site)) => format!("{app} ({site})"),
                    (Some(app), None) => app.clone(),
                    _ => "Unknown app".into(),
                };
                out += &format!(
                    "\n## {} · {app} · {}\n\n",
                    local_time(e.at),
                    mode_label(e.mode)
                );
                if e.raw != e.text {
                    out += &format!("Said:\n\n{}\n\n", quote(&e.raw));
                    if let Some(instruction) = &e.instruction {
                        out += &format!("Instruction:\n\n{}\n\n", quote(instruction));
                    }
                    out += "Text:\n\n";
                }
                out += &quote(&e.text);
                out += "\n";
            }
            out
        }
    }
}

/// Markdown block quote, so the text's own formatting can't break the
/// file's headings.
fn quote(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.is_empty() {
                ">".into()
            } else {
                format!("> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Adds the entry's app (and site) as the most recent, oldest last-out.
fn push_recent(recent: &mut Vec<RecentApp>, e: &Entry) {
    let Some(id) = e.app_id.clone().filter(|i| !i.is_empty()) else {
        return;
    };
    let item = RecentApp {
        name: e.app.clone().unwrap_or_else(|| id.clone()),
        id,
        site: e.site.clone(),
    };
    recent.retain(|r| r != &item);
    recent.push(item);
    if recent.len() > MAX_RECENT_APPS {
        recent.remove(0);
    }
}

fn load(path: &Path) -> std::io::Result<Vec<Entry>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut entries: Vec<Entry> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        // A line from an older format or a torn write is skipped.
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect();
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    Ok(entries)
}

fn append(path: &Path, entry: &Entry) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{}", serde_json::to_string(entry)?)
}

fn rewrite(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut f = File::create(&tmp)?;
        for e in entries {
            writeln!(f, "{}", serde_json::to_string(e)?)?;
        }
    }
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at: u64, text: &str) -> Entry {
        Entry {
            at,
            app: Some("Notes".into()),
            app_id: Some("com.apple.Notes".into()),
            site: None,
            mode: Mode::Dictate,
            raw: format!("raw {text}"),
            instruction: None,
            text: text.into(),
            note: None,
            stt: None,
            llm: None,
            millis: 500,
            cost_usd: None,
            audio: false,
            preset: None,
            pinned: false,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("splashdown-history-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("history.jsonl")
    }

    #[test]
    fn audio_is_kept_and_removed_with_its_entry() {
        let dir =
            std::env::temp_dir().join(format!("splashdown-history-audio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let h = History::open(dir.join("history.jsonl"));
        let clip = AudioClip::new(vec![0.1; 1600], 16_000);
        h.save_audio(42, &clip, Some(&clip)).unwrap();
        let (speech, instruction) = h.audio(42).expect("kept");
        assert_eq!(speech.samples.len(), 1600);
        assert!(instruction.is_some());
        h.delete(42).unwrap();
        assert!(h.audio(42).is_none(), "removed with the entry");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persists_lists_newest_first_and_searches() {
        let path = temp_path("basic");
        let h = History::open(path.clone());
        h.add(entry(1, "first note"));
        h.add(entry(2, "second thing"));
        assert_eq!(h.last().unwrap().at, 2);

        let reopened = History::open(path.clone());
        let all = reopened.list("");
        assert_eq!(all.iter().map(|e| e.at).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(reopened.list("FIRST").len(), 1, "case-insensitive");
        assert_eq!(reopened.list("notes").len(), 2, "matches app name");

        reopened.delete(1).unwrap();
        assert_eq!(History::open(path.clone()).list("").len(), 1);
        reopened.clear().unwrap();
        assert!(History::open(path.clone()).list("").is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn deletes_entries_after_the_chosen_days() {
        let path = temp_path("days");
        let h = History::open(path.clone());
        let now = Entry::now_ms();
        h.add(entry(now - 10 * DAY_MS, "old"));
        h.add(entry(now - 2 * DAY_MS, "recent"));
        h.save_audio(
            now - 10 * DAY_MS,
            &AudioClip::new(vec![0.1; 160], 16_000),
            None,
        )
        .unwrap();
        assert_eq!(h.list("").len(), 2, "kept while unset");

        h.set_keep_days(Some(7));
        assert_eq!(h.list("").len(), 1);
        assert!(
            h.audio(now - 10 * DAY_MS).is_none(),
            "recording removed too"
        );
        assert_eq!(History::open(path.clone()).list("").len(), 1, "saved");

        h.add(entry(now, "new"));
        h.set_keep_days(Some(1));
        let left: Vec<String> = h.list("").into_iter().map(|e| e.text).collect();
        assert_eq!(left, vec!["new"]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn exports_json_and_markdown() {
        let mut said = entry(2, "Hi Sam,\n\nSee you Thursday.");
        said.mode = Mode::DictateInstruct;
        said.instruction = Some("as an email".into());
        let plain = Entry {
            raw: "# not a heading".into(),
            text: "# not a heading".into(),
            ..entry(1, "")
        };
        let entries = [said, plain];

        let json: serde_json::Value =
            serde_json::from_str(&export(&entries, ExportFormat::Json)).unwrap();
        assert_eq!(json[0]["app"], "Notes");
        assert_eq!(json[0]["mode"], "dictate_instruct");
        assert_eq!(json[0]["said"], "raw Hi Sam,\n\nSee you Thursday.");
        assert_eq!(json[0]["instruction"], "as an email");
        assert_eq!(json[0]["text"], "Hi Sam,\n\nSee you Thursday.");
        assert!(json[0].get("stt").is_none(), "no internals");
        assert_eq!(json[0]["time"].as_str().unwrap().len(), 16);

        let md = export(&entries, ExportFormat::Markdown);
        assert!(md.starts_with("# Splashdown history\n\n2 dictations"));
        assert!(md.contains("· Notes · Instruct\n"));
        assert!(md.contains("Instruction:\n\n> as an email\n"));
        assert!(md.contains("> Hi Sam,\n>\n> See you Thursday.\n"));
        assert!(md.contains("> # not a heading\n"), "text is quoted");
        assert!(
            !md.contains("Said:\n\n> # not"),
            "said shown only when it differs"
        );
    }

    #[test]
    fn caps_entries() {
        let path = temp_path("cap");
        let h = History::open(path.clone());
        for i in 0..(MAX_ENTRIES as u64 + 5) {
            h.add(entry(i, "x"));
        }
        let reopened = History::open(path.clone());
        let all = reopened.list("");
        assert_eq!(all.len(), MAX_ENTRIES);
        assert_eq!(all.last().unwrap().at, 5, "oldest entries dropped");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn pinned_entries_come_first_and_stay() {
        let path = temp_path("pin");
        let h = History::open(path.clone());
        h.add(entry(1, "keep me"));
        h.pin(1, true).unwrap();
        for i in 2..(MAX_ENTRIES as u64 + 10) {
            h.add(entry(i, "x"));
        }
        let reopened = History::open(path.clone());
        let all = reopened.list("");
        assert_eq!(all.len(), MAX_ENTRIES);
        assert_eq!(
            (all[0].at, all[0].pinned),
            (1, true),
            "pinned first, and kept"
        );
        assert_eq!(all[1].at, MAX_ENTRIES as u64 + 9, "then newest first");

        // Old enough to expire, but pinned.
        reopened.set_keep_days(Some(1));
        let now = Entry::now_ms();
        reopened.add(entry(now, "today"));
        let left: Vec<u64> = reopened.list("").iter().map(|e| e.at).collect();
        assert_eq!(left, [1, now]);

        reopened.pin(1, false).unwrap();
        reopened.set_keep_days(Some(1));
        assert_eq!(reopened.list("").len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub fn get_history(query: String, state: State<'_, AppState>) -> Vec<Entry> {
    state.history.list(&query)
}

#[derive(Serialize)]
pub struct Retranscribed {
    /// The new final text.
    pub(super) text: String,
    /// What the speech model heard.
    pub(super) raw: String,
    pub(super) stt: String,
}

/// Transcribes a kept recording again with the current speech-to-text and
/// AI settings, and returns the result (the history entry is unchanged).
#[tauri::command]
pub async fn retranscribe_history(
    at: u64,
    state: State<'_, AppState>,
) -> Result<Retranscribed, String> {
    let entry = state.history.find(at).ok_or("That entry is gone.")?;
    if entry.mode == splashdown_core::Mode::Transform {
        return Err("Edits of selected text can't be redone: the selection isn't stored.".into());
    }
    let (speech, instruction) = state
        .history
        .audio(at)
        .ok_or("The recording for this entry wasn't kept.")?;
    let settings = state.settings();
    let engine = tauri::async_runtime::spawn_blocking(move || engine::build(&settings))
        .await
        .map_err(|e| e.to_string())??;
    let outcome = engine
        .pipeline
        .run(splashdown_core::AudioJob {
            mode: entry.mode,
            speech,
            instruction_speech: instruction,
            selection: None,
            context: engine.context.clone(),
            hints: engine.hints.clone(),
            standing_instruction: None,
            prefix: None,
        })
        .await
        .map_err(|e| e.to_string())?;
    crate::spend::record_outcome(&outcome);
    Ok(Retranscribed {
        text: outcome.text,
        raw: outcome.raw,
        stt: engine.pipeline.stt_name(),
    })
}

#[tauri::command]
pub fn delete_history(at: u64, state: State<'_, AppState>) -> Result<(), String> {
    state.history.delete(at).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pin_history(at: u64, pinned: bool, state: State<'_, AppState>) -> Result<(), String> {
    state.history.pin(at, pinned).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    state.history.clear().map_err(|e| e.to_string())
}

/// Copies text from the Settings window (the webview's own clipboard API
/// needs extra permissions).
#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    splashdown_platform::copy_to_clipboard(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_recent_apps(state: State<'_, AppState>) -> Vec<crate::history::RecentApp> {
    state.history.recent_apps()
}

/// Re-runs the AI pass on a history entry's words with another preset and
/// returns the new text (nothing is inserted).
#[tauri::command]
pub async fn redo_history(
    at: u64,
    preset_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let entry = state.history.find(at).ok_or("That entry is gone.")?;
    if entry.mode == splashdown_core::Mode::Transform {
        return Err("Transforms can't be redone: the original selection isn't stored.".into());
    }
    let session = state.session.clone();
    tauri::async_runtime::spawn_blocking(move || {
        session.redo(entry.raw, entry.instruction, preset_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
