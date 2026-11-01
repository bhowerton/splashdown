//! Notes as Markdown files in a folder the user picks: one `.md`
//! file per note, plus a `.splashdown/` folder beside them for earlier
//! versions and pins. There's no database: the folder is the truth, and
//! other apps (Obsidian, a sync service) may change it at any time.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};

/// Inside the notes folder; hidden by Finder, Explorer and Obsidian.
const META_DIR: &str = ".splashdown";
/// Recordings kept with notes; never listed as notes.
pub const ATTACHMENTS_DIR: &str = "Attachments";
/// Earlier versions kept per note.
const KEEP_VERSIONS: usize = 50;
/// Characters of a title used in a file name.
const NAME_CHARS: usize = 60;
/// Guards against walking a huge folder (a whole drive picked by mistake).
const MAX_NOTES: usize = 5000;
const MAX_DEPTH: usize = 6;

/// A note in the list.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    /// Relative to the notes folder, with `/` separators.
    pub path: String,
    pub title: String,
    /// The note's text without its front matter, for search.
    pub text: String,
    /// Last change, in Unix ms.
    pub modified: u64,
    pub pinned: bool,
    /// The subfolder it's in ("" at the top).
    pub folder: String,
    /// How it was made, from its front matter: "meeting", "file",
    /// "capture"… ("" when it doesn't say).
    pub source: String,
}

/// A note opened in the editor.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Note {
    pub path: String,
    /// The text without front matter (that's kept as it is on save).
    pub body: String,
    pub modified: u64,
    /// As in `Summary`.
    pub source: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Saved {
    pub note: Note,
    /// The file had been changed by another app since it was opened. Ours
    /// was written; theirs is kept as a version.
    pub conflict: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Version {
    /// When it was saved, in Unix ms (also its file name).
    pub at: u64,
    pub preview: String,
}

/// Pins and the last note opened, in `.splashdown/state.json`, so they
/// travel with the folder.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    pinned: Vec<String>,
    last: Option<String>,
    last_at: u64,
}

/// Where a captured dictation is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `Inbox.md` at the top of the folder.
    Inbox,
    /// Today's daily note: `folder/<format>.md`, the format using YYYY, MM
    /// and DD.
    Daily { folder: String, format: String },
    /// The note last opened, or the inbox if there's none.
    Last,
    /// A note by path.
    Note(String),
}

/// Text added to the end of a note, so it can be taken back.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Appended {
    pub path: String,
    /// Exactly what was added, separator included.
    pub added: String,
    /// The file's time after adding, to tell whether it's changed since.
    pub modified: u64,
}

/// Facts about a new note, for its front matter.
#[derive(Debug, Default, Clone)]
pub struct Origin<'a> {
    /// The app in front when it was started.
    pub app: Option<&'a str>,
    /// How it was made, when known: "scratchpad", "capture", "file".
    pub source: Option<&'a str>,
    /// The audio or video file it was transcribed from (its name).
    pub file: Option<&'a str>,
}

