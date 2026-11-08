//! Voiceprints: a speaker model's summary of someone's speech, to tell a
//! person again in a later meeting (`splashdown_core::voices`). NVIDIA
//! TitaNet-small, run by sherpa-onnx (ONNX Runtime) on the CPU: an hour of
//! a meeting's speech in about 20 s on Apple Silicon.

use std::path::Path;

#[cfg(not(windows))]
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

/// The rate the model takes.
#[cfg(not(windows))]
const RATE: i32 = splashdown_audio::TARGET_SAMPLE_RATE as i32;

#[cfg(not(windows))]
pub struct Voiceprinter {
    extractor: SpeakerEmbeddingExtractor,
}

/// Not on Windows yet (see Cargo.toml).
#[cfg(windows)]
pub struct Voiceprinter;

#[cfg(windows)]
impl Voiceprinter {
    pub fn load(_path: &Path) -> Result<Self, String> {
        Err("recognizing voices isn't available on Windows yet".into())
    }

    pub fn print(&self, _pcm: &[f32]) -> Option<Vec<f32>> {
        None
    }
}

#[cfg(not(windows))]
impl Voiceprinter {
    /// Loads the model at `path` (`models::VoiceModel`).
    pub fn load(path: &Path) -> Result<Self, String> {
        let config = SpeakerEmbeddingExtractorConfig {
            model: Some(path.to_string_lossy().into_owned()),
            num_threads: 2,
            ..Default::default()
        };
        SpeakerEmbeddingExtractor::create(&config)
            .map(|extractor| Self { extractor })
            .ok_or_else(|| format!("couldn't load the voice model at {}", path.display()))
    }

    /// The voiceprint of `pcm` (16 kHz mono), unit length; `None` if it
    /// couldn't be made.
    pub fn print(&self, pcm: &[f32]) -> Option<Vec<f32>> {
        let stream = self.extractor.create_stream()?;
        stream.accept_waveform(RATE, pcm);
        stream.input_finished();
        if !self.extractor.is_ready(&stream) {
            return None;
        }
        splashdown_core::voices::unit(self.extractor.compute(&stream)?)
    }
}
