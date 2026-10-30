//! Speech-to-text and text-model providers (spec §5). The interfaces live in
//! `splashdown_core::provider`; this crate implements them.

mod anthropic;
pub mod apple;
mod chat;
mod endpoint;
pub mod factory;
mod local_llm;
pub mod models;
pub mod pricing;
mod transcribe;
#[cfg(feature = "whisper-local")]
mod whisper;

pub use anthropic::AnthropicModel;
pub use chat::ChatModel;
pub use endpoint::{Endpoint, Flavor, OPENAI_BASE_URL, OPENROUTER_BASE_URL};
pub use local_llm::LocalLlm;
pub use models::{LocalModel, WhisperModel};
pub use transcribe::{ChatAudioTranscriber, TranscriptionApi};
#[cfg(feature = "whisper-local")]
pub use whisper::WhisperLocal;
