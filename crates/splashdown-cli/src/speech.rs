//! Speech-to-text for the CLI: transcribing a file of any length a piece
//! at a time, loading the chosen speech model, the mode a recording is
//! run in, and reading text aloud (`say`).

use super::*;

/// Prints the file's transcript as it goes: a title, then a paragraph per
/// piece starting with its time. Timings go to stderr.
pub(super) fn transcribe_file(
    rt: &tokio::runtime::Runtime,
    path: &std::path::Path,
    stt: &dyn splashdown_core::SpeechToText,
    hints: &splashdown_core::SttHints,
    cleanup: splashdown_audio::Cleanup,
) -> Result<()> {
    use splashdown_audio::file::{AudioFile, timestamp};

    let mut file = AudioFile::open(path).with_context(|| format!("reading {}", path.display()))?;
    let length = file.duration();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    println!("# {stem}");
    let start = std::time::Instant::now();
    let mut audio = std::time::Duration::ZERO;
    while let Some(piece) = file.next_piece()? {
        audio = piece.start + piece.clip.duration();
        let Some(clip) = splashdown_audio::prepare_with(&piece.clip, cleanup) else {
            continue;
        };
        let text = rt.block_on(stt.transcribe(&clip, hints))?.text;
        if !text.trim().is_empty() {
            println!("\n**[{}]** {}", timestamp(piece.start), text.trim());
        }
    }
    eprintln!(
        "transcribed {:.0} s of audio (the file says {}) in {:.1} s with {}",
        audio.as_secs_f64(),
        length.map_or("nothing".into(), |d| format!("{:.0} s", d.as_secs_f64())),
        start.elapsed().as_secs_f64(),
        stt.name()
    );
    Ok(())
}

pub(super) fn mode_for(raw: bool, has_selection: bool, has_instruction: bool) -> Mode {
    match (raw, has_selection, has_instruction) {
        (true, _, _) => Mode::Raw,
        (_, true, _) => Mode::Transform,
        (_, _, true) => Mode::DictateInstruct,
        _ => Mode::Dictate,
    }
}

pub(super) fn load_speech(
    path: &std::path::Path,
    cleanup: splashdown_audio::Cleanup,
) -> Result<Option<splashdown_core::AudioClip>> {
    let clip =
        splashdown_audio::wav::read(path).with_context(|| format!("reading {}", path.display()))?;
    let start = std::time::Instant::now();
    let prepared = splashdown_audio::prepare_with(&clip, cleanup);
    // For comparing the cost of --denoise and --whisper.
    eprintln!(
        "prepared {:.1} s of audio in {} ms",
        clip.duration().as_secs_f32(),
        start.elapsed().as_millis()
    );
    Ok(prepared)
}

/// `say`: loads the natural voice, reads `text` and reports the timings.
#[cfg(feature = "natural-voice")]
pub(super) fn say(text: &str, voice: Option<&str>) -> Result<()> {
    use std::time::Instant;
    let models_dir = opts::paths()?.models_dir;
    let started = Instant::now();
    let speech = splashdown_providers::NaturalSpeech::load(&models_dir).map_err(|e| {
        anyhow::anyhow!("{e} — download it with `splashdown-cli models download phonon-en`")
    })?;
    eprintln!(
        "loaded in {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    if voice == Some("list") {
        for v in splashdown_providers::NaturalVoice::voices(&models_dir) {
            println!(
                "{:<12} {} {}",
                v.name,
                v.locale,
                v.description.unwrap_or_default()
            );
        }
        return Ok(());
    }
    let started = Instant::now();
    let rate = speech.sample_rate();
    let mut first = None;
    let mut samples = 0usize;
    let pieces = speech
        .speak(text, voice)
        .map_err(anyhow::Error::msg)?
        .inspect(|p| {
            first.get_or_insert_with(|| started.elapsed());
            if let Ok(p) = p {
                samples += p.len();
            }
        });
    let stop = std::sync::atomic::AtomicBool::new(false);
    splashdown_audio::playback::play(pieces, rate, &stop).map_err(anyhow::Error::msg)?;
    let first = first.unwrap_or_default();
    eprintln!(
        "first sound after {:.0} ms; {:.1} s of speech",
        first.as_secs_f64() * 1000.0,
        samples as f64 / f64::from(rate)
    );
    Ok(())
}
