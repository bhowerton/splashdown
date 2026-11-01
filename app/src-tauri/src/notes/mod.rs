//! The Notes window, which replaced the scratchpad: notes are
//! Markdown files in a folder the user picks (`store`), listed and edited
//! here. ⌃⌥N or the menu bar opens it and remembers the app that was in
//! front, so "Paste into {app}" can hand focus back and paste the note there.

pub mod files;
mod link_helper;
pub mod meaning;
pub mod media;
pub mod store;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use splashdown_platform::InsertMethod;
use tauri::{
    AppHandle, DragDropEvent, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_dialog::DialogExt;

use crate::commands::AppState;
use crate::engine;
use splashdown_config::{CaptureTo, NotesSettings};
use store::{Appended, Note, Origin, Saved, Store, Summary, Target, Version};

pub const WINDOW: &str = "notes";
/// Shown in the menu; registered in `shortcuts.rs`.
pub const ACCELERATOR: &str = "Ctrl+Alt+N";
/// App event telling the page it was shown again, with the app "Paste into"
/// would paste into (or null).
const SHOWN: &str = "notes-shown";
/// App event with a new app for "Paste into" (you used another app).
const TARGET: &str = "notes-send-target";
/// App event asking the page to run a transform (its shortcut was pressed
/// while Notes had focus), with the transform's id.
pub const RUN_TRANSFORM: &str = "notes-run-transform";
/// Reopened after this long, the window starts a new note rather than
/// showing the last one.
const FRESH_AFTER: Duration = Duration::from_secs(6 * 3600);
/// How long to wait for the other app to come to the front before pasting.
const ACTIVATE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Once it's in front, a moment for its window to take keyboard focus.
const SETTLE: Duration = Duration::from_millis(120);

// ---------------------------------------------------------------- folder

/// `Documents/Splashdown Notes`.
fn default_folder() -> Option<PathBuf> {
    let documents = directories::UserDirs::new()
        .and_then(|d| d.document_dir().map(Path::to_path_buf))
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join("Documents")))?;
    Some(documents.join("Splashdown Notes"))
}

/// The folder in settings, or the default.
fn wanted_folder(state: &AppState) -> Option<PathBuf> {
    state.settings().notes.folder.or_else(default_folder)
}

/// Where notes go while the chosen folder can't be used (macOS refused
/// Documents access, a removed drive), so nothing typed is lost.
fn fallback_folder() -> Result<PathBuf, String> {
    Ok(engine::paths()?.config_dir.join("notes"))
}

struct Current {
    /// The folder asked for.
    wanted: PathBuf,
    store: Arc<Store>,
    /// Why notes are in the fallback folder instead.
    problem: Option<String>,
    /// When the folder asked for was last tried.
    tried: Instant,
}

/// While the fallback folder is in use, how often the chosen folder is
/// tried again (the user may have allowed access meanwhile).
const RETRY_FOLDER: Duration = Duration::from_secs(30);

fn current_slot() -> &'static Mutex<Option<Current>> {
    static CURRENT: OnceLock<Mutex<Option<Current>>> = OnceLock::new();
    CURRENT.get_or_init(Mutex::default)
}

