//! Faster text for long dictations. While the user is still speaking, the
//! recording up to each pause is transcribed in the background, so on release
//! only the stretch after the last pause is left to transcribe. The pieces
//! are joined before the AI pass (`splashdown_core::Prefix`); if any fails,
//! the whole recording is transcribed instead.
//!
//! Short dictations never reach a cut, so nothing changes for them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use splashdown_audio::{Cleanup, vad};
use splashdown_core::{AudioClip, Prefix, SpeechToText, SttHints, Usage};
use tauri::async_runtime::JoinHandle;

use crate::audio::AudioWorker;

/// Recording since the last cut before looking for a pause.
const FIRST_CHECK: Duration = Duration::from_secs(8);
/// Pauses this early after the last cut are skipped, so pieces aren't tiny.
const MIN_PIECE_SECS: usize = 6;
const CHECK_EVERY: Duration = Duration::from_secs(1);
/// A pause this long is a safe place to split words.
const MIN_PAUSE_MS: u32 = 400;
/// The newest audio might be the start of a word: don't split in it.
const GUARD_MS: u32 = 300;

type Piece = JoinHandle<Result<(String, Usage), String>>;

pub struct Chunker {
    stt: Arc<dyn SpeechToText>,
    hints: SttHints,
    /// Samples (in the recorder's buffer) already sent off.
    cut: usize,
    /// When the audio at `cut` was recorded, roughly.
    cut_at: Instant,
    checked_at: Instant,
    pieces: Vec<Piece>,
    /// Noise reduction and whisper mode, applied to each piece before it's
    /// transcribed.
    cleanup: Cleanup,
}

impl Chunker {
    pub fn new(stt: Arc<dyn SpeechToText>, hints: SttHints, cleanup: Cleanup) -> Self {
        let now = Instant::now();
        Self {
            stt,
            hints,
            cut: 0,
            cut_at: now,
            checked_at: now,
            pieces: Vec::new(),
            cleanup,
        }
    }

    /// Called often while recording: sends the audio up to a pause off to be
    /// transcribed, when one has come along.
    pub fn tick(&mut self, audio: &AudioWorker) {
        let now = Instant::now();
        if now.duration_since(self.cut_at) < FIRST_CHECK
            || now.duration_since(self.checked_at) < CHECK_EVERY
        {
            return;
        }
        self.checked_at = now;
        let clip = audio.since(self.cut);
        // Whisper mode: quiet speech is raised so its pauses show. Noise
        // reduction waits for the piece; it's too slow to run on the whole
        // stretch every second.
        let boosted = self
            .cleanup
            .quiet
            .then(|| splashdown_audio::gain::boost_quiet(&clip));
        let probe = boosted.as_ref().unwrap_or(&clip);
        let rate = clip.sample_rate.max(1) as usize;
        // The speech detector works on 16 kHz audio; the cut is then put
        // back at the recording's own rate.
        let target = splashdown_audio::TARGET_SAMPLE_RATE as usize;
        let probe = AudioClip::new(
            splashdown_audio::resample::resample(&probe.samples, probe.sample_rate, target as u32),
            target as u32,
        );
        let Some(pause) = vad::find_pause(&probe, MIN_PIECE_SECS * target, MIN_PAUSE_MS, GUARD_MS)
            .map(|at| (at * rate / target).min(clip.samples.len()))
        else {
            return;
        };
        let piece = AudioClip::new(clip.samples[..pause].to_vec(), clip.sample_rate);
        self.cut += pause;
        let after = Duration::from_secs_f64((clip.samples.len() - pause) as f64 / rate as f64);
        self.cut_at = now.checked_sub(after).unwrap_or(now);
        tracing::debug!(
            "transcribing {:.1} s ahead",
            piece.samples.len() as f64 / rate as f64
        );

        let (stt, hints, cleanup) = (self.stt.clone(), self.hints.clone(), self.cleanup);
        self.pieces.push(tauri::async_runtime::spawn(async move {
            let Some(clip) = splashdown_audio::prepare_with(&piece, cleanup) else {
                return Ok((String::new(), Usage::FREE));
            };
            stt.transcribe(&clip, &hints)
                .await
                .map(|t| (t.text.trim().to_string(), t.usage))
                .map_err(|e| e.to_string())
        }));
    }

    /// Recording ended. If anything was sent off, returns the prefix for the
    /// pipeline (with `whole`, the prepared recording, to fall back on) and
    /// the sample where the rest of the recording starts.
    pub fn finish(mut self, whole: AudioClip) -> Option<(Prefix, usize)> {
        if self.pieces.is_empty() {
            return None;
        }
        let pieces = std::mem::take(&mut self.pieces);
        let transcript = Box::pin(async move {
            let mut text = String::new();
            let mut usage = Usage::FREE;
            for piece in pieces {
                let result = piece.await.map_err(|e| e.to_string()).and_then(|r| r);
                let (piece, piece_usage) = result.inspect_err(|e| {
                    tracing::warn!("background transcription failed, starting over: {e}")
                })?;
                if !piece.is_empty() {
                    text = format!("{text} {piece}");
                }
                usage += piece_usage;
            }
            Ok((text.trim().to_string(), usage))
        });
        Some((Prefix { transcript, whole }, self.cut))
    }
}

impl Drop for Chunker {
    /// A recording that's canceled or becomes an instruction doesn't need
    /// its pieces.
    fn drop(&mut self) {
        for piece in &self.pieces {
            piece.abort();
        }
    }
}
