//! Platform-agnostic core of Splashdown: shared types, provider interfaces,
//! prompts, the sanity guard and the processing pipeline. Nothing in this
//! crate calls the OS; the only platform differences are compile-time
//! defaults and key names (`gesture::PhysicalKey`).

mod audio;
pub mod command;
pub mod fallback;
pub mod files;
pub mod gesture;
pub mod guard;
pub mod local_llm;
mod mode;
pub mod pipeline;
pub mod prompt;
pub mod provider;
pub mod spoken;
pub mod vocab;

pub use audio::AudioClip;
pub use mode::Mode;
pub use pipeline::{
    AudioJob, CommandOutcome, Fallback, Outcome, Pipeline, PipelineError, Prefix, TextJob,
};
pub use prompt::{CleanupLevel, PromptContext};
pub use provider::{
    Completion, ProviderError, ProviderResult, SpeechToText, SttHints, TextModel, TextRequest,
    Transcript, Usage,
};