/// The store for the notes folder, set up the first time: created
/// lazily (never at launch, so the Documents prompt only appears when
/// Notes is used), with the old scratchpad's notes moved in.
fn store(state: &AppState) -> Result<(Arc<Store>, Option<String>), String> {
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
fn migrate_scratchpad(store: &Store) {
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
fn label(path: &Path) -> String {
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
fn move_notes(from: &Store, to: &Path) -> Result<usize, String> {
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
fn merge_into(from: &Path, to: &Path) -> std::io::Result<()> {
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

// ---------------------------------------------------------------- capture

/// A dictation added to a note (dictation key + N), with what's needed to
/// take it back.
pub struct Captured {
    folder: Option<PathBuf>,
    appended: Appended,
    /// "Inbox", "2026-10-31", or the note's title.
    pub label: String,
}

fn capture_target(settings: &NotesSettings) -> Target {
    match settings.capture_to {
        CaptureTo::Inbox => Target::Inbox,
        CaptureTo::Daily => Target::Daily {
            folder: settings.daily_folder().to_string(),
            format: settings.daily_format().to_string(),
        },
        CaptureTo::Last => Target::Last,
    }
}

/// Where a capture would go now, for the overlay ("Inbox").
pub fn capture_label(settings: &NotesSettings) -> String {
    match settings.capture_to {
        CaptureTo::Inbox => "Inbox".into(),
        CaptureTo::Daily => "today's note".into(),
        CaptureTo::Last => "the last note".into(),
    }
}

/// Adds `text` to the capture note (or `to`, a note's path), with a link
/// to `audio` (a WAV recording) when there is one.
pub fn capture(
    settings: &NotesSettings,
    to: Option<&str>,
    text: &str,
    audio: Option<&[u8]>,
) -> Result<Captured, String> {
    let (store, _) = store_for(settings.folder.clone())?;
    let now = chrono::Local::now();
    let mut text = text.trim().to_string();
    if let Some(bytes) = audio {
        match store.save_attachment(&now.format("%Y-%m-%d %H%M").to_string(), "wav", bytes) {
            // A plain Markdown link, so it works outside Obsidian too.
            Ok(rel) => text.push_str(&format!(" [Recording]({})", rel.replace(' ', "%20"))),
            Err(e) => tracing::warn!("couldn't keep the recording with the note: {e}"),
        }
    }
    let target = to.map_or_else(
        || capture_target(settings),
        |p| {
            // "Ideas" means the note Ideas.md.
            let p = p.trim();
            if Path::new(p)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            {
                Target::Note(p.to_string())
            } else {
                Target::Note(format!("{p}.md"))
            }
        },
    );
    let appended = store
        .append(&target, &text, now)
        .map_err(|e| format!("Couldn't add it to your notes: {e}"))?;
    let label = Path::new(&appended.path)
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    Ok(Captured {
        folder: settings.folder.clone(),
        appended,
        label,
    })
}

/// Takes a capture back, if the note hasn't changed since.
pub fn undo_capture(captured: &Captured) -> Result<bool, String> {
    let (store, _) = store_for(captured.folder.clone())?;
    store
        .undo_append(&captured.appended)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- coding agents

/// Asked for while Settings doesn't allow it.
const AGENTS_OFF: &str = "Splashdown's notes are off for coding agents. To allow it, turn on “Let coding agents read notes” in Splashdown → Settings → Privacy.";
const AGENTS_NO_WRITE: &str = "Coding agents may read Splashdown's notes but not add to them. To allow it, turn on “…and add to them” in Splashdown → Settings → Privacy.";
/// Most notes one search returns.
const SEARCH_LIMIT: usize = 50;

/// `search_notes`: notes whose title or text contains `query` (any case),
/// titles first, then newest, as JSON with a short excerpt each.
pub fn agent_search(
    settings: &NotesSettings,
    query: &str,
    limit: Option<usize>,
) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    let (store, _) = store_for(settings.folder.clone())?;
    let needle = query.trim().to_lowercase();
    let limit = limit.unwrap_or(10).clamp(1, SEARCH_LIMIT);
    let mut hits: Vec<(bool, Summary)> = store
        .list()
        .into_iter()
        .filter_map(|n| {
            let in_title = n.title.to_lowercase().contains(&needle);
            (needle.is_empty() || in_title || n.text.to_lowercase().contains(&needle))
                .then_some((in_title, n))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.modified.cmp(&a.1.modified)));
    let results: Vec<serde_json::Value> = hits
        .into_iter()
        .take(limit)
        .map(|(_, n)| {
            serde_json::json!({
                "path": n.path,
                "title": n.title,
                "modified": chrono::DateTime::from_timestamp_millis(n.modified as i64)
                    .map(|t| t.with_timezone(&chrono::Local).to_rfc3339()),
                "excerpt": excerpt(&n.text, &needle),
            })
        })
        .collect();
    serde_json::to_string(&results).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- ask

/// Most notes a question gets excerpts from.
const ASK_NOTES: usize = 3;
/// Characters of each note an answer may use.
const ASK_EXCERPT: usize = 1200;

/// Excerpts of the notes that best match `question`, for the Ask panel, if
/// Settings allows it. Matching is by the question's words, weighted by how
/// rare each is across the notes, so "the", "der" or "le" count for little
/// in any language; and by meaning too when that's turned on.
pub fn excerpts_for(
    settings: &NotesSettings,
    question: &str,
) -> Vec<splashdown_core::ask::NoteExcerpt> {
    if !settings.ask_uses_notes() {
        return Vec::new();
    }
    let Ok((store, _)) = store_for(settings.folder.clone()) else {
        return Vec::new();
    };
    matching(
        &store,
        question,
        ASK_NOTES,
        ASK_EXCERPT,
        settings.ask_by_meaning,
    )
    .into_iter()
    .map(|(_, excerpt)| excerpt)
    .collect()
}

/// The `limit` notes that best match `question`: each one's path, and up
/// to `chars` of it around its best word, or the part closest in meaning.
/// With `by_meaning`, notes are ranked by both, each note by its places in
/// the two rankings (reciprocal rank fusion), so a note high in either
/// comes up and one high in both comes first.
fn matching(
    store: &Arc<Store>,
    question: &str,
    limit: usize,
    chars: usize,
    by_meaning: bool,
) -> Vec<(String, splashdown_core::ask::NoteExcerpt)> {
    /// Damps the difference between the top places (the usual 60).
    const K: f64 = 60.0;
    let notes = store.list();
    let mut ranked: Vec<(f64, &Summary, Option<String>, Option<meaning::Piece>)> = Vec::new();
    for (place, (note, word)) in best_matches(&notes, question, limit)
        .into_iter()
        .enumerate()
    {
        ranked.push((1.0 / (K + place as f64), note, Some(word), None));
    }
    let meant = if by_meaning {
        meaning::search(store, &notes, question, limit)
    } else {
        Vec::new()
    };
    for (place, hit) in meant.into_iter().enumerate() {
        let score = 1.0 / (K + place as f64);
        match ranked.iter_mut().find(|r| r.1.path == hit.path) {
            Some(r) => {
                r.0 += score;
                r.3 = Some(hit.piece);
            }
            None => {
                if let Some(note) = notes.iter().find(|n| n.path == hit.path) {
                    ranked.push((score, note, None, Some(hit.piece)));
                }
            }
        }
    }
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.modified.cmp(&a.1.modified)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, note, word, piece)| {
            let text = match (piece, word) {
                (Some(piece), _) => around_piece(&note.text, piece, chars),
                (None, Some(word)) => around(&note.text, &word, chars),
                (None, None) => clip(&note.text, chars, false),
            };
            (
                note.path.clone(),
                splashdown_core::ask::NoteExcerpt {
                    title: note.title.clone(),
                    date: day_of(note.modified),
                    text,
                },
            )
        })
        .collect()
}

/// Up to `max` characters of `text` with `piece` in them, starting a little
/// before it.
fn around_piece(text: &str, piece: meaning::Piece, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let before = max.saturating_sub(piece.len) / 3;
    let start = piece.start.saturating_sub(before).min(chars.len() - max);
    let mut out: String = chars[start..start + max].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if start + max < chars.len() {
        out.push('…');
    }
    out
}

/// "2026-10-30", for a time in Unix ms.
fn day_of(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

/// The first `max` characters of `text`, or the last ones when `recent`
/// (a meeting still going: what was said lately).
fn clip(text: &str, max: usize, recent: bool) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    if recent {
        let rest: String = text.chars().skip(count - max).collect();
        format!("…{rest}")
    } else {
        let start: String = text.chars().take(max).collect();
        format!("{start}…")
    }
}

/// Words too common to tell notes apart, in the languages most used: with
/// only a few notes, how often a word appears across them can't show it.
const COMMON_WORDS: &[&str] = &[
    // English
    "the", "and", "for", "are", "was", "were", "what", "when", "where", "which", "who", "whom",
    "why", "how", "that", "this", "these", "those", "with", "from", "about", "did", "does", "has",
    "have", "had", "can", "could", "would", "should", "will", "our", "your", "you", "they",
    "their", "them", "there", "then", "than", "but", "not", "any", "all", "its", "into", "out",
    "who's", "what's", "whats", "tell", "said", "say", // German
    "der", "die", "das", "und", "ist", "was", "wie", "wer", "wann", "wo", "warum", "welche",
    "haben", "hat", "wir", "ihr", "sie", "ein", "eine", "einen", "mit", "von", "für", "über",
    // French
    "les", "des", "est", "que", "qui", "quoi", "quand", "comment", "pour", "avec", "nous", "vous",
    "une", "dans", "sur", "pas", "quel", "quelle", // Spanish and Portuguese
    "los", "las", "que", "qué", "quién", "cuándo", "dónde", "cómo", "por", "para", "con", "una",
    "del", "nos", "fue", "son", "está", "uma", "não", "com", "quem", "quando", "onde", "como",
    // Italian and Dutch
    "che", "chi", "cosa", "quando", "dove", "come", "per", "con", "una", "het", "een", "wat", "wie",
    "wanneer", "waar", "hoe", "voor", "met", "van", "zijn",
];

/// Scripts written without spaces between words.
fn unspaced(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF // Hiragana, Katakana
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF // CJK ideographs
    )
}

/// The words in `text` worth matching, lowercased: three letters or more
/// and not among the most common words; in Chinese and Japanese, which
/// have no spaces, each pair of characters.
fn words_of(text: &str) -> std::collections::HashSet<String> {
    let mut words = std::collections::HashSet::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        let chars: Vec<char> = token.chars().collect();
        if chars.iter().any(|&c| unspaced(c)) {
            words.extend(chars.windows(2).map(|w| w.iter().collect::<String>()));
            continue;
        }
        let word = token.to_lowercase();
        if word.chars().count() >= 3 && !COMMON_WORDS.contains(&word.as_str()) {
            words.insert(word);
        }
    }
    words
}

/// The `limit` notes that match `question` best, each with its rarest
/// matching word (to excerpt around). A note matches on words found in its
/// title or text; a word in most of the notes counts for nothing.
fn best_matches<'a>(
    notes: &'a [Summary],
    question: &str,
    limit: usize,
) -> Vec<(&'a Summary, String)> {
    let mut words: Vec<String> = words_of(question).into_iter().collect();
    words.sort();
    let texts: Vec<std::collections::HashSet<String>> = notes
        .iter()
        .map(|n| words_of(&format!("{}\n{}", n.title, n.text)))
        .collect();
    let total = notes.len() as f64;
    // How telling each word is: rare across the notes is telling, and a
    // word in most of a handful of notes or more is just a common word.
    let weights: Vec<f64> = words
        .iter()
        .map(|w| {
            let found = texts.iter().filter(|t| t.contains(w)).count() as f64;
            if found == 0.0 || (total >= 4.0 && found > total / 2.0) {
                0.0
            } else {
                (total / found).ln() + 1.0
            }
        })
        .collect();
    let mut scored: Vec<(f64, usize, Option<usize>)> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let mut score = 0.0;
            let mut rarest: Option<usize> = None;
            for (w, word) in words.iter().enumerate() {
                if weights[w] > 0.0 && text.contains(word) {
                    score += weights[w];
                    if rarest.is_none_or(|r| weights[w] > weights[r]) {
                        rarest = Some(w);
                    }
                }
            }
            (score, i, rarest)
        })
        .filter(|(score, ..)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(notes[b.1].modified.cmp(&notes[a.1].modified))
    });
    scored
        .into_iter()
        .take(limit)
        .filter_map(|(_, i, rarest)| Some((&notes[i], words[rarest?].clone())))
        .collect()
}

