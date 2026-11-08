//! Reading text aloud with Windows' own voices (the ones in Settings → Time
//! & language → Speech): the text is turned into speech in one go, then
//! played. `speaking` says whether it still is.

use splashdown_core::sync::Locked;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows::Foundation::TypedEventHandler;
use windows::Media::Core::MediaSource;
use windows::Media::Playback::MediaPlayer;
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
use windows::Storage::Streams::IRandomAccessStream;
use windows::core::{HSTRING, Interface};

use crate::{PlatformError, Result};

/// A voice that can read text aloud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    /// The system's identifier for it.
    pub id: String,
    /// e.g. "Microsoft Zira".
    pub name: String,
    /// e.g. "en-US".
    pub language: String,
}

/// What's being played, and whether it has ended.
static PLAYING: Mutex<Option<(MediaPlayer, Arc<AtomicBool>)>> = Mutex::new(None);

fn playing() -> std::sync::MutexGuard<'static, Option<(MediaPlayer, Arc<AtomicBool>)>> {
    PLAYING.locked()
}

fn failed(what: &str, e: windows::core::Error) -> PlatformError {
    PlatformError::Failed(format!("{what}: {e}"))
}

fn speaks(voice: &VoiceInformation, language: &str) -> bool {
    voice.Language().is_ok_and(|l| {
        let l = l.to_string().to_lowercase();
        l == language || l.starts_with(&format!("{language}-"))
    })
}

/// The chosen voice when it speaks `language` (or the language isn't
/// known); otherwise the default voice if it speaks it, or the first that
/// does.
fn pick_voice(id: Option<&str>, language: Option<&str>) -> Option<VoiceInformation> {
    let all: Vec<VoiceInformation> = SpeechSynthesizer::AllVoices().ok()?.into_iter().collect();
    let language = language.map(str::to_lowercase);
    let chosen = id.and_then(|id| all.iter().find(|v| v.Id().is_ok_and(|v| v == id)));
    if let Some(chosen) = chosen
        && language.as_deref().is_none_or(|l| speaks(chosen, l))
    {
        return Some(chosen.clone());
    }
    let language = language?;
    SpeechSynthesizer::DefaultVoice()
        .ok()
        .filter(|v| speaks(v, &language))
        .or_else(|| all.into_iter().find(|v| speaks(v, &language)))
}

/// Reads `text` aloud, stopping anything being read. `voice` is used when
/// it speaks `language` (the text's, "de"); otherwise a voice for the
/// language is. `speed` 1.0 is the voice's usual pace.
pub fn speak(text: &str, voice: Option<&str>, language: Option<&str>, speed: f32) -> Result<()> {
    stop_speaking();
    let synth = SpeechSynthesizer::new().map_err(|e| failed("no speech", e))?;
    if let Some(voice) = pick_voice(voice, language) {
        let _ = synth.SetVoice(&voice);
    }
    if let Ok(options) = synth.Options() {
        let _ = options.SetSpeakingRate(f64::from(speed.clamp(0.5, 2.0)));
    }
    let stream = synth
        .SynthesizeTextToStreamAsync(&HSTRING::from(text))
        .and_then(|op| op.join())
        .map_err(|e| failed("couldn't turn the text into speech", e))?;
    let kind = stream.ContentType().unwrap_or_default();
    let stream: IRandomAccessStream = stream.cast().map_err(|e| failed("no speech", e))?;
    let source =
        MediaSource::CreateFromStream(&stream, &kind).map_err(|e| failed("no speech", e))?;
    let player = MediaPlayer::new().map_err(|e| failed("couldn't play speech", e))?;
    // Kept out of the media controls Windows shows (and out of the players
    // that pausing music pauses).
    if let Ok(commands) = player.CommandManager() {
        let _ = commands.SetIsEnabled(false);
    }
    let ended = Arc::new(AtomicBool::new(false));
    let flag = ended.clone();
    let _ = player.MediaEnded(&TypedEventHandler::new(move |_, _| {
        flag.store(true, Ordering::SeqCst);
        Ok(())
    }));
    let flag = ended.clone();
    let _ = player.MediaFailed(&TypedEventHandler::new(move |_, _| {
        flag.store(true, Ordering::SeqCst);
        Ok(())
    }));
    player
        .SetSource(&source)
        .and_then(|()| player.Play())
        .map_err(|e| failed("couldn't play speech", e))?;
    *playing() = Some((player, ended));
    Ok(())
}

/// Stops reading at once.
pub fn stop_speaking() {
    if let Some((player, _)) = playing().take() {
        let _ = player.Pause();
        let _ = player.Close();
    }
}

/// Whether something is still being read.
pub fn speaking() -> bool {
    playing()
        .as_ref()
        .is_some_and(|(_, ended)| !ended.load(Ordering::SeqCst))
}

/// The voices that can read text.
pub fn voices() -> Vec<Voice> {
    let Ok(all) = SpeechSynthesizer::AllVoices() else {
        return Vec::new();
    };
    all.into_iter()
        .filter_map(|v| {
            Some(Voice {
                id: v.Id().ok()?.to_string(),
                name: v.DisplayName().ok()?.to_string(),
                language: v.Language().ok()?.to_string(),
            })
        })
        .collect()
}
