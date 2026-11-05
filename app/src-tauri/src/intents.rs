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
    }
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
        assert!(serde_json::from_str::<Intent>(r#"{"cmd":"fly"}"#).is_err());
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
