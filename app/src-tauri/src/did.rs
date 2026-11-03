//! What the user just did, for the setup guide's walk-through: each kind
//! of thing (a dictation, a rewrite of selected text, a voice action…)
//! with when it last worked. The guide asks for what happened since a
//! step began, so a step only ticks off when it was actually done.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

/// Kinds of things done.
pub const DICTATED: &str = "dictated";
/// Selected text rewritten.
pub const REWROTE: &str = "rewrote";
/// Dictated with an instruction after the instruction key.
pub const INSTRUCTED: &str = "instructed";
/// Return pressed after the text went in.
pub const SENT: &str = "sent";
/// A spoken edit ("delete that last sentence").
pub const EDITED: &str = "edited";
/// A voice action.
pub const ACTED: &str = "acted";
/// A question asked (the Ask panel).
pub const ASKED: &str = "asked";
/// Had text read aloud (or a summary of it).
pub const READ: &str = "read";

#[derive(Debug, Clone, Serialize)]
pub struct Did {
    pub what: &'static str,
    /// Milliseconds since the Unix epoch, like history entries.
    pub at: u64,
    /// What the pill said ("Opened Calculator").
    pub text: String,
}

static LAST: Mutex<Option<HashMap<&'static str, Did>>> = Mutex::new(None);

/// Notes that `what` just worked; `text` is what the pill said.
pub fn note(what: &'static str, text: &str) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    LAST.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(
            what,
            Did {
                what,
                at,
                text: text.to_string(),
            },
        );
}

/// What was done at or after `since` (milliseconds since the epoch).
#[tauri::command]
pub fn did_since(since: u64) -> Vec<Did> {
    LAST.lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|last| last.values().filter(|d| d.at >= since).cloned().collect())
        .unwrap_or_default()
}