pub struct Store {
    root: PathBuf,
    /// Parsed files by path, reused while their size and time match.
    cache: Mutex<HashMap<PathBuf, (u64, u64, Summary)>>,
    /// Notes opened since their last version was taken: the next save
    /// keeps what was on disk first.
    needs_version: Mutex<HashSet<String>>,
    /// Held while text is added to the end of a note, so a capture and a
    /// file transcript's next paragraph never write over each other.
    appending: Mutex<()>,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            cache: Mutex::default(),
            needs_version: Mutex::default(),
            appending: Mutex::default(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Creates the folder (and `.splashdown/`) if needed, and checks a file
    /// can be written there.
    pub fn ensure(&self) -> io::Result<()> {
        std::fs::create_dir_all(self.meta())?;
        let probe = self.meta().join(format!("probe-{}", std::process::id()));
        std::fs::write(&probe, b"")?;
        std::fs::remove_file(probe)
    }

    fn meta(&self) -> PathBuf {
        self.root.join(META_DIR)
    }

    // ------------------------------------------------------------ listing

    /// Every note, pinned first, then newest first.
    pub fn list(&self) -> Vec<Summary> {
        let pinned: HashSet<String> = self.state().pinned.into_iter().collect();
        let mut files = Vec::new();
        walk(&self.root, 0, &mut files);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut seen = HashSet::new();
        let mut notes: Vec<Summary> = files
            .into_iter()
            .filter_map(|path| {
                let meta = std::fs::metadata(&path).ok()?;
                let modified = mtime_ms(&meta);
                seen.insert(path.clone());
                let summary = match cache.get(&path) {
                    Some((len, time, s)) if *len == meta.len() && *time == modified => s.clone(),
                    _ => {
                        let text = std::fs::read_to_string(&path).ok()?;
                        let s = self.summarize(&path, &text, modified)?;
                        cache.insert(path.clone(), (meta.len(), modified, s.clone()));
                        s
                    }
                };
                Some(Summary {
                    pinned: pinned.contains(&summary.path),
                    ..summary
                })
            })
            .collect();
        cache.retain(|p, _| seen.contains(p));
        notes.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.modified.cmp(&a.modified))
                .then(a.path.cmp(&b.path))
        });
        notes
    }

    fn summarize(&self, path: &Path, text: &str, modified: u64) -> Option<Summary> {
        let rel = self.relative(path)?;
        let (front, body) = split_front_matter(text);
        Some(Summary {
            source: front_matter_value(front, "source").unwrap_or_default(),
            title: title_of(body),
            text: body.to_string(),
            modified,
            pinned: false,
            folder: rel
                .rsplit_once('/')
                .map(|(dir, _)| dir.to_string())
                .unwrap_or_default(),
            path: rel,
        })
    }

    fn relative(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let parts: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        Some(parts.join("/"))
    }

    /// The file for `rel`, refusing anything outside the notes folder:
    /// absolute paths, `..`, and the `.splashdown/` folder.
    fn resolve(&self, rel: &str) -> io::Result<PathBuf> {
        let path = Path::new(rel);
        // Only notes: never another file that happens to be in the folder
        // (an attachment, a vault's images), which reading as a note could
        // overwrite.
        let ok = !rel.is_empty()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            && path.components().all(|c| match c {
                Component::Normal(name) => !name.to_string_lossy().starts_with('.'),
                _ => false,
            })
            && path
                .components()
                .next()
                .is_none_or(|c| c.as_os_str() != ATTACHMENTS_DIR);
        if !ok {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a note"));
        }
        let full = self.root.join(path);
        // A symlink inside the folder mustn't lead out of it, including
        // through a folder on the way to a note that doesn't exist yet.
        let existing = full.ancestors().find(|p| p.exists()).unwrap_or(&self.root);
        if let (Ok(real), Ok(root)) = (existing.canonicalize(), self.root.canonicalize())
            && !real.starts_with(&root)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "outside the notes folder",
            ));
        }
        Ok(full)
    }

    // ------------------------------------------------------------ reading and writing

    /// Opens a note for editing. Its first change afterwards keeps the
    /// text as it was in a version.
    pub fn open(&self, rel: &str) -> io::Result<Note> {
        let note = self.read(rel)?;
        self.needs_version
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(rel.to_string());
        self.set_last(Some(rel));
        Ok(note)
    }

    pub fn read(&self, rel: &str) -> io::Result<Note> {
        let path = self.resolve(rel)?;
        let text = std::fs::read_to_string(&path)?;
        let (front, body) = split_front_matter(&text);
        Ok(Note {
            source: front_matter_value(front, "source").unwrap_or_default(),
            path: rel.to_string(),
            body: body.replace("\r\n", "\n"),
            modified: mtime_ms(&std::fs::metadata(&path)?),
        })
    }

    /// A new note holding `body`, named after its first line.
    pub fn create(
        &self,
        body: &str,
        now: DateTime<Local>,
        origin: &Origin<'_>,
    ) -> io::Result<Note> {
        self.create_in("", body, now, origin)
    }

    /// `create` in subfolder `folder` of the notes folder ("" for the top),
    /// made if needed.
    pub fn create_in(
        &self,
        folder: &str,
        body: &str,
        now: DateTime<Local>,
        origin: &Origin<'_>,
    ) -> io::Result<Note> {
        self.ensure()?;
        let parts: Vec<String> = folder
            .split('/')
            .map(file_name_part)
            .filter(|p| !p.is_empty())
            .collect();
        let dir = parts.iter().fold(self.root.clone(), |d, p| d.join(p));
        std::fs::create_dir_all(&dir)?;
        let path = self.new_path(&dir, body, now);
        let mut front = format!(
            "---\nsplashdown-id: {}\ncreated: {}\n",
            new_id(now),
            now.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
        );
        if let Some(source) = origin.source {
            front.push_str(&format!("source: {source}\n"));
        }
        if let Some(app) = origin.app.filter(|a| !a.is_empty()) {
            front.push_str(&format!("app: {}\n", yaml_scalar(app)));
        }
        if let Some(file) = origin.file.filter(|f| !f.is_empty()) {
            front.push_str(&format!("file: {}\n", yaml_scalar(file)));
        }
        front.push_str("---\n");
        self.write(&path, &format!("{front}{body}"))?;
        let rel = self.relative(&path).expect("inside the folder");
        self.set_last(Some(&rel));
        self.read(&rel)
    }

    /// Saves the editor's text. `base` is the file's time when the editor
    /// loaded it: if it changed since (another app), that version is kept
    /// before ours is written.
    pub fn save(&self, rel: &str, body: &str, base: Option<u64>) -> io::Result<Saved> {
        let path = self.resolve(rel)?;
        // Missing is fine (it's put back); unreadable isn't: writing over it
        // would lose it.
        let on_disk = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let changed_elsewhere = match (&on_disk, base) {
            (Some(_), Some(base)) => {
                std::fs::metadata(&path).map(|m| mtime_ms(&m)).ok() != Some(base)
            }
            _ => false,
        };
        let mut conflict = false;
        if let Some(text) = &on_disk {
            let (front, old_body) = split_front_matter(text);
            if old_body.replace("\r\n", "\n") == body {
                return Ok(Saved {
                    note: self.read(rel)?,
                    conflict: false,
                });
            }
            let first_change = self
                .needs_version
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(rel);
            if first_change || changed_elsewhere {
                self.keep_version(&path, text)?;
                conflict = changed_elsewhere;
            }
            let body = if text.contains("\r\n") {
                body.replace('\n', "\r\n")
            } else {
                body.to_string()
            };
            self.write(&path, &format!("{front}{body}"))?;
        } else {
            // Deleted or moved elsewhere meanwhile: put it back.
            self.write(&path, body)?;
        }
        Ok(Saved {
            note: self.read(rel)?,
            conflict,
        })
    }

    /// Writes through a temporary file in `.splashdown/`, so a crash or a
    /// full disk never leaves half a note.
    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::create_dir_all(self.meta())?;
        let tmp = self.meta().join(format!(
            "saving-{}-{}.md",
            std::process::id(),
            unique_suffix()
        ));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// `YYYY-MM-DD <first line>.md` in `dir`, numbered if taken; untitled
    /// notes get the time instead.
    fn new_path(&self, dir: &Path, body: &str, now: DateTime<Local>) -> PathBuf {
        let title = file_name_part(&title_of(body));
        let stem = if title.is_empty() {
            now.format("%Y-%m-%d %H%M").to_string()
        } else {
            format!("{} {title}", now.format("%Y-%m-%d"))
        };
        unique(dir, &stem)
    }

    /// Gives a note a new file name (in the same folder). Returns its path.
    pub fn rename(&self, rel: &str, name: &str) -> io::Result<String> {
        let path = self.resolve(rel)?;
        let stem = file_name_part(name);
        if stem.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Type a name first.",
            ));
        }
        let dir = path.parent().unwrap_or(&self.root);
        let current = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if current == stem {
            return Ok(rel.to_string());
        }
        // Only the case changes ("plan" → "Plan"): the file system sees the
        // same name, so it isn't taken.
        let target = if current.to_lowercase() == stem.to_lowercase() {
            dir.join(format!("{stem}.md"))
        } else {
            unique(dir, &stem)
        };
        std::fs::rename(&path, &target)?;
        let new_rel = self.relative(&target).expect("inside the folder");
        let mut state = self.state();
        for p in state.pinned.iter_mut().chain(state.last.iter_mut()) {
            if p == rel {
                p.clone_from(&new_rel);
            }
        }
        self.save_state(&state);
        self.move_versions(rel, &new_rel, &target);
        Ok(new_rel)
    }

    /// Moves a note to the Trash (Recycle Bin); it's never deleted outright.
    /// Its versions stay in `.splashdown/`.
    pub fn delete(&self, rel: &str) -> Result<(), String> {
        let path = self.resolve(rel).map_err(|e| e.to_string())?;
        #[allow(unused_mut)]
        let mut trash = trash::TrashContext::default();
        // The default asks Finder through AppleScript, which a signed app
        // can't do without an Automation permission it doesn't ask for.
        #[cfg(target_os = "macos")]
        {
            use trash::macos::{DeleteMethod, TrashContextExtMacos};
            trash.set_delete_method(DeleteMethod::NsFileManager);
        }
        trash
            .delete(&path)
            .map_err(|e| format!("Couldn't move it to the Trash: {e}"))?;
        let mut state = self.state();
        state.pinned.retain(|p| p != rel);
        if state.last.as_deref() == Some(rel) {
            state.last = None;
        }
        self.save_state(&state);
        Ok(())
    }

    // ------------------------------------------------------------ versions

    fn versions_dir(&self, path: &Path, rel: &str) -> PathBuf {
        let key = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| front_matter_id(&t))
            .unwrap_or_else(|| format!("path-{:016x}", fnv1a(rel)));
        self.meta().join("versions").join(key)
    }

    fn keep_version(&self, path: &Path, text: &str) -> io::Result<()> {
        let rel = self.relative(path).unwrap_or_default();
        let dir = self.versions_dir(path, &rel);
        std::fs::create_dir_all(&dir)?;
        let mut at = chrono::Utc::now().timestamp_millis().max(0) as u64;
        while dir.join(format!("{at}.md")).exists() {
            at += 1;
        }
        std::fs::write(dir.join(format!("{at}.md")), text)?;
        let mut kept = version_times(&dir);
        kept.sort_unstable();
        let excess = kept.len().saturating_sub(KEEP_VERSIONS);
        for old in &kept[..excess] {
            let _ = std::fs::remove_file(dir.join(format!("{old}.md")));
        }
        Ok(())
    }

    /// Keeps the note as it is now as a version, before something else
    /// (a transform) changes it.
    pub fn keep_current(&self, rel: &str) -> io::Result<()> {
        let path = self.resolve(rel)?;
        let text = std::fs::read_to_string(&path)?;
        self.keep_version(&path, &text)
    }

    /// A note's earlier versions, newest first.
    pub fn versions(&self, rel: &str) -> io::Result<Vec<Version>> {
        let path = self.resolve(rel)?;
        let dir = self.versions_dir(&path, rel);
        let mut times = version_times(&dir);
        times.sort_unstable_by(|a, b| b.cmp(a));
        Ok(times
            .into_iter()
            .filter_map(|at| {
                let text = std::fs::read_to_string(dir.join(format!("{at}.md"))).ok()?;
                let (_, body) = split_front_matter(&text);
                Some(Version {
                    at,
                    preview: body.chars().take(400).collect(),
                })
            })
            .collect())
    }

    /// Puts an earlier version back. What's there now becomes a version
    /// first, so restoring can itself be undone.
    pub fn restore(&self, rel: &str, at: u64) -> io::Result<Note> {
        let path = self.resolve(rel)?;
        let dir = self.versions_dir(&path, rel);
        let old = std::fs::read_to_string(dir.join(format!("{at}.md")))?;
        let current = std::fs::read_to_string(&path)?;
        self.keep_version(&path, &current)?;
        let (front, _) = split_front_matter(&current);
        let (_, body) = split_front_matter(&old);
        self.write(&path, &format!("{front}{body}"))?;
        self.read(rel)
    }

    /// After a rename, versions keyed by the old path follow the note.
    fn move_versions(&self, old_rel: &str, new_rel: &str, new_path: &Path) {
        let has_id = std::fs::read_to_string(new_path)
            .ok()
            .and_then(|t| front_matter_id(&t))
            .is_some();
        if has_id {
            return;
        }
        let versions = self.meta().join("versions");
        let _ = std::fs::rename(
            versions.join(format!("path-{:016x}", fnv1a(old_rel))),
            versions.join(format!("path-{:016x}", fnv1a(new_rel))),
        );
    }

    // ------------------------------------------------------------ pins and state

    fn state(&self) -> State {
        std::fs::read_to_string(self.meta().join("state.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save_state(&self, state: &State) {
        let json = serde_json::to_string_pretty(state).expect("notes state serialize");
        if let Err(e) = self.write(&self.meta().join("state.json"), &json) {
            tracing::warn!("couldn't save the notes state: {e}");
        }
    }

    pub fn set_pinned(&self, rel: &str, pinned: bool) {
        let mut state = self.state();
        state.pinned.retain(|p| p != rel);
        if pinned {
            state.pinned.insert(0, rel.to_string());
        }
        self.save_state(&state);
    }

    /// The note last opened or created, and when (Unix ms).
    pub fn last(&self) -> Option<(String, u64)> {
        let state = self.state();
        state.last.map(|p| (p, state.last_at))
    }

    pub fn set_last(&self, rel: Option<&str>) {
        let mut state = self.state();
        state.last = rel.map(str::to_string);
        state.last_at = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.save_state(&state);
    }

    // ------------------------------------------------------------ capture

    /// The note a capture goes to.
    pub fn target_path(&self, target: &Target, now: DateTime<Local>) -> String {
        match target {
            Target::Inbox => "Inbox.md".to_string(),
            Target::Daily { folder, format } => {
                // A format with "/" (Obsidian's "YYYY/MM/YYYY-MM-DD") puts
                // the note in folders by date.
                let dated = daily_name(format, now);
                let (subfolders, name) = dated.rsplit_once('/').unwrap_or(("", &dated));
                let folder: Vec<String> = folder
                    .split('/')
                    .chain(subfolders.split('/'))
                    .map(file_name_part)
                    .filter(|s| !s.is_empty())
                    .collect();
                let name = file_name_part(name);
                let name = if name.is_empty() {
                    now.format("%Y-%m-%d").to_string()
                } else {
                    name
                };
                folder
                    .into_iter()
                    .chain(std::iter::once(format!("{name}.md")))
                    .collect::<Vec<_>>()
                    .join("/")
            }
            Target::Last => self
                .last()
                .map(|(p, _)| p)
                .filter(|p| self.resolve(p).is_ok_and(|f| f.exists()))
                .unwrap_or_else(|| "Inbox.md".to_string()),
            Target::Note(path) => path.clone(),
        }
    }

    /// Adds `text` at the end of the target note as its own paragraph,
    /// starting with the time (`**09:15** …`), creating the note if needed.
    pub fn append(
        &self,
        target: &Target,
        text: &str,
        now: DateTime<Local>,
    ) -> io::Result<Appended> {
        let _appending = self.appending.lock().unwrap_or_else(|e| e.into_inner());
        self.ensure()?;
        let rel = self.target_path(target, now);
        let path = self.resolve(&rel)?;
        let existing = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            // Never write over a note that can't be read.
            Err(e) => return Err(e),
        };
        let existing = if existing.trim().is_empty() {
            // A new note starts with a heading: its title in the list.
            match target {
                Target::Daily { .. } => format!("# {}\n", now.format("%Y-%m-%d")),
                _ => "# Inbox\n".to_string(),
            }
        } else {
            existing
        };
        let separator = if existing.ends_with("\n\n") {
            ""
        } else if existing.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let added = format!("{separator}**{}** {}\n", now.format("%H:%M"), text.trim());
        self.write(&path, &format!("{existing}{added}"))?;
        Ok(Appended {
            modified: mtime_ms(&std::fs::metadata(&path)?),
            path: rel,
            added,
        })
    }

    /// Takes back what `append` added, if the note hasn't changed since.
    /// Returns whether it did.
    pub fn undo_append(&self, appended: &Appended) -> io::Result<bool> {
        let _appending = self.appending.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.resolve(&appended.path)?;
        let text = std::fs::read_to_string(&path)?;
        let unchanged = mtime_ms(&std::fs::metadata(&path)?) == appended.modified;
        let Some(before) = text.strip_suffix(&appended.added).filter(|_| unchanged) else {
            return Ok(false);
        };
        self.write(&path, before)?;
        Ok(true)
    }

    /// Adds `text` to the end of note `rel` as its own paragraph, as is
    /// (a file transcript's next piece).
    pub fn add_paragraph(&self, rel: &str, text: &str) -> io::Result<()> {
        let _appending = self.appending.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.resolve(rel)?;
        let existing = std::fs::read_to_string(&path)?;
        let separator = if existing.ends_with("\n\n") {
            ""
        } else if existing.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        self.write(&path, &format!("{existing}{separator}{}\n", text.trim()))
    }

    /// Puts `summary` under `## Summary` above the transcript of note `rel`
    /// (before its first `**[` paragraph), with `## Transcript` after it.
    pub fn insert_summary(&self, rel: &str, summary: &str) -> io::Result<()> {
        let _appending = self.appending.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.resolve(rel)?;
        let text = std::fs::read_to_string(&path)?;
        let at = transcript_start(&text).unwrap_or(text.len());
        let (before, after) = text.split_at(at);
        let before = before.trim_end();
        let block = format!("\n\n## Summary\n\n{}\n\n## Transcript\n\n", summary.trim());
        self.write(&path, &format!("{before}{block}{after}"))
    }

    /// Puts `text` under `## {heading}` in note `rel` (a meeting's own
    /// notes, written while the transcript grows below them).
    pub fn set_section(&self, rel: &str, heading: &str, text: &str) -> io::Result<()> {
        let _appending = self.appending.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.resolve(rel)?;
        let file = std::fs::read_to_string(&path)?;
        let (front, body) = split_front_matter(&file);
        let body = with_section(&body.replace("\r\n", "\n"), heading, text);
        self.write(&path, &format!("{front}{body}"))
    }

    /// Saves a recording in `Attachments/` and returns its path in the
    /// folder, for a link.
    pub fn save_attachment(&self, stem: &str, extension: &str, bytes: &[u8]) -> io::Result<String> {
        let (path, rel) = self.new_attachment(stem, extension)?;
        std::fs::write(&path, bytes)?;
        Ok(rel)
    }

    /// A free file name in `Attachments/` for a recording to be written to:
    /// its full path, and its path in the folder for a link.
    pub fn new_attachment(&self, stem: &str, extension: &str) -> io::Result<(PathBuf, String)> {
        let dir = self.root.join(ATTACHMENTS_DIR);
        std::fs::create_dir_all(&dir)?;
        let stem = file_name_part(stem);
        let mut path = dir.join(format!("{stem}.{extension}"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{stem} {n}.{extension}"));
            n += 1;
        }
        let rel = self.relative(&path).expect("inside the folder");
        Ok((path, rel))
    }

    // ------------------------------------------------------------ the old scratchpad

    /// Turns the scratchpad's notes (`scratchpad.json`: the current one
    /// and up to 20 earlier) into files, keeping their times. Returns how
    /// many were made.
    pub fn import_scratchpad(&self, json: &str) -> io::Result<usize> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Old {
            text: String,
            at: u64,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Scratchpad {
            current: Old,
            earlier: Vec<Old>,
        }
        let pad: Scratchpad = serde_json::from_str(json)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut made = 0;
        // Oldest first, so the newest is the last note opened.
        for note in pad
            .earlier
            .iter()
            .rev()
            .chain(std::iter::once(&pad.current))
        {
            if note.text.trim().is_empty() {
                continue;
            }
            let at = Local
                .timestamp_millis_opt(i64::try_from(note.at).unwrap_or(0))
                .single()
                .unwrap_or_else(Local::now);
            self.create(
                &note.text,
                at,
                &Origin {
                    source: Some("scratchpad"),
                    ..Origin::default()
                },
            )?;
            made += 1;
        }
        Ok(made)
    }
}

// ---------------------------------------------------------------- helpers

/// A daily note's name from a format with YYYY, MM and DD (others are
/// kept as they are).
fn daily_name(format: &str, now: DateTime<Local>) -> String {
    format
        .replace("YYYY", &now.format("%Y").to_string())
        .replace("MM", &now.format("%m").to_string())
        .replace("DD", &now.format("%d").to_string())
}

/// `.md` files under `dir`, skipping dot folders and `Attachments/`.
fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_NOTES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || (depth == 0 && name == ATTACHMENTS_DIR) {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, depth + 1, out),
            Ok(_)
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md")) =>
            {
                out.push(path);
                if out.len() >= MAX_NOTES {
                    return;
                }
            }
            _ => {}
        }
    }
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

