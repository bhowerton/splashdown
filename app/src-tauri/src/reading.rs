//! Reading text aloud with the system's voices: "read this to me", "read
//! me a summary of this" (an Ask answer, read as it's written), the Ask
//! panel's speaker button, and Last dictation → Read it to me. Nothing
//! read this way leaves the computer.
//!
//! While it reads, music is paused, the status pill says so, and Esc (or
//! starting a dictation) stops it. Text is read a few sentences at a time,
//! so the voice starts at once and an answer can be read while it's still
//! being written.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use splashdown_core::aloud;
use splashdown_platform::{OverlayState, Quieten};
use tauri::Emitter;

use crate::ui::Ui;

/// App event: whether something is being read (`bool`), for the Ask
/// panel's speaker button.
pub const CHANGED: &str = "reading-changed";

/// How often the voice is checked on.
const POLL: Duration = Duration::from_millis(80);
/// How long a warning stays in the pill.
const WARNING_FOR: Duration = Duration::from_secs(3);

/// A reading in progress.
struct Reading {
    n: u64,
    /// Pieces still to read.
    queue: VecDeque<String>,
    /// No more pieces are coming.
    closed: bool,
    /// The text's language, once known ("de").
    language: Option<&'static str>,
    /// Find out the language from the text (not when trying a voice).
    detect: bool,
    voice: Option<String>,
    speed: f32,
    /// The pill shown for it, to hide only if nothing replaced it.
    shown: u64,
    /// Music paused while it reads; dropping it plays it again.
    _quiet: Option<splashdown_platform::QuietOutput>,
}

static READING: Mutex<Option<Reading>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static COUNT: AtomicU64 = AtomicU64::new(0);
static UI: OnceLock<Ui> = OnceLock::new();
/// Esc stopped a reading, so its key-up is swallowed too.
static SWALLOW_ESC_UP: AtomicBool = AtomicBool::new(false);

fn reading() -> MutexGuard<'static, Option<Reading>> {
    READING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Called once at start-up with the status pill.
pub fn init(ui: Ui) {
    let _ = UI.set(ui);
}

/// Whether something is being read now.
pub fn is_reading() -> bool {
    reading().is_some()
}

/// A reading that may still be given more text (an answer being written).
pub struct Reader(u64);

impl Reader {
    /// Adds text to read after what's already queued.
    pub fn push(&self, text: &str) {
        let text = aloud::for_listening(text);
        let mut slot = reading();
        let Some(r) = slot.as_mut().filter(|r| r.n == self.0) else {
            return;
        };
        if r.detect && r.language.is_none() {
            r.language = aloud::language(&text);
        }
        r.queue.extend(aloud::pieces(&text));
        WAKE.notify_all();
    }

    /// Stops this reading (if it's still the current one).
    pub fn cancel(self) {
        finish(self.0);
    }

    /// Nothing more is coming: once what's queued is read, it's done.
    pub fn close(self) {
        if let Some(r) = reading().as_mut().filter(|r| r.n == self.0) {
            r.closed = true;
        }
        WAKE.notify_all();
    }
}

/// Starts a reading, stopping any before it. `label` is what the pill says
/// ("Reading aloud").
pub fn start(label: &str) -> Reader {
    stop();
    let Some(ui) = UI.get() else {
        return Reader(u64::MAX);
    };
    let general = ui.settings().map(|s| s.general).unwrap_or_default();
    let n = COUNT.fetch_add(1, Ordering::SeqCst) + 1;
    let esc = "Esc to stop";
    ui.show(OverlayState::Reading {
        label: format!("{label} · {esc}"),
    });
    *reading() = Some(Reading {
        n,
        queue: VecDeque::new(),
        closed: false,
        language: None,
        detect: true,
        voice: general.read_voice.clone().filter(|v| !v.is_empty()),
        speed: general.read_speed(),
        shown: ui.generation(),
        _quiet: splashdown_platform::quiet_output(Quieten::Pause),
    });
    changed(true);
    let started = std::thread::Builder::new()
        .name("splashdown-reading".into())
        .spawn(move || read_all(n));
    if let Err(e) = started {
        tracing::warn!("couldn't start reading aloud: {e}");
        finish(n);
    }
    Reader(n)
}

