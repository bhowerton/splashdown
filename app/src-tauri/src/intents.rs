//! Splashdown's actions in Shortcuts, Spotlight and Siri (macOS App
//! Intents; the actions themselves are Swift, in
//! `crates/splashdown-apple-shim/swift/Sources/SplashdownAppleShim/
//! Intents.swift`). Each sends an [`Intent`] as JSON and gets a [`Reply`]
//! back. Most go through the session like the control socket's requests,
//! so a dictation started here finishes with the key or Esc as usual.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use splashdown_control::Request;
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// What an action asks for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Intent {
    /// Add text to the capture note; answers with the note.
    AddToNote {
        text: String,
    },
    /// A hands-free dictation whose text comes back instead of being
    /// typed, with `prompt` in the overlay.
    Dictate {
        #[serde(default)]
        prompt: Option<String>,
    },
    /// Start dictating into the focused field, or finish.
    ToggleDictation,
    /// Start a hands-free recording that goes into a note, or finish it.
    RecordNote,
    StartMeeting,
    /// Stop the meeting; with `wait`, answer once its note is finished.
    StopMeeting {
        #[serde(default)]
        wait: bool,
    },
    /// Notes matching `query` (none: the newest), narrowed like
    /// `splashdown-mcp`'s `list_notes`.
    FindNotes {
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        when: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        folder: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// The notes at these paths (Shortcuts keeps a note by its path).
    NotesAt {
        paths: Vec<String>,
    },
    /// A note's text, or one `section` of it: summary, my_notes or
    /// transcript.
    ReadNote {
        path: String,
        #[serde(default)]
        section: Option<String>,
    },
    /// A question about the notes, answered by the AI.
    Ask {
        question: String,
    },
    /// An audio or video file, transcribed into a new note.
    TranscribeFile {
        path: PathBuf,
    },
    /// Show a note in the Notes window.
    OpenNote {
        path: String,
    },
}

/// A note, as Shortcuts shows and keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NoteRef {
    /// Inside the notes folder; what Shortcuts keeps.
    path: String,
    title: String,
    /// meeting, transcript, capture or note.
    kind: String,
    /// When it was made (RFC 3339), if its front matter says.
    #[serde(skip_serializing_if = "Option::is_none")]
    created: Option<String>,
    /// Its folder inside the notes folder ("" at the top).
    folder: String,
}

impl NoteRef {
    /// From one note in a list or search reply (`notes/agents.rs`).
    fn from_listed(v: &serde_json::Value) -> Option<Self> {
        let path = v["path"].as_str()?.to_string();
        let folder = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some(NoteRef {
            title: v["title"].as_str().unwrap_or(&path).to_string(),
            kind: v["kind"].as_str().unwrap_or("note").to_string(),
            created: v["created"].as_str().map(str::to_string),
            folder,
            path,
        })
    }
}

/// What the action shows and hands on. Empty fields are left out.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
struct Reply {
    /// For the person: "Added to Inbox".
    #[serde(skip_serializing_if = "String::is_empty")]
    message: String,
    /// Text for the next action (a dictation).
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    /// A note's file, absolute.
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<PathBuf>,
    /// One note, to pass on.
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<NoteRef>,
    /// Notes found, or the ones an answer drew on.
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<Vec<NoteRef>>,
}

impl Reply {
    fn say(message: impl Into<String>) -> Self {
        Reply {
            message: message.into(),
            ..Reply::default()
        }
    }
}

/// How long Stop Meeting waits for the note (the summary can take a while
/// with a slow model).
const NOTE_WAIT: Duration = Duration::from_secs(10 * 60);

/// Asked for while Settings doesn't allow it.
const READING_OFF: &str = "Shortcuts can't read your Splashdown notes. To allow it, turn on “Let shortcuts find and read notes” in Splashdown → Settings → Privacy.";

