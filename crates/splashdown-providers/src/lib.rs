//! Speech-to-text and text-model providers. The interfaces live in
//! `splashdown_core::provider`; this crate implements them.

mod anthropic;
pub mod apple;
mod chat;
mod download;
mod endpoint;
pub mod factory;
mod local_llm;
#[cfg(feature = "local-speech")]
mod local_speech;
pub mod models;
#[cfg(feature = "natural-voice")]
mod natural_voice;
pub mod pricing;
#[cfg(feature = "local-speech")]
mod silero;
mod transcribe;
#[cfg(feature = "voiceprints")]
mod voiceprint;

/// Whether this processor can run the speech library (speech recognition
/// and speech detection on this computer) as the app is built.
///
/// The Windows app's copy is compiled for AVX2 and its companions
/// (`scripts/win-cpu-floor.ps1`), and it has no slower path to fall back
/// to: on a processor without them, calling into it stops the app with
/// "illegal instruction". Everything that uses the library asks this first.
/// Always true on a Mac, where it's built for what every supported Mac has.
pub fn speech_library_runs_here() -> bool {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        std::arch::is_x86_feature_detected!("avx2")
            && std::arch::is_x86_feature_detected!("fma")
            && std::arch::is_x86_feature_detected!("f16c")
            && std::arch::is_x86_feature_detected!("bmi2")
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        true
    }
}

/// What to tell someone whose processor can't (`speech_library_runs_here`).
pub const SPEECH_NEEDS_NEWER_PROCESSOR: &str = "This computer's processor is too old for speech recognition on the computer itself (it needs AVX2, which PCs have had since about 2013). Choose a cloud provider for speech to text instead.";

pub use anthropic::AnthropicModel;
pub use chat::ChatModel;
pub use endpoint::{Endpoint, Flavor, OPENAI_BASE_URL, OPENROUTER_BASE_URL};
pub use local_llm::{LocalEmbedder, LocalLlm, stop_all_helpers};
#[cfg(feature = "local-speech")]
pub use local_speech::{
    LocalSpeech, Profile as SpeechProfile, SpeakerTurn, diarize, loading as speech_model_loading,
};
pub use models::{LocalModel, NaturalVoice, NaturalVoiceEntry, SpeechModel};
#[cfg(feature = "natural-voice")]
pub use natural_voice::NaturalSpeech;
#[cfg(feature = "local-speech")]
pub use silero::Silero;
pub use transcribe::{ChatAudioTranscriber, TranscriptionApi};
#[cfg(feature = "voiceprints")]
pub use voiceprint::Voiceprinter;