/// Reads `text` aloud. `Err` with a message for the pill when there's
/// nothing to read.
pub fn read(text: &str, label: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Nothing to read".into());
    }
    let reader = start(label);
    reader.push(text);
    reader.close();
    Ok(())
}

/// Stops reading at once.
pub fn stop() {
    let n = reading().as_ref().map(|r| r.n);
    if let Some(n) = n {
        finish(n);
    }
}

/// Esc, before the dictation keys see it: stops a reading. Whether to
/// swallow the key.
pub fn on_escape(down: bool) -> bool {
    if !down {
        return SWALLOW_ESC_UP.swap(false, Ordering::SeqCst);
    }
    if !is_reading() {
        return false;
    }
    stop();
    SWALLOW_ESC_UP.store(true, Ordering::SeqCst);
    true
}

/// Reads reading `n`'s pieces one after another until they run out and no
/// more are coming, or it's stopped.
fn read_all(n: u64) {
    loop {
        let piece = {
            let mut slot = reading();
            loop {
                let Some(r) = slot.as_mut().filter(|r| r.n == n) else {
                    return;
                };
                if let Some(piece) = r.queue.pop_front() {
                    break (piece, r.voice.clone(), r.language, r.speed);
                }
                if r.closed {
                    drop(slot);
                    finish(n);
                    return;
                }
                slot = WAKE.wait(slot).unwrap_or_else(|e| e.into_inner());
            }
        };
        let (text, voice, language, speed) = piece;
        if let Err(e) = speak(&text, voice.as_deref(), language, speed) {
            tracing::warn!("couldn't read aloud: {e}");
            finish(n);
            if let Some(ui) = UI.get() {
                ui.show(OverlayState::Message {
                    tone: splashdown_platform::Tone::Warning,
                    text: "Couldn't read that aloud".into(),
                });
                let shown = ui.generation();
                std::thread::sleep(WARNING_FOR);
                ui.hide_if_unchanged(shown);
            }
            return;
        }
        // Stopped while the speech was being made (Windows makes it first).
        if !current(n) {
            stop_speaking();
            return;
        }
        while speaking() {
            if !current(n) {
                return;
            }
            std::thread::sleep(POLL);
        }
    }
}

fn current(n: u64) -> bool {
    reading().as_ref().is_some_and(|r| r.n == n)
}

/// Ends reading `n`: the voice stops, music plays again and the pill goes.
fn finish(n: u64) {
    let ended = reading().take_if(|r| r.n == n);
    let Some(ended) = ended else {
        return;
    };
    WAKE.notify_all();
    stop_speaking();
    if let Some(ui) = UI.get() {
        ui.hide_if_unchanged(ended.shown);
    }
    drop(ended);
    changed(false);
}

fn changed(on: bool) {
    if let Some(ui) = UI.get() {
        let _ = ui.app().emit(CHANGED, on);
    }
}

#[cfg(target_os = "macos")]
fn speak(
    text: &str,
    voice: Option<&str>,
    language: Option<&str>,
    speed: f32,
) -> Result<(), String> {
    splashdown_apple_shim::speak(text, voice, language, speed);
    Ok(())
}