/// A note marked hidden from agents (its ⋯ menu), asked for by path.
const HIDDEN: &str = "That note is hidden from agents and Shortcuts (its ⋯ menu in Notes).";

/// Most of a note one Get Note Text returns, in characters.
const READ_MOST: usize = 1_000_000;

/// Lets the actions run. Call once the app's state is managed.
pub fn serve(app: AppHandle) {
    splashdown_apple_shim::intents::start(move |json| {
        let intent: Intent = serde_json::from_str(&json)
            .map_err(|e| format!("Splashdown didn't understand that action ({e})."))?;
        let reply = answer(&app, intent)?;
        serde_json::to_string(&reply).map_err(|e| e.to_string())
    });
}

fn answer(app: &AppHandle, intent: Intent) -> Result<Reply, String> {
    let state = app.state::<AppState>();
    let control = |request: Request| state.session.control(request);
    match intent {
        Intent::AddToNote { text } => {
            if text.trim().is_empty() {
                return Err("There's no text to add.".into());
            }
            let notes = state.settings().notes;
            let captured = crate::notes::capture(&notes, None, &text, None)?;
            Ok(Reply {
                message: format!("Added to {}", captured.label),
                file: captured.file(),
                ..Reply::default()
            })
        }
        Intent::Dictate { prompt } => {
            let text = control(Request::Listen {
                prompt,
                to: Some("Shortcuts".into()),
            })?
            .unwrap_or_default();
            Ok(Reply {
                text: Some(text),
                ..Reply::default()
            })
        }
        Intent::ToggleDictation => control(Request::Toggle).map(|_| Reply::default()),
        Intent::RecordNote => control(Request::Note { text: None }).map(|_| Reply::default()),
        Intent::StartMeeting => {
            crate::meeting::start(app).map(|_| Reply::say("Recording the meeting"))
        }
        Intent::StopMeeting { wait } => {
            let note = crate::meeting::status().map(|s| s.note);
            crate::meeting::stop()?;
            let file = note.and_then(|n| note_file(&state, &n));
            if !wait {
                return Ok(Reply {
                    message: "Stopping; the note is being finished.".into(),
                    file,
                    ..Reply::default()
                });
            }
            let started = Instant::now();
            while crate::meeting::is_running() {
                if started.elapsed() > NOTE_WAIT {
                    return Err("The meeting note is taking too long to finish.".into());
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Ok(Reply {
                message: "The meeting note is ready.".into(),
                file,
                ..Reply::default()
            })
        }
        Intent::FindNotes {
            query,
            when,
            kind,
            folder,
            limit,
        } => {
            let notes = readable(&state)?;
            let filter = splashdown_control::NoteFilter {
                when,
                kind,
                folder,
                ..Default::default()
            };
            let limit = Some(limit.unwrap_or(20).clamp(1, 50));
            let found = match query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
                Some(query) => {
                    let reply = crate::notes::agent_search(&notes, query, limit, &filter)?;
                    serde_json::from_str::<serde_json::Value>(&reply).map_err(|e| e.to_string())?
                }
                None => {
                    let reply = crate::notes::agent_list(&notes, &filter, false, limit, None)?;
                    serde_json::from_str::<serde_json::Value>(&reply)
                        .map_err(|e| e.to_string())?["notes"]
                        .take()
                }
            };
            let found: Vec<NoteRef> = found
                .as_array()
                .map(|a| a.iter().filter_map(NoteRef::from_listed).collect())
                .unwrap_or_default();
            Ok(Reply {
                notes: Some(found),
                ..Reply::default()
            })
        }
        Intent::NotesAt { paths } => {
            let notes = readable(&state)?;
            let found = crate::notes::notes_at(&notes, &paths)?
                .iter()
                .filter_map(NoteRef::from_listed)
                .collect();
            Ok(Reply {
                notes: Some(found),
                ..Reply::default()
            })
        }
        Intent::ReadNote { path, section } => {
            let notes = readable(&state)?;
            let text = read_all(&notes, &path, section.as_deref())?;
            Ok(Reply {
                text: Some(text),
                ..Reply::default()
            })
        }
        Intent::Ask { question } => {
            readable(&state)?;
            let (answer, sources) = crate::notes::ask_for_shortcuts(&state, &question)?;
            let notes = readable(&state)?;
            let paths: Vec<String> = sources.into_iter().map(|(_, path)| path).collect();
            let drawn_on = crate::notes::notes_at(&notes, &paths)?
                .iter()
                .filter_map(NoteRef::from_listed)
                .collect();
            Ok(Reply {
                text: Some(answer),
                notes: Some(drawn_on),
                ..Reply::default()
            })
        }
        Intent::TranscribeFile { path } => {
            let done = crate::notes::files::transcribe_and_wait(app, path)?;
            let notes = state.settings().notes;
            let file = note_file(&state, &done.note);
            let note = NoteRef {
                title: crate::notes::title_at(&notes, &done.note).unwrap_or(done.note.clone()),
                kind: "transcript".into(),
                created: None,
                folder: std::path::Path::new(&done.note)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path: done.note,
            };
            Ok(Reply {
                message: done
                    .stopped
                    .unwrap_or_else(|| format!("Transcribed into “{}”", note.title)),
                file,
                note: Some(note),
                ..Reply::default()
            })
        }
        Intent::OpenNote { path } => {
            let notes = readable(&state)?;
            if crate::notes::notes_at(&notes, std::slice::from_ref(&path))?.is_empty() {
                return Err(format!(
                    "There's no note at {path} that Shortcuts can open."
                ));
            }
            let handle = app.clone();
            app.run_on_main_thread(move || {
                crate::notes::open_to(&handle, Some(&format!("note:{path}")));
            })
            .map_err(|e| e.to_string())?;
            Ok(Reply::default())
        }
    }
}

/// The notes settings as the coding-agent note functions see them, for a
/// Shortcuts action: allowed when Settings lets shortcuts read notes,
/// meetings included (they're the user's own), and still never a note
/// marked hidden.
fn readable(state: &AppState) -> Result<splashdown_config::NotesSettings, String> {
    let notes = state.settings().notes;
    if !notes.shortcuts_read() {
        return Err(READING_OFF.into());
    }
    Ok(splashdown_config::NotesSettings {
        agents_read: true,
        agents_meetings: Some(true),
        ..notes
    })
}

/// All of a note's text, or one section of it, page by page.
fn read_all(
    notes: &splashdown_config::NotesSettings,
    path: &str,
    section: Option<&str>,
) -> Result<String, String> {
    let section = section
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("all");
    let mut text = String::new();
    let mut cursor = None;
    loop {
        let part = splashdown_control::ReadPart {
            section: Some(section.to_string()),
            max_chars: Some(40_000),
            cursor: cursor.take(),
            ..Default::default()
        };
        let reply = crate::notes::agent_read(notes, path, &part).map_err(|e| {
            if e == crate::notes::HIDDEN {
                HIDDEN.to_string()
            } else {
                e
            }
        })?;
        let page: serde_json::Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
        if page.get("missing").is_some_and(|m| !m.is_null()) {
            let title = page["title"].as_str().unwrap_or(path);
            let what = match section {
                "my_notes" => "My notes",
                "transcript" => "transcript",
                _ => "Summary",
            };
            return Err(format!("“{title}” has no {what} section."));
        }
        text.push_str(page["text"].as_str().unwrap_or_default());
        match page["next_cursor"].as_str() {
            Some(next) if text.len() < READ_MOST => cursor = Some(next.to_string()),
            _ => break,
        }
    }
    Ok(text)
}

/// A note's path inside the notes folder, made absolute.
fn note_file(state: &AppState, path: &str) -> Option<PathBuf> {
    let (store, _) = crate::notes::store_for(state.settings().notes.folder).ok()?;
    Some(store.root().join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intents_read_as_swift_sends_them() {
        let read = |json: &str| serde_json::from_str::<Intent>(json).unwrap();
        assert_eq!(
            read(r#"{"cmd":"add_to_note","text":"Buy milk"}"#),
            Intent::AddToNote {
                text: "Buy milk".into()
            }
        );
        assert_eq!(
            read(r#"{"cmd":"dictate"}"#),
            Intent::Dictate { prompt: None }
        );
        assert_eq!(
            read(r#"{"cmd":"dictate","prompt":"Reply to Sam"}"#),
            Intent::Dictate {
                prompt: Some("Reply to Sam".into())
            }
        );
        assert_eq!(
            read(r#"{"cmd":"toggle_dictation"}"#),
            Intent::ToggleDictation
        );
        assert_eq!(read(r#"{"cmd":"record_note"}"#), Intent::RecordNote);
        assert_eq!(read(r#"{"cmd":"start_meeting"}"#), Intent::StartMeeting);
        assert_eq!(
            read(r#"{"cmd":"stop_meeting","wait":true}"#),
            Intent::StopMeeting { wait: true }
        );
        assert_eq!(
            read(r#"{"cmd":"find_notes","query":"budget","kind":"meeting"}"#),
            Intent::FindNotes {
                query: Some("budget".into()),
                when: None,
                kind: Some("meeting".into()),
                folder: None,
                limit: None,
            }
        );
        assert_eq!(
            read(r#"{"cmd":"notes_at","paths":["Inbox.md"]}"#),
            Intent::NotesAt {
                paths: vec!["Inbox.md".into()]
            }
        );
        assert_eq!(
            read(r#"{"cmd":"read_note","path":"Inbox.md","section":"summary"}"#),
            Intent::ReadNote {
                path: "Inbox.md".into(),
                section: Some("summary".into())
            }
        );
        assert_eq!(
            read(r#"{"cmd":"ask","question":"What's due?"}"#),
            Intent::Ask {
                question: "What's due?".into()
            }
        );
        assert_eq!(
            read(r#"{"cmd":"transcribe_file","path":"/tmp/a.m4a"}"#),
            Intent::TranscribeFile {
                path: "/tmp/a.m4a".into()
            }
        );
        assert_eq!(
            read(r#"{"cmd":"open_note","path":"Inbox.md"}"#),
            Intent::OpenNote {
                path: "Inbox.md".into()
            }
        );
        assert!(serde_json::from_str::<Intent>(r#"{"cmd":"fly"}"#).is_err());
    }

    #[test]
    fn notes_are_passed_on_by_path() {
        let listed = serde_json::json!({
            "path": "Meetings/Sync.md",
            "title": "Sync",
            "kind": "meeting",
            "created": "2026-11-05T11:24:06-05:00",
        });
        assert_eq!(
            NoteRef::from_listed(&listed),
            Some(NoteRef {
                path: "Meetings/Sync.md".into(),
                title: "Sync".into(),
                kind: "meeting".into(),
                created: Some("2026-11-05T11:24:06-05:00".into()),
                folder: "Meetings".into(),
            })
        );
        let top = serde_json::json!({ "path": "Inbox.md" });
        let top = NoteRef::from_listed(&top).unwrap();
        assert_eq!((top.title.as_str(), top.folder.as_str()), ("Inbox.md", ""));
        assert_eq!(NoteRef::from_listed(&serde_json::json!({})), None);
    }

    #[test]
    fn replies_leave_out_what_they_lack() {
        assert_eq!(
            serde_json::to_string(&Reply::say("Added to Inbox")).unwrap(),
            r#"{"message":"Added to Inbox"}"#
        );
        let dictated = Reply {
            text: Some("Hello".into()),
            ..Reply::default()
        };
        assert_eq!(
            serde_json::to_string(&dictated).unwrap(),
            r#"{"text":"Hello"}"#
        );
    }
}
