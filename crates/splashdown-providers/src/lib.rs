//! Speech-to-text and text-model providers (spec §5). The interfaces live in
//! `splashdown_core::provider`; this crate implements them.

mod anthropic;
pub mod apple;
mod chat;
mod endpoint;
pub mod factory;
mod local_llm;
#[cfg(feature = "local-speech")]
mod local_speech;
pub mod models;
pub mod pricing;
mod transcribe;

pub use anthropic::AnthropicModel;
pub use chat::ChatModel;
pub use endpoint::{Endpoint, Flavor, OPENAI_BASE_URL, OPENROUTER_BASE_URL};
pub use local_llm::LocalLlm;
#[cfg(feature = "local-speech")]
pub use local_speech::LocalSpeech;
pub use models::{LocalModel, SpeechModel};
pub use transcribe::{ChatAudioTranscriber, TranscriptionApi};