/// Up to `max` characters of `text` around the first `word` (any case).
fn around(text: &str, word: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    // The word itself, not inside a longer one ("art", not "start").
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let lower = if lower.len() == chars.len() {
        lower
    } else {
        chars.clone()
    };
    let word: Vec<char> = word.chars().collect();
    let edge = |i: usize| {
        lower
            .get(i)
            .is_none_or(|c| !c.is_alphanumeric() || unspaced(*c))
    };
    let at = (0..lower.len().saturating_sub(word.len() - 1))
        .find(|&i| {
            lower[i..i + word.len()] == word[..]
                && (unspaced(word[0]) || (i == 0 || edge(i - 1)) && edge(i + word.len()))
        })
        .unwrap_or(0);
    let start = at.saturating_sub(max / 3).min(chars.len() - max);
    let mut out: String = chars[start..start + max].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if start + max < chars.len() {
        out.push('…');
    }
    out
}

/// About 240 characters of `text` around the first match.
fn excerpt(text: &str, needle: &str) -> String {
    let lower = text.to_lowercase();
    let at = if needle.is_empty() {
        None
    } else {
        lower.find(needle)
    };
    let chars: Vec<char> = text.chars().collect();
    let start = at
        .map(|byte| lower[..byte].chars().count().saturating_sub(80))
        .unwrap_or(0)
        .min(chars.len());
    let end = (start + 240).min(chars.len());
    let mut out: String = chars[start..end].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if end < chars.len() {
        out.push('…');
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `read_note`: a note's text as JSON.
pub fn agent_read(settings: &NotesSettings, path: &str) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    let (store, _) = store_for(settings.folder.clone())?;
    let note = store
        .read(path)
        .map_err(|e| format!("Couldn't read {path}: {e}"))?;
    // The reply is escaped twice on its way to the agent and must fit the
    // control channel's line limit, so a long note (an hour's transcript)
    // is cut short, and says so.
    let text: String = note.body.chars().take(AGENT_READ_CHARS).collect();
    let truncated = text.len() < note.body.len();
    serde_json::to_string(&serde_json::json!({
        "path": note.path,
        "title": store::title_of(&note.body),
        "text": text,
        "truncated": truncated,
    }))
    .map_err(|e| e.to_string())
}

/// The most of a note `read_note` returns, in characters.
const AGENT_READ_CHARS: usize = 50_000;

/// `append_note`: adds `text` to the capture note, or the note at `to`.
pub fn agent_append(
    settings: &NotesSettings,
    text: &str,
    to: Option<&str>,
) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    if !settings.agents_write {
        return Err(AGENTS_NO_WRITE.into());
    }
    if text.trim().is_empty() {
        return Err("There's no text to add.".into());
    }
    let captured = capture(settings, to, text, None)?;
    Ok(format!(
        "Added to {} ({})",
        captured.label, captured.appended.path
    ))
}