fn version_times(dir: &Path) -> Vec<u64> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .strip_suffix(".md")?
                        .parse()
                        .ok()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Splits `---` front matter from the body. The front matter is returned
/// with its closing line, so writing `front + body` restores the file.
pub fn split_front_matter(text: &str) -> (&str, &str) {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return ("", text);
    };
    let mut offset = text.len() - rest.len();
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end() == "---" {
            return text.split_at(offset);
        }
    }
    ("", text)
}

/// `key: value` from front matter, unquoted.
fn front_matter_value(front: &str, key: &str) -> Option<String> {
    front.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix(':')?.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// Whether `line` starts a part of a meeting note: a heading of its own
/// level or a transcript turn.
fn starts_part(line: &str) -> bool {
    let line = line.trim_end();
    is_turn(line)
        || line.strip_prefix("## ").is_some_and(|h| {
            ["My notes", "Summary", "Transcript"]
                .iter()
                .any(|known| h.trim().eq_ignore_ascii_case(known))
        })
}

/// A transcript turn: `**[0:04] You:** …` (a meeting) or `**[0:04]** …`
/// (a file). Not any line that happens to start with `**[`, which the
/// user may type in their own notes.
pub fn is_turn(line: &str) -> bool {
    let Some((stamp, rest)) = line.strip_prefix("**[").and_then(|l| l.split_once(']')) else {
        return false;
    };
    let parts: Vec<&str> = stamp.split(':').collect();
    let stamp_ok = (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        && parts[1..].iter().all(|p| p.len() == 2);
    stamp_ok
        && (rest.starts_with("**")
            || rest
                .strip_prefix(' ')
                .and_then(|r| r.split_once(":**"))
                .is_some_and(|(who, _)| !who.is_empty() && !who.contains('*')))
}

/// Where the transcript's first turn starts in `text`, if it has one.
pub fn transcript_start(text: &str) -> Option<usize> {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if is_turn(line) {
            return Some(at);
        }
        at += line.len();
    }
    None
}

/// What's under `## {heading}` in `body`, if it has that section.
pub fn section(body: &str, heading: &str) -> Option<String> {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let start = lines.iter().position(|l| {
        l.trim_end()
            .strip_prefix("## ")
            .is_some_and(|h| h.trim().eq_ignore_ascii_case(heading))
    })?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| starts_part(l))
        .map_or(lines.len(), |i| start + 1 + i);
    Some(lines[start + 1..end].concat().trim().to_string())
}