#[cfg(windows)]
fn speak(
    text: &str,
    voice: Option<&str>,
    language: Option<&str>,
    speed: f32,
) -> Result<(), String> {
    splashdown_platform::speech::speak(text, voice, language, speed).map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn speak(_: &str, _: Option<&str>, _: Option<&str>, _: f32) -> Result<(), String> {
    Err("reading aloud isn't available here".into())
}

fn speaking() -> bool {
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::speaking();
    #[cfg(windows)]
    return splashdown_platform::speech::speaking();
    #[allow(unreachable_code)]
    false
}

fn stop_speaking() {
    #[cfg(target_os = "macos")]
    splashdown_apple_shim::stop_speaking();
    #[cfg(windows)]
    splashdown_platform::speech::stop_speaking();
}

/// A voice offered in Settings.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub language: String,
    /// "Premium", "Enhanced" or "" (macOS only).
    pub quality: &'static str,
}

/// The voices that can read text, best first within each language.
pub fn voices() -> Vec<Voice> {
    #[cfg(target_os = "macos")]
    let mut voices: Vec<(Voice, u8)> = splashdown_apple_shim::voices()
        .into_iter()
        .map(|v| {
            let quality = match v.quality {
                3 => "Premium",
                2 => "Enhanced",
                _ => "",
            };
            (
                Voice {
                    id: v.id,
                    name: v.name,
                    language: v.language,
                    quality,
                },
                v.quality,
            )
        })
        .collect();
    #[cfg(windows)]
    let mut voices: Vec<(Voice, u8)> = splashdown_platform::speech::voices()
        .into_iter()
        .map(|v| {
            (
                Voice {
                    id: v.id,
                    name: v.name,
                    language: v.language,
                    quality: "",
                },
                1,
            )
        })
        .collect();
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut voices: Vec<(Voice, u8)> = Vec::new();
    voices.sort_by(|(a, qa), (b, qb)| {
        a.language
            .cmp(&b.language)
            .then(qb.cmp(qa))
            .then(a.name.cmp(&b.name))
    });
    voices.into_iter().map(|(v, _)| v).collect()
}

// ---------------------------------------------------------------- commands

/// The voices for Settings → General → Reading aloud.
#[tauri::command]
pub async fn reading_voices() -> Vec<Voice> {
    tauri::async_runtime::spawn_blocking(voices)
        .await
        .unwrap_or_default()
}

/// Settings' "Try it": reads a sentence with `voice` at `speed`, without
/// saving them.
#[tauri::command]
pub fn reading_try(voice: Option<String>, speed: f32) {
    let voice = voice.filter(|v| !v.is_empty());
    let language = voice
        .as_ref()
        .and_then(|id| voices().into_iter().find(|v| &v.id == id))
        .map(|v| v.language);
    // In the voice's own language, where there's a sentence for it.
    let sample = match language.as_deref().and_then(|l| l.split(['-', '_']).next()) {
        Some("de") => "So liest Splashdown dir deinen Text vor.",
        Some("fr") => "Voici comment Splashdown vous lit votre texte.",
        Some("es") => "Así es como Splashdown te lee el texto.",
        Some("it") => "Ecco come Splashdown ti legge il testo.",
        Some("pt") => "É assim que o Splashdown lê o seu texto.",
        Some("nl") => "Zo leest Splashdown je tekst aan je voor.",
        _ => "This is how Splashdown reads text back to you.",
    };
    let reader = start("Trying the voice");
    if let Some(r) = reading().as_mut().filter(|r| r.n == reader.0) {
        // The chosen voice whatever the sentence's language.
        r.detect = voice.is_none();
        r.voice = voice;
        r.speed = speed.clamp(0.5, 2.0);
    }
    reader.push(sample);
    reader.close();
}

/// The Ask panel's speaker button: reads `text`, or stops if it's reading.
#[tauri::command]
pub fn reading_toggle(text: String) -> Result<bool, String> {
    if is_reading() {
        stop();
        return Ok(false);
    }
    read(&text, "Reading the answer")?;
    Ok(true)
}

/// Whether something is being read (for a window that just opened).
#[tauri::command]
pub fn reading_state() -> bool {
    is_reading()
}

/// Stops reading (the Ask panel's Esc).
#[tauri::command]
pub fn reading_stop() {
    stop();
}