// ---------------------------------------------------------------- window

/// The app that was in front when the window opened: (pid, name).
fn target() -> &'static Mutex<Option<(i32, String)>> {
    static TARGET: OnceLock<Mutex<Option<(i32, String)>>> = OnceLock::new();
    TARGET.get_or_init(Mutex::default)
}

fn target_lock() -> std::sync::MutexGuard<'static, Option<(i32, String)>> {
    target().lock().unwrap_or_else(|e| e.into_inner())
}

/// Apps that take no text, so Send never goes to them: the file manager,
/// the system's settings, the desktop and the like (bundle IDs on macOS,
/// executable names on Windows).
const NO_TEXT: &[&str] = &[
    "com.apple.finder",
    "com.apple.systempreferences",
    "com.apple.dock",
    "com.apple.loginwindow",
    "com.apple.controlcenter",
    "com.apple.notificationcenterui",
    "com.apple.Spotlight",
    "explorer.exe",
    "SystemSettings.exe",
    "SearchHost.exe",
    "StartMenuExperienceHost.exe",
    "ShellExperienceHost.exe",
    "LockApp.exe",
];

/// Remembers the app in front as where Send goes, unless it's us
/// (Settings, Notes, Ask) or takes no text: then the one before stays.
/// Returns whether it changed.
fn remember_front_app() -> bool {
    let Some((pid, name, id)) = splashdown_platform::frontmost_app() else {
        return false;
    };
    if u32::try_from(pid).ok() == Some(std::process::id())
        || NO_TEXT.iter().any(|n| n.eq_ignore_ascii_case(&id))
    {
        return false;
    }
    let mut target = target_lock();
    let changed = target.as_ref().is_none_or(|(p, _)| *p != pid);
    *target = Some((pid, name));
    changed
}

/// Keeps track of the last app used, so Send goes where you just were even
/// when Notes stayed open while you worked elsewhere. Twice a second; it's
/// only asking which app is in front.
pub fn watch_front_app(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("front-app".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(500));
                if remember_front_app()
                    && let Some(window) = app.get_webview_window(WINDOW)
                    && window.is_visible().unwrap_or(false)
                {
                    let _ = app.emit_to(WINDOW, TARGET, target_name());
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("couldn't follow the app in front: {e}");
    }
}

fn target_name() -> Option<String> {
    target_lock()
        .as_ref()
        .map(|(_, name)| name.clone())
        .filter(|n| !n.is_empty())
}

/// Whether the Notes window is in front with focus.
pub fn is_focused(app: &AppHandle) -> bool {
    app.get_webview_window(WINDOW)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

/// ⌃⌥N: hides Notes if it has focus, otherwise opens it.
pub fn toggle(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW)
        && window.is_visible().unwrap_or(false)
        && window.is_focused().unwrap_or(false)
    {
        let _ = window.hide();
        // Hand focus back rather than leaving no app in front.
        if let Some((pid, _)) = *target_lock() {
            let _ = splashdown_platform::activate_app(pid);
        }
        return;
    }
    open(app);
}

/// Shows the Notes window (creating it the first time) with the note
/// focused.
pub fn open(app: &AppHandle) {
    remember_front_app();
    read_for_meaning(app);
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to(WINDOW, SHOWN, target_name());
        return;
    }
    let result = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("notes.html".into()))
        .title("Notes")
        .inner_size(1100.0, 680.0)
        .min_inner_size(420.0, 300.0)
        .resizable(true)
        // A window of its own, unless the user keeps it on top to dictate
        // into while they look at another app.
        .always_on_top(
            app.try_state::<AppState>()
                .is_some_and(|s| s.settings().notes.keep_on_top),
        )
        // Comes up on the Space (desktop) the user is on, not the one it was
        // first opened on.
        .visible_on_all_workspaces(true)
        .center()
        .focused(true)
        .build();
    match result {
        Ok(window) => {
            // Closing only hides it, so the page (and anything not yet
            // saved) stays.
            let handle = window.clone();
            let app = app.clone();
            window.on_window_event(move |event| match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = handle.hide();
                }
                // Audio and video files dropped on the window are
                // transcribed.
                // Files only: dragging text or a link isn't a transcription.
                WindowEvent::DragDrop(DragDropEvent::Enter { paths, .. }) if !paths.is_empty() => {
                    let _ = app.emit_to(WINDOW, files::DRAGGING, true);
                }
                WindowEvent::DragDrop(DragDropEvent::Leave) => {
                    let _ = app.emit_to(WINDOW, files::DRAGGING, false);
                }
                WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
                    let _ = app.emit_to(WINDOW, files::DRAGGING, false);
                    files::dropped(&app, paths.clone());
                }
                _ => {}
            });
        }
        Err(e) => tracing::error!("failed to open Notes: {e}"),
    }
}

