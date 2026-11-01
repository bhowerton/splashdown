//! Never losing a dictation: each recording is written to disk while it's
//! being processed and deleted once its text is in. If speech-to-text or
//! the AI failed (offline, an outage, a bad key), or the app closed first,
//! it's kept instead, and "Retry last dictation" in the menu bar runs it
//! again. One failed recording is kept (the latest), whatever the history
//! setting; a dictation that works doesn't replace it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use splashdown_core::AudioClip;

/// What's known about a kept recording.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Kept {
    /// When it was recorded (Unix ms).
    pub at: u64,
    /// The app it was dictated into.
    pub app: Option<String>,
    /// The preset it was dictated with.
    pub preset: Option<String>,
    /// Why it's kept.
    pub reason: String,
}

/// The recording being processed now, or the failed one kept.
#[derive(Clone, Copy)]
enum Slot {
    Processing,
    Failed,
}

impl Slot {
    fn file(self, dir: &Path, part: &str) -> PathBuf {
        let name = match self {
            Slot::Processing => "dictation-processing",
            Slot::Failed => "dictation-failed",
        };
        dir.join(format!("{name}{part}"))
    }

    fn files(self, dir: &Path) -> [PathBuf; 3] {
        [".wav", "-instruction.wav", ".json"].map(|part| self.file(dir, part))
    }
}

fn dir() -> Option<PathBuf> {
    crate::engine::paths().ok().map(|p| p.config_dir)
}

/// Keeps a recording while it's processed.
pub fn start(speech: &AudioClip, instruction: Option<&AudioClip>, kept: &Kept) {
    if let Some(dir) = dir() {
        start_in(&dir, speech, instruction, kept);
    }
}

fn start_in(dir: &Path, speech: &AudioClip, instruction: Option<&AudioClip>, kept: &Kept) {
    let [wav, instruction_wav, meta] = Slot::Processing.files(dir);
    let written = std::fs::write(wav, splashdown_audio::wav::encode(speech))
        .and_then(|()| match instruction {
            Some(clip) => std::fs::write(&instruction_wav, splashdown_audio::wav::encode(clip)),
            None => remove(&instruction_wav),
        })
        .and_then(|()| std::fs::write(meta, serde_json::to_string(kept)?));
    if let Err(e) = written {
        tracing::warn!("couldn't keep the recording while it's processed: {e}");
    }
}

/// Its text is in (or there was no speech in it): nothing to keep.
pub fn done() {
    if let Some(dir) = dir() {
        clear_slot(&dir, Slot::Processing);
    }
}

/// It couldn't be turned into text: keeps it for Retry, saying why.
pub fn failed(reason: &str) {
    if let Some(dir) = dir() {
        failed_in(&dir, reason);
    }
}

fn failed_in(dir: &Path, reason: &str) {
    let from = Slot::Processing.files(dir);
    if !from[0].exists() {
        return;
    }
    clear_slot(dir, Slot::Failed);
    for (from, to) in from.iter().zip(Slot::Failed.files(dir)) {
        if from.exists()
            && let Err(e) = std::fs::rename(from, &to)
        {
            tracing::warn!("couldn't keep the failed recording: {e}");
        }
    }
    let meta = Slot::Failed.file(dir, ".json");
    let mut kept = read_meta(&meta).unwrap_or_default();
    kept.reason = reason.to_string();
    let _ = serde_json::to_string(&kept).map(|json| std::fs::write(meta, json));
}

/// At launch: a recording still being processed means the app closed
/// before its text was in.
pub fn recover() {
    failed("Splashdown closed before it finished");
}

/// The kept recording, if there is one.
pub fn kept() -> Option<Kept> {
    kept_in(&dir()?)
}

fn kept_in(dir: &Path) -> Option<Kept> {
    let [wav, _, meta] = Slot::Failed.files(dir);
    wav.exists().then(|| read_meta(&meta).unwrap_or_default())
}

/// The kept recording's audio: the speech and, if one was spoken, the
/// instruction after it.
pub fn audio() -> Option<(AudioClip, Option<AudioClip>)> {
    audio_in(&dir()?)
}

fn audio_in(dir: &Path) -> Option<(AudioClip, Option<AudioClip>)> {
    let [wav, instruction, _] = Slot::Failed.files(dir);
    let speech = splashdown_audio::wav::read(&wav).ok()?;
    Some((speech, splashdown_audio::wav::read(&instruction).ok()))
}

/// Retried successfully: the kept recording goes.
pub fn forget() {
    if let Some(dir) = dir() {
        clear_slot(&dir, Slot::Failed);
    }
}

fn clear_slot(dir: &Path, slot: Slot) {
    for path in slot.files(dir) {
        if let Err(e) = remove(&path) {
            tracing::warn!("couldn't delete {}: {e}", path.display());
        }
    }
}

fn read_meta(path: &Path) -> Option<Kept> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_recording_is_kept_until_retried() {
        let dir = std::env::temp_dir().join(format!("splashdown-rescue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let clip = |n: usize| AudioClip::new(vec![0.25; n], 16_000);
        let kept = |app: &str| Kept {
            at: 1,
            app: Some(app.into()),
            preset: None,
            reason: String::new(),
        };

        // Worked: nothing kept.
        start_in(&dir, &clip(1600), None, &kept("Mail"));
        clear_slot(&dir, Slot::Processing);
        assert!(kept_in(&dir).is_none());

        // Failed: kept, with the instruction and why.
        start_in(&dir, &clip(1600), Some(&clip(800)), &kept("Slack"));
        failed_in(&dir, "OpenRouter is offline");
        let k = kept_in(&dir).unwrap();
        assert_eq!(k.app.as_deref(), Some("Slack"));
        assert_eq!(k.reason, "OpenRouter is offline");
        let (speech, instruction) = audio_in(&dir).unwrap();
        assert_eq!(speech.samples.len(), 1600);
        assert_eq!(instruction.unwrap().samples.len(), 800);

        // A later dictation that works leaves it alone.
        start_in(&dir, &clip(3200), None, &kept("Mail"));
        clear_slot(&dir, Slot::Processing);
        assert_eq!(kept_in(&dir).unwrap().app.as_deref(), Some("Slack"));

        // A later failure replaces it, without an instruction left over.
        start_in(&dir, &clip(3200), None, &kept("Notes"));
        failed_in(&dir, "closed");
        assert_eq!(kept_in(&dir).unwrap().app.as_deref(), Some("Notes"));
        assert!(audio_in(&dir).unwrap().1.is_none());

        // Nothing in progress: nothing to recover.
        failed_in(&dir, "closed again");
        assert_eq!(kept_in(&dir).unwrap().reason, "closed");

        clear_slot(&dir, Slot::Failed);
        assert!(kept_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
