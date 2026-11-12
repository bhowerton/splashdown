//! What the user just did, for the setup guide's walk-through: each kind
//! of thing (a dictation, a rewrite of selected text, a voice action…)
//! with when it last worked. The guide asks for what happened since a
//! step began, so a step only ticks off when it was actually done.

use splashdown_core::sync::Locked;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use serde::Serialize;

/// Kinds of things done.
pub const DICTATED: &str = "dictated";
/// Text went into one of the user's apps, not Splashdown's own windows.
pub const ELSEWHERE: &str = "elsewhere";
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
/// A draft written by Compose (into the field or the panel).
pub const COMPOSED: &str = "composed";

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
    let at = now();
    LAST.locked().get_or_insert_with(HashMap::new).insert(
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
    LAST.locked()
        .as_ref()
        .map(|last| last.values().filter(|d| d.at >= since).cloned().collect())
        .unwrap_or_default()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

// While someone tries dictating in the guide, two live signs explain a
// failure by themselves: whether Splashdown noticed the key, and whether
// the microphone hears them. They come from what the overlay is told
// (`Ui::tray_icon` and `Ui::level`).

/// Recording now (the key is held, or a hands-free dictation runs).
static HELD: AtomicBool = AtomicBool::new(false);
/// When recording last started, and when speech was last heard (ms since
/// the epoch).
static HELD_AT: AtomicU64 = AtomicU64::new(0);
static HEARD_AT: AtomicU64 = AtomicU64::new(0);
/// The loudest level since the guide last asked (f32 bits).
static PEAK: AtomicU32 = AtomicU32::new(0);

/// Recording started (`true`) or ended.
pub fn recording(on: bool) {
    if on && !HELD.swap(true, Ordering::Relaxed) {
        HELD_AT.store(now(), Ordering::Relaxed);
    } else if !on {
        HELD.store(false, Ordering::Relaxed);
    }
}

/// The microphone's level while recording, as the overlay gets it.
pub fn level(level: f32) {
    if level > crate::session::SPEECH_LEVEL {
        HEARD_AT.store(now(), Ordering::Relaxed);
    }
    // Only the session's thread writes it, so a plain compare is enough.
    if level > f32::from_bits(PEAK.load(Ordering::Relaxed)) {
        PEAK.store(level.to_bits(), Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Signs {
    /// Recording right now.
    held: bool,
    /// When recording last started, and speech was last heard (ms since
    /// the epoch, 0 for never).
    held_at: u64,
    heard_at: u64,
    /// The loudest level since the last call (0–1, mostly well under 0.3).
    level: f32,
}

/// The guide's live signs; each call starts a new peak.
#[tauri::command]
pub fn try_signs() -> Signs {
    Signs {
        held: HELD.load(Ordering::Relaxed),
        held_at: HELD_AT.load(Ordering::Relaxed),
        heard_at: HEARD_AT.load(Ordering::Relaxed),
        level: f32::from_bits(PEAK.swap(0, Ordering::Relaxed)),
    }
}
