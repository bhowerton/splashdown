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