/// `body` with `text` under `## {heading}`, replacing what was there (up
/// to the next `## ` heading or transcript turn). A new section goes
/// before the summary or transcript, or at the end; empty `text` takes the
/// section out.
pub fn with_section(body: &str, heading: &str, text: &str) -> String {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let is_heading = |l: &str| {
        l.trim_end()
            .strip_prefix("## ")
            .is_some_and(|h| h.trim().eq_ignore_ascii_case(heading))
    };
    let text = text.trim();
    let block = if text.is_empty() {
        String::new()
    } else {
        format!("## {heading}\n\n{text}\n\n")
    };
    if let Some(start) = lines.iter().position(|l| is_heading(l)) {
        let end = lines[start + 1..]
            .iter()
            .position(|l| starts_part(l))
            .map_or(lines.len(), |i| start + 1 + i);
        let before = lines[..start].concat();
        let after = lines[end..].concat();
        let block = if after.is_empty() && !block.is_empty() {
            format!("{}\n", block.trim_end())
        } else {
            block
        };
        return format!("{before}{block}{after}");
    }
    if block.is_empty() {
        return body.to_string();
    }
    match lines.iter().position(|l| starts_part(l)) {
        Some(at) => {
            let before = lines[..at].concat();
            let before = before.trim_end();
            let gap = if before.is_empty() { "" } else { "\n\n" };
            format!("{before}{gap}{block}{}", lines[at..].concat())
        }
        None => {
            let before = body.trim_end();
            let gap = if before.is_empty() { "" } else { "\n\n" };
            format!("{before}{gap}{}\n", block.trim_end())
        }
    }
}

