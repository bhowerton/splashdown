//! The natural reading voice: Gradium's Phonon, English, on the CPU (it's
//! faster there than on a Mac's GPU, and the same everywhere). Text in, 24
//! kHz audio out, a piece at a time while the rest is still being made.

use std::path::Path;

use ptts::checkpoint::{Checkpoint, ResolveOptions};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Quant, SpeechOptions, Synth};

use crate::models::NaturalVoice;

/// The loaded model and its voices.
pub struct NaturalSpeech {
    synth: Synth,
}

impl std::fmt::Debug for NaturalSpeech {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NaturalSpeech").finish_non_exhaustive()
    }
}

impl NaturalSpeech {
    /// Loads the downloaded model and its voices from `models_dir`.
    pub fn load(models_dir: &Path) -> Result<Self, String> {
        let dir = NaturalVoice::dir_in(models_dir);
        let options = ResolveOptions {
            quant: Quant::Q80,
            weights: None,
        };
        let mut checkpoint = Checkpoint::resolve(&dir, options).map_err(|e| e.to_string())?;
        checkpoint
            .add_voice_catalog(dir.join(NaturalVoice::CATALOG))
            .map_err(|e| e.to_string())?;
        let mut synth = checkpoint
            .builder(Lang::En)
            .device(DeviceKind::Cpu)
            .build()
            .map_err(|e| e.to_string())?;
        checkpoint.register_voices(&mut synth);
        Ok(Self { synth })
    }

    /// Samples per second of what `speak` gives.
    pub fn sample_rate(&self) -> u32 {
        self.synth.sample_rate()
    }

    /// The voices it can read in, by name.
    pub fn voices(&self) -> Vec<String> {
        self.synth.voices()
    }

    /// Starts reading `text` in `voice` (the default when `None` or not
    /// known): mono samples at `sample_rate`, a piece at a time. Making
    /// them stops when the iterator is dropped.
    pub fn speak(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<impl Iterator<Item = Result<Vec<f32>, String>> + use<>, String> {
        let mut options = SpeechOptions::default();
        if let Some(voice) = voice.filter(|v| self.synth.voices().iter().any(|n| n == v)) {
            options = options.voice(voice);
        }
        let stream = self
            .synth
            .stream_with(text, &options)
            .map_err(|e| e.to_string())?;
        Ok(stream.map(|piece| piece.map_err(|e| e.to_string())))
    }
}