/// Hides Notes, brings back the app that was in front before it opened,
/// and pastes `text` there. On failure the note is left on the clipboard
/// and the window comes back.
fn send(app: &AppHandle, text: &str) -> Result<(), String> {
    let Some((pid, name)) = target_lock().clone() else {
        return Err("No app to send to. Copy the note instead.".into());
    };
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.hide();
    }
    if let Err(e) = paste_into(pid, text) {
        tracing::warn!("notes send failed: {e}");
        let _ = splashdown_platform::copy_to_clipboard(text);
        if let Some(window) = app.get_webview_window(WINDOW) {
            let _ = window.show();
            let _ = window.set_focus();
        }
        return Err(format!(
            "Couldn't paste into {name}. It's on the clipboard."
        ));
    }
    Ok(())
}

/// Brings the app `pid` to the front and pastes `text` there. Blocks for
/// up to a couple of seconds.
pub(crate) fn paste_into(pid: i32, text: &str) -> Result<(), String> {
    splashdown_platform::activate_app(pid)
        .map_err(|e| e.to_string())
        .and_then(|()| wait_for_front(pid))
        .and_then(|()| {
            std::thread::sleep(SETTLE);
            splashdown_platform::insert_text(text, InsertMethod::Paste).map_err(|e| e.to_string())
        })
}

fn wait_for_front(pid: i32) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < ACTIVATE_TIMEOUT {
        if splashdown_platform::frontmost_app().is_some_and(|(p, _, _)| p == pid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err("the app didn't come to the front".into())
}

// ---------------------------------------------------------------- commands

/// The folder, as Settings and the Notes window show it.
#[derive(Serialize)]
pub struct Folder {
    path: String,
    /// "Documents › Splashdown Notes".
    label: String,
    /// The sync service with a copy, e.g. "iCloud Drive".
    synced: Option<&'static str>,
    /// Why notes are somewhere else for now.
    problem: Option<String>,
}

fn folder_of(root: &Path, problem: Option<String>) -> Folder {
    Folder {
        path: root.display().to_string(),
        label: label(root),
        synced: cloud_service(root),
        problem,
    }
}

/// What the page shows when it opens.
#[derive(Serialize)]
pub struct View {
    folder: Folder,
    notes: Vec<Summary>,
    /// The note to show, unless it's time for a fresh one.
    open: Option<Note>,
    /// The app "Paste into" pastes into.
    target: Option<String>,
    /// The window stays above other apps' windows.
    on_top: bool,
}

fn view(state: &AppState) -> Result<View, String> {
    let (store, problem) = store(state)?;
    let fresh_after = FRESH_AFTER.as_millis() as u64;
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let open = store
        .last()
        .filter(|(_, at)| now.saturating_sub(*at) < fresh_after)
        .and_then(|(path, _)| store.open(&path).ok());
    Ok(View {
        folder: folder_of(store.root(), problem),
        notes: store.list(),
        open,
        target: target_name(),
        on_top: state.settings().notes.keep_on_top,
    })
}

/// Keeps the window above other apps' windows, or not, and remembers it.
#[tauri::command]
pub fn notes_keep_on_top(
    on: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WINDOW) {
        window.set_always_on_top(on).map_err(|e| e.to_string())?;
    }
    let mut settings = state.settings();
    settings.notes.keep_on_top = on;
    settings.save(&engine::paths()?.config_file())?;
    *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
    crate::commands::settings_changed_elsewhere(&app);
    Ok(())
}

#[tauri::command]
pub fn notes_load(state: State<'_, AppState>) -> Result<View, String> {
    view(&state)
}

