//! Silero VAD, run by transcribe.cpp: a small model that tells speech from
//! silence, noise and music far better than loudness does, in any
//! language. Its weights (885 KB, MIT) are built in and written next to the
//! speech models the first time they're needed, so it works with no
//! download, whichever speech-to-text is chosen.

use std::path::Path;
use std::sync::Mutex;

use splashdown_audio::vad::SpeechDetector;
use transcribe_cpp::{Backend, Model, ModelOptions, VadOptions, VadSession};

/// `ggml-org/whisper-vad`, Silero v6.2.0 in whisper.cpp's format.
const WEIGHTS: &[u8] = include_bytes!("../models/ggml-silero-v6.2.0.bin");
const FILE: &str = "ggml-silero-v6.2.0.bin";

pub struct Silero {
    session: Mutex<VadSession>,
    frame: usize,
    // Keeps the weights loaded for the session.
    _model: Model,
}

impl Silero {
    /// Loads the model, writing its file into `models_dir` first if it
    /// isn't there.
    pub fn load(models_dir: &Path) -> Result<Silero, String> {
        let path = models_dir.join(FILE);
        let current = std::fs::metadata(&path).is_ok_and(|m| m.len() == WEIGHTS.len() as u64);
        if !current {
            std::fs::create_dir_all(models_dir).map_err(|e| e.to_string())?;
            // A new file renamed into place, never rewritten in place: a
            // loaded model may be mapped from it.
            let tmp = path.with_extension("bin.part");
            std::fs::write(&tmp, WEIGHTS).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        }
        transcribe_cpp::disable_logging();
        // It's tiny: the CPU is quickest, and keeps the GPU for speech.
        let options = ModelOptions {
            backend: Backend::Cpu,
            ..Default::default()
        };
        let model = Model::load_with(&path, &options).map_err(|e| e.to_string())?;
        let frame = model.vad_info().map_err(|e| e.to_string())?.frame_samples as usize;
        let session = model.vad_session().map_err(|e| e.to_string())?;
        Ok(Silero {
            session: Mutex::new(session),
            frame,
            _model: model,
        })
    }
}

impl SpeechDetector for Silero {
    fn frame(&self) -> usize {
        self.frame
    }

    fn probabilities(&self, samples: &[f32]) -> Option<Vec<f32>> {
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        match session.run(samples, &VadOptions::default()) {
            Ok(result) => Some(result.probs),
            Err(e) => {
                tracing::warn!("speech detection failed: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silero() -> Silero {
        let dir = std::env::temp_dir().join(format!("splashdown-silero-{}", std::process::id()));
        Silero::load(&dir).unwrap()
    }

    #[test]
    fn silence_and_noise_arent_speech() {
        let silero = silero();
        assert_eq!(silero.frame(), 512);
        let silence = vec![0.0f32; 16_000];
        let probs = silero.probabilities(&silence).unwrap();
        // 31¼ frames: the last, partial one is scored too.
        assert_eq!(probs.len(), 32);
        assert!(probs.iter().all(|&p| p < 0.2), "{probs:?}");
        // A loud hum, which loudness takes for speech.
        let hum: Vec<f32> = (0..16_000)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 120.0 / 16_000.0).sin() * 0.3)
            .collect();
        let probs = silero.probabilities(&hum).unwrap();
        assert!(probs.iter().all(|&p| p < 0.5), "{probs:?}");
    }

    /// Speed and results on a real recording:
    /// `WAV=path cargo test -p splashdown-providers --release -- --ignored --nocapture silero`.
    #[test]
    #[ignore = "needs a recording in WAV"]
    fn silero_on_a_recording() {
        let path = std::env::var("WAV").expect("WAV=path to a 16 kHz recording");
        let clip = splashdown_audio::wav::read(Path::new(&path)).unwrap();
        let silero = silero();
        let started = std::time::Instant::now();
        let probs = silero.probabilities(&clip.samples).unwrap();
        let took = started.elapsed();
        let speech = probs.iter().filter(|&&p| p >= 0.5).count();
        // Each frame's chance of speech, 0–9.
        let digits: String = probs
            .iter()
            .map(|p| char::from(b'0' + (p * 9.0).round() as u8))
            .collect();
        println!("{digits}");
        println!(
            "{:.1} s of audio in {:.3} s; {} of {} frames speech",
            clip.duration().as_secs_f32(),
            took.as_secs_f32(),
            speech,
            probs.len()
        );
    }
}