fn front_matter_id(text: &str) -> Option<String> {
    let (front, _) = split_front_matter(text);
    front.lines().find_map(|line| {
        let id = line.strip_prefix("splashdown-id:")?.trim();
        let ok = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        ok.then(|| id.to_string())
    })
}

/// The first line with text, without Markdown heading marks.
pub fn title_of(body: &str) -> String {
    body.lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
}

/// A title made safe as a file name on macOS and Windows: no `\ / : * ? "
/// < > |` or control characters, no trailing dots or spaces, not a
/// reserved Windows name, at most `NAME_CHARS` characters.
pub fn file_name_part(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "\\/:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut name: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(NAME_CHARS)
        .collect();
    name = name
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_string();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&name)) {
        name.push('_');
    }
    name
}

/// `dir/stem.md`, or `stem 2.md`, `stem 3.md`… if taken.
fn unique(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.md"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} {n}.md"));
        n += 1;
    }
    path
}

fn new_id(now: DateTime<Local>) -> String {
    format!(
        "{}-{:04x}",
        now.timestamp_millis(),
        unique_suffix() & 0xffff
    )
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as u64);
    nanos ^ NEXT.fetch_add(0x9e37, Ordering::Relaxed)
}

/// A YAML value that's safe whatever the app name contains.
fn yaml_scalar(s: &str) -> String {
    let plain = s.chars().all(|c| c.is_alphanumeric() || " .-_".contains(c)) && !s.starts_with(' ');
    if plain {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> Store {
        let root =
            std::env::temp_dir().join(format!("splashdown-notes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Store::new(root)
    }

    fn at(s: &str) -> DateTime<Local> {
        DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&Local)
    }

    #[test]
    fn file_names_are_safe_everywhere() {
        assert_eq!(
            file_name_part("Reply to Priya: re/Q3?"),
            "Reply to Priya re Q3"
        );
        assert_eq!(file_name_part("  ...hidden.  "), "hidden");
        assert_eq!(file_name_part("con"), "con_");
        assert_eq!(file_name_part(&"x".repeat(100)).len(), NAME_CHARS);
        assert_eq!(file_name_part("tab\there"), "tab here");
    }

    #[test]
    fn a_transcript_grows_a_paragraph_at_a_time() {
        let store = temp_store("transcript");
        let note = store
            .create(
                "# Interview\n\nTranscribed from **Interview.m4a**.\n",
                at("2026-10-31T09:15:00-05:00"),
                &Origin {
                    source: Some("file"),
                    file: Some("Interview.m4a"),
                    ..Origin::default()
                },
            )
            .unwrap();
        store
            .add_paragraph(&note.path, "**[0:00]** Hello.")
            .unwrap();
        store
            .add_paragraph(&note.path, " **[0:31]** Again. ")
            .unwrap();
        let raw = std::fs::read_to_string(store.root().join(&note.path)).unwrap();
        assert!(
            raw.contains("\nsource: file\nfile: Interview.m4a\n"),
            "{raw}"
        );
        assert!(
            raw.ends_with(
                "Transcribed from **Interview.m4a**.\n\n**[0:00]** Hello.\n\n**[0:31]** Again.\n"
            ),
            "{raw}"
        );

        // A meeting: in its own folder, with the summary put on top.
        let meeting = store
            .create_in(
                "Meetings",
                "# Zoom meeting\n\nRecorded on this computer.\n",
                at("2026-10-31T09:15:00-05:00"),
                &Origin::default(),
            )
            .unwrap();
        assert!(meeting.path.starts_with("Meetings/"), "{}", meeting.path);
        store
            .add_paragraph(&meeting.path, "**[0:00] Them:** Hi.")
            .unwrap();
        store
            .add_paragraph(&meeting.path, "*Ended at 0:05.*")
            .unwrap();
        store
            .insert_summary(&meeting.path, "### Overview\nA hello.")
            .unwrap();
        assert_eq!(
            store.read(&meeting.path).unwrap().body,
            "# Zoom meeting\n\nRecorded on this computer.\n\n## Summary\n\n### Overview\nA hello.\n\n## Transcript\n\n**[0:00] Them:** Hi.\n\n*Ended at 0:05.*\n"
        );
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_meetings_own_notes_sit_above_its_transcript() {
        let store = temp_store("sections");
        let meeting = store
            .create_in(
                "Meetings",
                "# Zoom meeting\n\nRecorded on this computer.\n",
                at("2026-10-31T09:15:00-05:00"),
                &Origin {
                    source: Some("meeting"),
                    ..Origin::default()
                },
            )
            .unwrap();
        assert_eq!(meeting.source, "meeting");
        assert_eq!(store.list()[0].source, "meeting");
        // Written before anyone speaks, then the transcript grows below.
        store
            .set_section(&meeting.path, "My notes", "- pricing?")
            .unwrap();
        store
            .add_paragraph(&meeting.path, "**[0:00] Them:** Hi.")
            .unwrap();
        store
            .set_section(
                &meeting.path,
                "My notes",
                "- pricing?\n- Sam: numbers by Thu",
            )
            .unwrap();
        store
            .add_paragraph(&meeting.path, "**[0:04] You:** Hello.")
            .unwrap();
        store
            .insert_summary(&meeting.path, "### Overview\nA hello.")
            .unwrap();
        assert_eq!(
            store.read(&meeting.path).unwrap().body,
            "# Zoom meeting\n\nRecorded on this computer.\n\n## My notes\n\n- pricing?\n- Sam: numbers by Thu\n\n## Summary\n\n### Overview\nA hello.\n\n## Transcript\n\n**[0:00] Them:** Hi.\n\n**[0:04] You:** Hello.\n"
        );
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn sections_are_added_replaced_and_taken_out() {
        let body = "# Call\n\n**[0:00] You:** Hi.\n";
        let with = with_section(body, "My notes", "one");
        assert_eq!(
            with,
            "# Call\n\n## My notes\n\none\n\n**[0:00] You:** Hi.\n"
        );
        assert_eq!(
            with_section(&with, "My notes", "two"),
            "# Call\n\n## My notes\n\ntwo\n\n**[0:00] You:** Hi.\n"
        );
        assert_eq!(with_section(&with, "My notes", "  "), body);
        assert_eq!(section(&with, "My notes").as_deref(), Some("one"));
        // What the user types can look like a heading or a turn.
        let mine = "Agenda:\n## Pricing\n**[x] call Sam";
        let typed = with_section(body, "My notes", mine);
        assert_eq!(with_section(&typed, "My notes", mine), typed);
        assert_eq!(section(&typed, "My notes").as_deref(), Some(mine));
        assert_eq!(transcript_start(&typed), typed.find("**[0:00]"));
        assert!(is_turn("**[1:02:30] Them:** Hi") && is_turn("**[0:04]** Hi"));
        assert!(!is_turn("**[x] call Sam") && !is_turn("**[0:4] You:** no"));
        assert_eq!(section(body, "My notes"), None);
        // Last in the note.
        let last = with_section("# Call\n", "My notes", "one");
        assert_eq!(last, "# Call\n\n## My notes\n\none\n");
        assert_eq!(
            with_section(&last, "My notes", "two"),
            "# Call\n\n## My notes\n\ntwo\n"
        );
        assert_eq!(
            front_matter_value("---\nsource: meeting\napp: \"Zoom\"\n---\n", "app").as_deref(),
            Some("Zoom")
        );
    }

    #[test]
    fn front_matter_round_trips() {
        let text = "---\nsplashdown-id: 1-ab\ncreated: x\n---\nHello\nthere";
        let (front, body) = split_front_matter(text);
        assert_eq!(body, "Hello\nthere");
        assert_eq!(format!("{front}{body}"), text);
        assert_eq!(front_matter_id(text).as_deref(), Some("1-ab"));
        // No front matter, or an unclosed one: all body.
        assert_eq!(split_front_matter("Hello"), ("", "Hello"));
        assert_eq!(
            split_front_matter("---\nnot closed"),
            ("", "---\nnot closed")
        );
        assert_eq!(title_of("\n## Groceries \nmilk"), "Groceries");
    }

    #[test]
    fn create_list_save_and_versions() {
        let store = temp_store("basic");
        let note = store
            .create(
                "Groceries\nmilk",
                at("2026-10-31T09:15:00-05:00"),
                &Origin {
                    app: Some("Mail"),
                    ..Origin::default()
                },
            )
            .unwrap();
        assert!(note.path.ends_with(" Groceries.md"), "{}", note.path);
        let raw = std::fs::read_to_string(store.root().join(&note.path)).unwrap();
        assert!(raw.starts_with("---\nsplashdown-id: "));
        assert!(raw.contains("\napp: Mail\n") && raw.ends_with("---\nGroceries\nmilk"));

        // Renaming the title doesn't rename the file.
        store.open(&note.path).unwrap();
        let saved = store
            .save(&note.path, "Shopping\nmilk, eggs", Some(note.modified))
            .unwrap();
        assert!(!saved.conflict);
        assert_eq!(saved.note.path, note.path);
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Shopping");

        // The first change after opening kept the original.
        let versions = store.versions(&note.path).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].preview, "Groceries\nmilk");
        // A second save in the same session doesn't add one.
        store
            .save(&note.path, "Shopping\nmilk, eggs, tea", None)
            .unwrap();
        assert_eq!(store.versions(&note.path).unwrap().len(), 1);

        // Restoring keeps what was there as a version too.
        let restored = store.restore(&note.path, versions[0].at).unwrap();
        assert_eq!(restored.body, "Groceries\nmilk");
        assert_eq!(store.versions(&note.path).unwrap().len(), 2);
        let raw = std::fs::read_to_string(store.root().join(&note.path)).unwrap();
        assert!(raw.contains("app: Mail"), "front matter kept");
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn another_apps_change_is_kept_as_a_version() {
        let store = temp_store("conflict");
        let note = store
            .create("Plan", at("2026-10-31T09:00:00Z"), &Origin::default())
            .unwrap();
        let file = store.root().join(&note.path);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, text.replace("Plan", "Plan from my phone")).unwrap();

        let saved = store
            .save(&note.path, "Plan, edited here", Some(note.modified))
            .unwrap();
        assert!(saved.conflict);
        assert_eq!(saved.note.body, "Plan, edited here");
        assert_eq!(
            store.versions(&note.path).unwrap()[0].preview,
            "Plan from my phone"
        );
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn listing_skips_hidden_and_attachments_and_sorts_pins_first() {
        let store = temp_store("list");
        store.ensure().unwrap();
        let root = store.root();
        std::fs::create_dir_all(root.join("Work")).unwrap();
        std::fs::create_dir_all(root.join("Attachments")).unwrap();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::write(root.join("Work/Standup.md"), "# Standup\nshipped").unwrap();
        std::fs::write(root.join("Attachments/x.md"), "no").unwrap();
        std::fs::write(root.join(".obsidian/y.md"), "no").unwrap();
        std::fs::write(root.join("notes.txt"), "no").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("Ideas.md"), "Ideas").unwrap();

        let list = store.list();
        let paths: Vec<_> = list.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(paths, ["Ideas.md", "Work/Standup.md"]);
        assert_eq!(list[1].folder, "Work");
        assert_eq!(list[1].title, "Standup");

        store.set_pinned("Work/Standup.md", true);
        assert_eq!(store.list()[0].path, "Work/Standup.md");
        assert!(store.list()[0].pinned);

        // A note made elsewhere keeps having no front matter when saved.
        store.save("Ideas.md", "Ideas\nmore", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("Ideas.md")).unwrap(),
            "Ideas\nmore"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn paths_stay_inside_the_folder() {
        let store = temp_store("confine");
        store.ensure().unwrap();
        for bad in [
            "../x.md",
            "/etc/hosts",
            ".splashdown/state.json",
            "",
            "a/../../b.md",
            // Only notes, so nothing else in the folder can be overwritten.
            "Attachments/a.md",
            "Taxes/2025.csv",
            "photo.png",
        ] {
            assert!(store.read(bad).is_err(), "{bad}");
        }
        std::fs::write(store.root().join("photo.png"), [0xff, 0xd8]).unwrap();
        let png = Target::Note("photo.png".into());
        assert!(
            store
                .append(&png, "x", at("2026-10-31T09:15:00-05:00"))
                .is_err()
        );
        assert_eq!(
            std::fs::read(store.root().join("photo.png")).unwrap(),
            [0xff, 0xd8]
        );

        // A note that isn't UTF-8 is left alone rather than replaced.
        std::fs::write(store.root().join("Old.md"), [b'c', b'a', b'f', 0xe9]).unwrap();
        let old = Target::Note("Old.md".into());
        assert!(
            store
                .append(&old, "x", at("2026-10-31T09:15:00-05:00"))
                .is_err()
        );
        assert!(store.save("Old.md", "new", None).is_err());
        assert_eq!(std::fs::read(store.root().join("Old.md")).unwrap().len(), 4);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn rename_and_unique_names() {
        let store = temp_store("rename");
        let now = at("2026-10-31T09:00:00Z");
        let a = store.create("Plan", now, &Origin::default()).unwrap();
        let b = store.create("Plan", now, &Origin::default()).unwrap();
        assert!(b.path.ends_with("Plan 2.md"), "{}", b.path);
        let untitled = store.create("   ", now, &Origin::default()).unwrap();
        assert!(untitled.path.len() > 10, "named after the time");

        store.set_pinned(&a.path, true);
        let renamed = store.rename(&a.path, "Q4 plan").unwrap();
        assert_eq!(renamed, "Q4 plan.md");
        assert!(store.list().iter().any(|n| n.path == renamed && n.pinned));
        // A change of case only keeps the name, not "Q4 Plan 2".
        assert_eq!(store.rename(&renamed, "Q4 Plan").unwrap(), "Q4 Plan.md");
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn captures_append_with_the_time_and_can_be_taken_back() {
        let store = temp_store("append");
        let now = at("2026-10-31T09:15:00-05:00").with_timezone(&Local);
        let first = store.append(&Target::Inbox, "Buy milk ", now).unwrap();
        assert_eq!(first.path, "Inbox.md");
        let time = now.format("%H:%M").to_string();
        let inbox = || std::fs::read_to_string(store.root().join("Inbox.md")).unwrap();
        assert_eq!(inbox(), format!("# Inbox\n\n**{time}** Buy milk\n"));
        let second = store.append(&Target::Inbox, "Call Sam", now).unwrap();
        assert_eq!(
            inbox(),
            format!("# Inbox\n\n**{time}** Buy milk\n\n**{time}** Call Sam\n")
        );
        assert!(store.undo_append(&second).unwrap());
        assert_eq!(inbox(), format!("# Inbox\n\n**{time}** Buy milk\n"));
        // Undoing again finds nothing to take back.
        assert!(!store.undo_append(&second).unwrap());

        let daily = Target::Daily {
            folder: "Journal/Daily".into(),
            format: "YYYY-MM-DD ddd".into(),
        };
        let d = store.append(&daily, "Ran 5k", now).unwrap();
        assert_eq!(
            d.path,
            format!("Journal/Daily/{} ddd.md", now.format("%Y-%m-%d"))
        );
        // Folders by date, as Obsidian can make them.
        let nested = Target::Daily {
            folder: "Journal".into(),
            format: "YYYY/MM/YYYY-MM-DD".into(),
        };
        assert_eq!(
            store.target_path(&nested, now),
            format!("Journal/{}.md", now.format("%Y/%m/%Y-%m-%d"))
        );
        // With nothing opened yet, "last" is the inbox.
        assert_eq!(store.target_path(&Target::Last, now), "Inbox.md");
        assert!(store.list().iter().any(|n| n.title == "Inbox"));
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn imports_the_scratchpad() {
        let store = temp_store("import");
        let json = r#"{"current":{"text":"Current idea","at":1761900000000},
            "earlier":[{"text":"Older","at":1761800000000},{"text":"  ","at":1}]}"#;
        assert_eq!(store.import_scratchpad(json).unwrap(), 2);
        let list = store.list();
        assert_eq!(list.len(), 2);
        let raw = std::fs::read_to_string(store.root().join(&list[0].path)).unwrap();
        assert!(raw.contains("source: scratchpad"));
        assert_eq!(
            store.last().map(|(p, _)| p),
            Some(
                list.iter()
                    .find(|n| n.title == "Current idea")
                    .unwrap()
                    .path
                    .clone()
            )
        );
        let _ = std::fs::remove_dir_all(store.root());
    }
}