/// Polled every few seconds; a big folder (or files a sync service has to
/// download first) takes a while to read, so not on the main thread.
#[tauri::command]
pub async fn notes_list(state: State<'_, AppState>) -> Result<Vec<Summary>, String> {
    let folder = state.settings().notes.folder;
    tauri::async_runtime::spawn_blocking(move || Ok(store_for(folder)?.0.list()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn notes_open(path: String, state: State<'_, AppState>) -> Result<Note, String> {
    store(&state)?.0.open(&path).map_err(|e| e.to_string())
}

/// Saves the editor's text: a new file the first time a note has any,
/// otherwise the note's own file. `None` for a new note that's still blank.
#[tauri::command]
pub fn notes_save(
    path: Option<String>,
    text: String,
    base: Option<u64>,
    folder: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<Saved>, String> {
    let store = store(&state)?.0;
    let saved = match path {
        Some(path) => store.save(&path, &text, base),
        None if text.trim().is_empty() => return Ok(None),
        None => {
            let app = target_name();
            // A new note goes in the folder being looked at.
            store
                .create_in(
                    folder.as_deref().unwrap_or(""),
                    &text,
                    chrono::Local::now(),
                    &Origin {
                        app: app.as_deref(),
                        ..Origin::default()
                    },
                )
                .map(|note| Saved {
                    note,
                    conflict: false,
                })
        }
    };
    saved.map(Some).map_err(|e| e.to_string())
}

/// Starts a new note: the next save creates its file.
#[tauri::command]
pub fn notes_new(state: State<'_, AppState>) -> Result<(), String> {
    store(&state)?.0.set_last(None);
    Ok(())
}

#[tauri::command]
pub fn notes_delete(path: String, state: State<'_, AppState>) -> Result<(), String> {
    not_recording(&path)?;
    store(&state)?.0.delete(&path)
}

/// Today's daily note (in the folder and with the name set for captures
/// in Settings → Notes), made if it isn't there yet.
#[tauri::command]
pub fn notes_daily(state: State<'_, AppState>) -> Result<Note, String> {
    let settings = state.settings().notes;
    let (store, _) = store(&state)?;
    store
        .daily(
            &Target::Daily {
                folder: settings.daily_folder().to_string(),
                format: settings.daily_format().to_string(),
            },
            chrono::Local::now(),
        )
        .map_err(|e| e.to_string())
}

/// Neither a meeting nor a file's transcript is being written into a note
/// in `folder`, so it can be moved or trashed.
fn nothing_written_in(folder: &str) -> Result<(), String> {
    let inside = |note: &str| note.starts_with(&format!("{folder}/"));
    if crate::meeting::status().is_some_and(|m| inside(&m.note)) {
        return Err(
            "A recording is being written into a note in this folder. Stop it first.".into(),
        );
    }
    if files::notes_transcribing().is_some_and(|j| inside(&j.note)) {
        return Err("A file is being transcribed into a note in this folder. Wait for it, or stop it first.".into());
    }
    Ok(())
}

/// The subfolders of the notes folder, for the sidebar.
#[tauri::command]
pub async fn notes_folders(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let (store, _) = store(&state)?;
    tauri::async_runtime::spawn_blocking(move || store.folders())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn notes_create_folder(
    parent: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    store(&state)?
        .0
        .create_folder(&parent, &name)
        .map_err(|e| e.to_string())
}

/// Renames a folder (`parent` unchanged) or moves it into another.
#[tauri::command]
pub fn notes_move_folder(
    path: String,
    parent: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    nothing_written_in(&path)?;
    store(&state)?
        .0
        .move_folder(&path, &parent, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn notes_delete_folder(path: String, state: State<'_, AppState>) -> Result<(), String> {
    nothing_written_in(&path)?;
    store(&state)?.0.delete_folder(&path)
}

/// Moves a note into `folder` ("" for the top). Returns its new path.
#[tauri::command]
pub fn notes_move(
    path: String,
    folder: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    not_recording(&path)?;
    if files::notes_transcribing().is_some_and(|j| j.note == path) {
        return Err("This note is being transcribed into. Wait for it, or stop it first.".into());
    }
    store(&state)?
        .0
        .move_note(&path, &folder)
        .map_err(|e| e.to_string())
}

/// Sets the tags in a note's front matter.
#[tauri::command]
pub fn notes_set_tags(
    path: String,
    tags: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Note, String> {
    store(&state)?
        .0
        .set_front_tags(&path, &tags)
        .map_err(|e| e.to_string())
}

/// A meeting writes into its note as it goes: it can't be moved meanwhile.
fn not_recording(path: &str) -> Result<(), String> {
    match crate::meeting::status() {
        Some(meeting) if meeting.note == path => {
            Err("A meeting is being written into this note. Stop the meeting first.".into())
        }
        _ => Ok(()),
    }
}

#[tauri::command]
pub fn notes_pin(path: String, pinned: bool, state: State<'_, AppState>) -> Result<(), String> {
    store(&state)?.0.set_pinned(&path, pinned);
    Ok(())
}

#[tauri::command]
pub fn notes_rename(
    path: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    not_recording(&path)?;
    store(&state)?
        .0
        .rename(&path, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn notes_versions(path: String, state: State<'_, AppState>) -> Result<Vec<Version>, String> {
    store(&state)?.0.versions(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn notes_restore(path: String, at: u64, state: State<'_, AppState>) -> Result<Note, String> {
    store(&state)?
        .0
        .restore(&path, at)
        .map_err(|e| e.to_string())
}

/// Shows a note, or the notes folder, in Finder / File Explorer.
#[tauri::command]
pub fn notes_reveal(path: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    let store = store(&state)?.0;
    let target = match path {
        Some(p) => store.root().join(p),
        None => store.root().to_path_buf(),
    };
    splashdown_platform::reveal_in_file_manager(&target).map_err(|e| e.to_string())
}

/// The folder Settings shows. Doesn't create it, so opening Settings never
/// triggers the Documents prompt.
#[tauri::command]
pub fn notes_folder(state: State<'_, AppState>) -> Result<Folder, String> {
    let slot = current_slot().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = slot.as_ref() {
        return Ok(folder_of(c.store.root(), c.problem.clone()));
    }
    drop(slot);
    let wanted = wanted_folder(&state).ok_or("Couldn't find your Documents folder.")?;
    Ok(folder_of(&wanted, None))
}

/// A native folder picker, starting at the current folder. `None` if the
/// user cancelled.
#[tauri::command]
pub async fn notes_pick_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let start = wanted_folder(&state);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app
            .dialog()
            .file()
            .set_title("Choose a folder for your notes");
        if let Some(dir) = start.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        dialog.blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

/// Uses `folder` for notes from now on, moving the current notes there
/// first if asked. Returns the new view.
#[tauri::command]
pub fn notes_set_folder(
    folder: String,
    move_notes_too: bool,
    state: State<'_, AppState>,
) -> Result<View, String> {
    let folder = PathBuf::from(folder);
    Store::new(folder.clone())
        .ensure()
        .map_err(|e| format!("Can't use {}: {e}", label(&folder)))?;
    if move_notes_too {
        let (old, _) = store(&state)?;
        if old.root() != folder {
            let moved = move_notes(&old, &folder)?;
            tracing::info!("moved {moved} notes to the new folder");
        }
    }
    let mut settings = state.settings();
    settings.notes.folder = (Some(&folder) != default_folder().as_ref()).then_some(folder);
    settings.save(&engine::paths()?.config_file())?;
    *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
    *current_slot().lock().unwrap_or_else(|e| e.into_inner()) = None;
    view(&state)
}

/// A transform as the Notes menu shows it.
#[derive(Serialize)]
pub struct TransformItem {
    id: String,
    name: String,
    place: splashdown_config::Place,
    shortcut: Option<String>,
}

#[tauri::command]
pub fn notes_transforms(state: State<'_, AppState>) -> Vec<TransformItem> {
    state
        .settings()
        .transforms()
        .into_iter()
        .map(|t| TransformItem {
            id: t.id,
            name: t.name,
            place: t.place,
            shortcut: t.shortcut,
        })
        .collect()
}

/// Runs transform `id` on `text` (the note, or the selection in it). The
/// note as it is on disk is kept as a version first, so the change can
/// always be undone from History. The page puts the result in place.
#[tauri::command]
pub async fn notes_transform(
    path: Option<String>,
    id: String,
    text: String,
    extra: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::session::TransformOutcome, String> {
    let session = state.session.clone();
    let out = tauri::async_runtime::spawn_blocking(move || session.transform_with(id, text, extra))
        .await
        .map_err(|e| e.to_string())??;
    if out.unchanged.is_none()
        && let Some(path) = path
    {
        let (store, _) = store(&state)?;
        if let Err(e) = store.keep_current(&path) {
            tracing::warn!("couldn't keep a version before the transform: {e}");
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- ask

/// App event to the page with a question's answer so far (`AskUpdate`).
const ASKED: &str = "notes-ask";
/// Most notes a question about all of them is answered from, and how much
/// of each.
const NOTES_ASK_NOTES: usize = 5;
const NOTES_ASK_EXCERPT: usize = 2000;
/// How much of one note a question about it may use.
const ONE_NOTE_CHARS: usize = 24_000;

/// A note an answer may cite, as `[n]`: `n` is its place in the list, from 1.
#[derive(Debug, Clone, Serialize)]
pub struct Source {
    title: String,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct AskUpdate {
    /// The page's number for the question, so an older answer still coming
    /// in is told apart.
    asked: u64,
    answer: String,
    done: bool,
    error: Option<String>,
    sources: Vec<Source>,
}

/// A question asked in the Notes window, about the note at `path` or (with
/// none) the notes that match it. The answer streams to the page as
/// `notes-ask` events. `recent` asks about the end of a long note (a
/// meeting still going); `earlier` holds this conversation's questions and
/// answers so far.
#[tauri::command]
pub async fn notes_ask(
    question: String,
    path: Option<String>,
    recent: bool,
    earlier: Vec<(String, String)>,
    asked: u64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Ok(());
    }
    let session = state.session.clone();
    let setup = tauri::async_runtime::spawn_blocking(move || session.answer_model())
        .await
        .map_err(|e| e.to_string())??
        .ok_or("Asking needs an AI model: choose one in Settings → Providers.")?;
    let (store, _) = store(&state)?;
    let (notes, sources): (Vec<_>, Vec<_>) = match &path {
        Some(path) => {
            let note = store.read(path).map_err(|e| e.to_string())?;
            let title = store::title_of(&note.body);
            let excerpt = splashdown_core::ask::NoteExcerpt {
                title: title.clone(),
                date: day_of(note.modified),
                text: clip(&note.body, ONE_NOTE_CHARS, recent),
            };
            let source = Source {
                title,
                path: path.clone(),
            };
            (vec![excerpt], vec![source])
        }
        None => {
            // Finding notes by meaning can take a moment.
            let by_meaning = state.settings().notes.ask_by_meaning;
            let asked = question.clone();
            tauri::async_runtime::spawn_blocking(move || {
                matching(
                    &store,
                    &asked,
                    NOTES_ASK_NOTES,
                    NOTES_ASK_EXCERPT,
                    by_meaning,
                )
            })
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(path, excerpt)| {
                let source = Source {
                    title: excerpt.title.clone(),
                    path,
                };
                (excerpt, source)
            })
            .unzip()
        }
    };
    tracing::info!("asking in Notes, with {} note(s)", notes.len());
    let context = splashdown_core::ask::AskContext {
        notes,
        user_name: setup.user_name,
        today: chrono::Local::now().format("%A, %-d %B %Y").to_string(),
        earlier,
        cite_numbers: true,
        ..Default::default()
    };
    let request = splashdown_core::ask::request(&question, &context);
    let model = setup.model;
    tauri::async_runtime::spawn(async move {
        let emit = |answer: &str, done: bool, error: Option<String>| {
            let _ = app.emit_to(
                WINDOW,
                ASKED,
                AskUpdate {
                    asked,
                    answer: answer.to_string(),
                    done,
                    error,
                    sources: sources.clone(),
                },
            );
        };
        let mut on_text = |so_far: &str| emit(so_far, false, None);
        let result = model.complete_streaming(&request, &mut on_text).await;
        let usage = match &result {
            Ok(completion) => Some(completion.usage),
            Err(splashdown_core::ProviderError::Interrupted { usage, .. }) => Some(*usage),
            Err(_) => None,
        };
        if let Some(usage) = usage {
            crate::spend::shared().record(&[(model.name(), usage)], chrono::Local::now());
        }
        match result {
            Ok(completion) => emit(completion.text.trim(), true, None),
            Err(e) => {
                tracing::warn!("asking in Notes failed: {e}");
                let message = crate::errors::pipeline(
                    &splashdown_core::PipelineError::Llm(e),
                    None,
                    Some(&model.name()),
                );
                emit("", true, Some(message));
            }
        }
    });
    Ok(())
}

/// Pastes can block for a few hundred ms, so this runs off the main thread.
#[tauri::command]
pub async fn notes_send(text: String, app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || send(&app, &text))
        .await
        .map_err(|e| e.to_string())?
}

/// History → "Save to notes": adds the text to the capture note. Returns
/// the note's name.
#[tauri::command]
pub fn notes_add(text: String, state: State<'_, AppState>) -> Result<String, String> {
    capture(&state.settings().notes, None, &text, None).map(|c| c.label)
}

/// "Edit transforms…": Settings, on the tab that lists them.
#[tauri::command]
pub fn notes_edit_transforms(app: AppHandle) {
    crate::tray::open_settings_tab(&app, "words");
}

/// From Settings → Privacy.
/// Brings the notes' vectors up to date in the background, when Ask finds
/// notes by meaning: changed notes are then ready by the time they're
/// asked about.
fn read_for_meaning(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let settings = state.settings().notes;
    if !settings.ask_by_meaning || !meaning::available() {
        return;
    }
    if let Ok((store, _)) = store_for(settings.folder) {
        meaning::update_soon(store);
    }
}

/// How far along reading the notes for meaning is, for Settings: notes
/// read, of all of them (and starts reading any that aren't).
#[tauri::command]
pub async fn notes_meaning_progress(app: AppHandle) -> Result<(usize, usize), String> {
    read_for_meaning(&app);
    let state = app.state::<AppState>();
    let (store, _) = store(&state)?;
    tauri::async_runtime::spawn_blocking(move || meaning::progress(store.root(), &store.list()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_notes(app: AppHandle) {
    open(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(title: &str, text: &str, modified: u64) -> Summary {
        Summary {
            path: format!("{title}.md"),
            title: title.into(),
            text: text.into(),
            modified,
            pinned: false,
            folder: String::new(),
            source: String::new(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn questions_find_the_notes_that_match() {
        let notes = [
            summary(
                "Launch plan",
                "We moved the launch to Thursday. The docs are late.",
                1,
            ),
            summary("Groceries", "Milk, eggs and the bread from the bakery.", 2),
            summary(
                "Standup",
                "The onboarding bug is fixed. The launch is close.",
                3,
            ),
            summary("Ideas", "The app could read the clipboard.", 4),
            summary("Recipes", "The best bread needs time.", 5),
        ];
        let found = best_matches(&notes, "When is the launch? What about the docs?", 3);
        let titles: Vec<&str> = found.iter().map(|(n, _)| n.title.as_str()).collect();
        // "the" is in every note and counts for nothing; "docs" is rarest.
        assert_eq!(titles, ["Launch plan", "Standup"]);
        assert_eq!(found[0].1, "docs");
        assert!(best_matches(&notes, "the", 3).is_empty());
        // Whole words: "stand" isn't in "Standup".
        assert!(best_matches(&notes, "stand", 3).is_empty());
        let found = best_matches(&notes, "eggs", 3);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.title, "Groceries");
        assert!(best_matches(&notes, "", 3).is_empty());
        // A single note still matches.
        let one = [summary("Launch plan", "Thursday.", 1)];
        assert_eq!(best_matches(&one, "launch day", 3).len(), 1);
    }

    #[test]
    fn common_words_and_unspaced_scripts() {
        let w = words_of("What did we decide about the Launch?");
        assert!(w.contains("decide") && w.contains("launch"));
        assert!(!w.contains("what") && !w.contains("the") && !w.contains("about"));
        // Japanese: pairs of characters.
        let w = words_of("発売日はいつ");
        assert!(w.contains("発売") && w.contains("売日"));
        // With a single note, common words don't make it match.
        let one = [summary("Weather", "What is the forecast", 1)];
        assert!(best_matches(&one, "what's the time", 3).is_empty());
    }

    #[test]
    fn excerpts_are_cut_around_the_word() {
        let text = format!("{} needle {}", "a".repeat(2000), "b".repeat(2000));
        let cut = around(&text, "needle", 300);
        assert!(cut.starts_with('…') && cut.ends_with('…'));
        assert!(cut.contains("needle"));
        assert_eq!(cut.chars().count(), 302);
        assert_eq!(around("short", "x", 300), "short");
        // The whole word, not part of a longer one.
        let text = format!("start {} the art show", "x".repeat(2000));
        assert!(around(&text, "art", 300).contains("art show"));
    }

    #[test]
    fn moving_notes_never_overwrites() {
        let base =
            std::env::temp_dir().join(format!("splashdown-notes-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let from = Store::new(base.join("from"));
        from.ensure().unwrap();
        std::fs::create_dir_all(base.join("from/Work")).unwrap();
        std::fs::write(base.join("from/Plan.md"), "Plan").unwrap();
        std::fs::write(base.join("from/Work/Standup.md"), "Standup").unwrap();
        std::fs::create_dir_all(base.join("from/.splashdown/versions/1-ab")).unwrap();
        std::fs::write(base.join("from/.splashdown/versions/1-ab/5.md"), "old").unwrap();
        std::fs::write(base.join("from/.splashdown/state.json"), "{}").unwrap();
        std::fs::create_dir_all(base.join("from/Attachments")).unwrap();
        std::fs::write(base.join("from/Attachments/a.wav"), "RIFF").unwrap();
        // The new folder is set up before the move, as Notes does.
        Store::new(base.join("to")).ensure().unwrap();
        std::fs::write(base.join("to/Plan.md"), "Theirs").unwrap();

        assert_eq!(move_notes(&from, &base.join("to")).unwrap(), 2);
        assert!(base.join("to/.splashdown/versions/1-ab/5.md").exists());
        assert!(base.join("to/.splashdown/state.json").exists());
        assert!(base.join("to/Attachments/a.wav").exists());
        assert_eq!(
            std::fs::read_to_string(base.join("to/Plan.md")).unwrap(),
            "Theirs"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("to/Plan 2.md")).unwrap(),
            "Plan"
        );
        assert!(base.join("to/Work/Standup.md").exists());
        assert!(base.join("to/.splashdown").is_dir());
        assert!(from.list().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn labels_inside_home_use_arrows() {
        let home = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf();
        assert_eq!(
            label(&home.join("Documents/Splashdown Notes")),
            "Documents › Splashdown Notes"
        );
        assert_eq!(
            cloud_service(&home.join("Library/Mobile Documents/x")).is_some(),
            cfg!(target_os = "macos")
        );
    }
}
