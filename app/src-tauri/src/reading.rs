//! Reading text aloud: "read this to me", "read me a summary of this" (an
//! Ask answer, read as it's written), the Ask panel's speaker button, and
//! Last dictation → Read it to me. English is read in the natural voice
//! when it's downloaded and chosen (Phonon, run here); other languages, or
//! without it, in the system's voices. Nothing read this way leaves the
//! computer.
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

/// A chosen voice that's one of the natural voices: `natural:Harper`.
pub const NATURAL: &str = "natural:";
/// The system's voices chosen, the best one for each language: `system:`.
pub const SYSTEM: &str = "system:";
/// The natural voice that reads English when no voice is chosen.
const NATURAL_DEFAULT: &str = "Harper";

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
    /// Set when it's stopped, for the natural voice's playback.
    stop: std::sync::Arc<AtomicBool>,
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
        stop: std::sync::Arc::default(),
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

/// The next piece of reading `n` to read, waiting for one; `None` once it
/// has been stopped, or nothing more is coming.
fn next_piece(n: u64) -> Option<String> {
    let mut slot = reading();
    loop {
        let r = slot.as_mut().filter(|r| r.n == n)?;
        if let Some(piece) = r.queue.pop_front() {
            return Some(piece);
        }
        if r.closed {
            return None;
        }
        slot = WAKE.wait(slot).unwrap_or_else(|e| e.into_inner());
    }
}

/// Reads reading `n`'s pieces one after another until they run out and no
/// more are coming, or it's stopped: in the natural voice when it's chosen
/// and the text is English, otherwise in a system voice.
fn read_all(n: u64) {
    let Some(first) = next_piece(n) else {
        return finish(n);
    };
    let Some((voice, language, speed, stop)) = reading()
        .as_ref()
        .filter(|r| r.n == n)
        .map(|r| (r.voice.clone(), r.language, r.speed, r.stop.clone()))
    else {
        return;
    };
    let automatic = voice.is_none();
    let name = natural_name(voice.as_deref(), language).map(str::to_string);
    let mut speech = name.as_ref().and_then(|_| natural_speech());
    if speech.is_none() && name.is_some() && automatic && offer_download(n) {
        speech = natural_speech();
    }
    if !current(n) {
        return;
    }
    let natural = speech.zip(name);
    let result = match natural {
        Some((speech, name)) => {
            let pieces = std::iter::once(first).chain(std::iter::from_fn(|| next_piece(n)));
            let samples = pieces.flat_map(|piece| match speech.speak(&piece, Some(&name)) {
                Ok(samples) => Box::new(samples) as Box<dyn Iterator<Item = _>>,
                Err(e) => Box::new(std::iter::once(Err(e))),
            });
            splashdown_audio::playback::play(samples, speech.sample_rate(), &stop)
        }
        None => {
            // A natural voice chosen, but English isn't what's read (or it
            // isn't there): the system's voice for the language.
            let voice = voice.filter(|v| !v.starts_with(NATURAL) && v != SYSTEM);
            let mut piece = Some(first);
            let mut result = Ok(());
            while let Some(text) = piece.take() {
                result = speak(&text, voice.as_deref(), language, speed);
                if result.is_err() {
                    break;
                }
                // Stopped while the speech was being made (Windows makes it
                // first).
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
                piece = next_piece(n);
            }
            result
        }
    };
    let failed = result.is_err() && current(n);
    finish(n);
    if let Err(e) = result
        && failed
    {
        tracing::warn!("couldn't read aloud: {e}");
        if let Some(ui) = UI.get() {
            ui.show(OverlayState::Message {
                tone: splashdown_platform::Tone::Warning,
                text: "Couldn't read that aloud".into(),
            });
            let shown = ui.generation();
            std::thread::sleep(WARNING_FOR);
            ui.hide_if_unchanged(shown);
        }
    }
}

/// The natural voice to read in, when it's English (or the language isn't
/// known) and a natural voice is chosen, or none is (Automatic: Harper).
/// `None`: a system voice reads it.
fn natural_name<'a>(voice: Option<&'a str>, language: Option<&str>) -> Option<&'a str> {
    if language.is_some_and(|l| l != "en") {
        return None;
    }
    match voice {
        None => Some(NATURAL_DEFAULT),
        Some(v) => v.strip_prefix(NATURAL),
    }
}

