//! Platform-agnostic core of Splashdown: shared types, provider interfaces,
//! prompts, the sanity guard and the processing pipeline. Nothing in this
//! crate calls the OS; the only platform differences are compile-time
//! defaults and key names (`gesture::PhysicalKey`).

pub mod action;
pub mod ask;
mod audio;
pub mod command;
pub mod computer;
pub mod fallback;
pub mod files;
pub mod gesture;
pub mod guard;
pub mod local_llm;
pub mod meeting;
mod mode;
pub mod pipeline;
pub mod prompt;
pub mod provider;
pub mod spoken;
pub mod vocab;

pub use audio::AudioClip;
pub use mode::Mode;
pub use pipeline::{
    ActionOutcome, AudioJob, CommandOutcome, Fallback, Outcome, Pipeline, PipelineError, Prefix,
    TextJob,
};
pub use prompt::{CleanupLevel, PromptContext};
pub use provider::{
    Completion, OnText, ProviderError, ProviderResult, SpeechStream, SpeechToText, SttHints,
    TextModel, TextRequest, Transcript, Usage,
};