/// Asks once, the first time English is read without the natural voices,
/// whether to download them; if so, downloads them (the pill shows how
/// it's going). Whether they're here now.
fn offer_download(n: u64) -> bool {
    let Some(ui) = UI.get() else {
        return false;
    };
    let app = ui.app().clone();
    let Some(state) = tauri::Manager::try_state::<crate::state::AppState>(&app) else {
        return false;
    };
    if state.settings().general.natural_voice_offered {
        return false;
    }
    let answer = crate::prompt::ask(
        &app,
        "Download Phonon 31M?",
        "Splashdown can read English in Phonon 31M, a text-to-speech model that sounds \
         like a person and runs on this computer: a one-time download of 68 MB.",
        &["Download", "Use the system voice"],
    );
    if answer.is_some()
        && let Err(e) = state.update(&app, |s| s.general.natural_voice_offered = true)
    {
        tracing::warn!("couldn't save that the natural voices were offered: {e}");
    }
    if answer != Some(0) || !current(n) {
        return false;
    }
    let shown = |text: String| {
        ui.show(OverlayState::Reading {
            label: format!("{text} · Esc to stop"),
        });
        if let Some(r) = reading().as_mut().filter(|r| r.n == n) {
            r.shown = ui.generation();
        }
    };
    shown("Downloading Phonon 31M…".into());
    let fetched = tauri::async_runtime::block_on(crate::models::fetch_model(
        &app,
        splashdown_providers::NaturalVoice::ID,
        |done, total| {
            if let Some(total) = total.filter(|t| *t > 0)
                && current(n)
            {
                shown(format!("Downloading Phonon 31M… {}%", done * 100 / total));
            }
        },
    ));
    if let Err(e) = &fetched {
        tracing::warn!("couldn't download the natural voices: {e}");
    }
    if current(n) {
        shown("Reading aloud".into());
    }
    fetched.is_ok()
}

/// The natural voice, loaded the first time it's needed (and kept), if
/// it's been downloaded.
fn natural_speech() -> Option<std::sync::Arc<splashdown_providers::NaturalSpeech>> {
    static LOADED: Mutex<Option<std::sync::Arc<splashdown_providers::NaturalSpeech>>> =
        Mutex::new(None);
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(speech) = loaded.as_ref() {
        return Some(speech.clone());
    }
    let models = crate::engine::paths().ok()?.models_dir;
    if !splashdown_providers::NaturalVoice::is_downloaded(&models) {
        return None;
    }
    let started = std::time::Instant::now();
    match splashdown_providers::NaturalSpeech::load(&models) {
        Ok(speech) => {
            tracing::info!("natural voice loaded in {:?}", started.elapsed());
            let speech = std::sync::Arc::new(speech);
            *loaded = Some(speech.clone());
            Some(speech)
        }
        Err(e) => {
            tracing::warn!("couldn't load the natural voice: {e}");
            None
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
    ended.stop.store(true, Ordering::SeqCst);
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
    /// "Natural" (the downloaded natural voices), "Premium", "Enhanced"
    /// or "" (macOS only).
    pub quality: &'static str,
    /// What it sounds like (natural voices).
    pub about: Option<String>,
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
                    about: None,
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
                    about: None,
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
    // The natural voices first, once they're downloaded.
    let natural = crate::engine::paths()
        .map(|p| splashdown_providers::NaturalVoice::voices(&p.models_dir))
        .unwrap_or_default()
        .into_iter()
        .map(|v| Voice {
            id: format!("{NATURAL}{}", v.name),
            name: v.name,
            language: match v.locale.as_str() {
                "en-gb" => "en-GB".into(),
                _ => "en-US".into(),
            },
            quality: "Natural",
            about: v.description,
        });
    natural.chain(voices.into_iter().map(|(v, _)| v)).collect()
}

/// Whether the natural voice is downloaded, and its size.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NaturalStatus {
    pub downloaded: bool,
    pub mb: u32,
}

// ---------------------------------------------------------------- commands

/// The voices for Settings → AI Models → Text to speech.
#[tauri::command]
pub async fn reading_voices() -> Vec<Voice> {
    tauri::async_runtime::spawn_blocking(voices)
        .await
        .unwrap_or_default()
}

/// Settings: whether the natural voice is here to choose.
#[tauri::command]
pub fn reading_natural() -> NaturalStatus {
    NaturalStatus {
        downloaded: crate::engine::paths()
            .is_ok_and(|p| splashdown_providers::NaturalVoice::is_downloaded(&p.models_dir)),
        mb: splashdown_providers::NaturalVoice::APPROX_MB,
    }
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
        r.detect = voice.as_deref().is_none_or(|v| v == SYSTEM);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_gets_a_natural_voice_unless_a_system_one_is_chosen() {
        assert_eq!(natural_name(None, Some("en")), Some("Harper"));
        assert_eq!(natural_name(None, None), Some("Harper"));
        assert_eq!(
            natural_name(Some("natural:Maeve"), Some("en")),
            Some("Maeve")
        );
        assert_eq!(natural_name(Some("natural:Maeve"), Some("de")), None);
        assert_eq!(natural_name(None, Some("es")), None);
        let samantha = "com.apple.voice.compact.en-US.Samantha";
        assert_eq!(natural_name(Some(samantha), Some("en")), None);
        assert_eq!(natural_name(Some(SYSTEM), Some("en")), None);
    }
}
