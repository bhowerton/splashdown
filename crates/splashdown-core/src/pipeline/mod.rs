//! Turns a recording (or text) into the final text to insert:
//! speech-to-text → AI pass → sanity guard.
//!
//! - `audio`: from a recording, speech-to-text and then the text pass.
//! - `text`: the AI pass on text, then the guard.
//! - `commands`: spoken commands and voice actions.

mod audio;
mod commands;
#[cfg(test)]
mod tests;
mod text;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::guard::GuardReason;
use crate::prompt::PromptContext;
use crate::provider::{ProviderError, SpeechToText, SttHints, TextModel, Usage};
use crate::{AudioClip, Mode};

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("didn't catch that — no speech found")]
    NoSpeech,
    #[error("{0:?} needs an AI model, but none is configured")]
    NoModel(Mode),
    #[error("transform needs selected text")]
    NoSelection,
    #[error("speech-to-text failed: {0}")]
    Stt(ProviderError),
    #[error("AI pass failed: {0}")]
    Llm(ProviderError),
}

/// Why the raw transcript was inserted instead of the AI output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    Guard(GuardReason),
    /// The model call failed; dictation still goes through.
    ModelError(String),
}

/// Text input to the AI pass.
#[derive(Debug, Clone)]
pub struct TextJob {
    pub mode: Mode,
    /// Dictated content (Dictate, DictateInstruct, Raw) — unused for Transform.
    pub content: String,
    /// Spoken instruction (DictateInstruct, Transform).
    pub instruction: Option<String>,
    /// Selected text (Transform).
    pub selection: Option<String>,
    pub context: PromptContext,
}

/// A prefix's text and what transcribing it cost, once it's ready.
pub type PrefixTranscript = Pin<Box<dyn Future<Output = Result<(String, Usage), String>> + Send>>;

/// The start of a long dictation, transcribed in the background while the
/// user was still speaking. `AudioJob::speech` is then only what came after.
pub struct Prefix {
    /// The prefix's text and what transcribing it cost.
    pub transcript: PrefixTranscript,
    /// The whole recording, transcribed instead if the prefix failed.
    pub whole: AudioClip,
}

impl std::fmt::Debug for Prefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prefix").finish_non_exhaustive()
    }
}

/// Audio input. For Transform, `speech` is the instruction.
#[derive(Debug)]
pub struct AudioJob {
    pub mode: Mode,
    pub speech: AudioClip,
    /// Speech after the instruction-key tap (DictateInstruct).
    pub instruction_speech: Option<AudioClip>,
    pub selection: Option<String>,
    pub context: PromptContext,
    pub hints: SttHints,
    /// An instruction applied to plain dictation when none was spoken (a
    /// preset's "Translate into …").
    pub standing_instruction: Option<String>,
    /// Speech before `speech`, already being transcribed (plain dictation
    /// only; see `Prefix`).
    pub prefix: Option<Prefix>,
}

/// What a spoken command asked for, and what interpreting it cost.
#[derive(Debug, Clone)]
pub struct CommandOutcome {
    pub command: crate::command::Command,
    pub llm_name: String,
    pub usage: Usage,
    pub llm_time: Duration,
}

/// The action a `do` command was planned into, and what planning it cost.
#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub plan: crate::action::Plan,
    pub llm_name: String,
    pub usage: Usage,
    pub llm_time: Duration,
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub mode: Mode,
    /// Transcript of the content (or of the instruction, for Transform).
    pub raw: String,
    pub instruction: Option<String>,
    /// What to insert.
    pub text: String,
    pub fallback: Option<Fallback>,
    pub stt_time: Duration,
    pub llm_time: Duration,
    pub stt_name: Option<String>,
    pub llm_name: Option<String>,
    /// What the provider calls cost (speech-to-text and AI pass together).
    pub usage: Usage,
    /// The AI pass's share of `usage`.
    pub llm_usage: Usage,
    /// With voice actions on, a Transform's instruction asked to do
    /// something ("email this to Sam") rather than to change the
    /// selection: the words as said, for `plan_action`. `text` is empty.
    pub action: Option<String>,
    /// A Transform's instruction asked to hear the selection read aloud:
    /// "" to read it as it is, otherwise what to answer and read out
    /// instead ("summarize this"). `text` is empty.
    pub read: Option<String>,
    /// A Transform's instruction asked for a draft that answers the
    /// selection ("reply to this saying I'm in"): what to write, as said.
    /// `text` is empty.
    pub compose: Option<String>,
}

pub struct Pipeline {
    stt: Arc<dyn SpeechToText>,
    llm: Option<Arc<dyn TextModel>>,
    /// Handles instructions and transforms instead of `llm`, when set: a
    /// stronger cloud model behind a local one that does plain cleanup.
    instruction_llm: Option<Arc<dyn TextModel>>,
    /// Voice actions are on: an instruction for a selection may be one.
    actions: bool,
}

impl Pipeline {
    pub fn new(stt: Arc<dyn SpeechToText>, llm: Option<Arc<dyn TextModel>>) -> Self {
        Self {
            stt,
            llm,
            instruction_llm: None,
            actions: false,
        }
    }

    /// With voice actions on, an instruction for selected text is first
    /// checked for a request to do something (see `Outcome::action`).
    pub fn with_actions(mut self, on: bool) -> Self {
        self.actions = on;
        self
    }

    /// Sends instructions and transforms to `model` (if given) instead of
    /// the main model. Needs a main model; plain dictation keeps using it.
    pub fn with_instruction_model(mut self, model: Option<Arc<dyn TextModel>>) -> Self {
        self.instruction_llm = model;
        self
    }

    /// e.g. `openrouter/google/gemini-3.1-flash-lite`.
    pub fn stt_name(&self) -> String {
        self.stt.name()
    }

    /// The speech-to-text provider, for transcribing ahead in the
    /// background (see `Prefix`).
    pub fn stt(&self) -> Arc<dyn SpeechToText> {
        self.stt.clone()
    }

    pub fn llm_name(&self) -> Option<String> {
        self.llm.as_ref().map(|l| l.name())
    }

    /// Lets the models prepare while the user is still speaking.
    pub fn warm_up(&self) {
        self.stt.warm_up();
        for llm in self.llm.iter().chain(&self.instruction_llm) {
            llm.warm_up();
        }
    }

    /// The model for questions (Ask): the one for instructions, if set.
    pub fn answer_model(&self) -> Option<Arc<dyn TextModel>> {
        self.instruction_llm.clone().or_else(|| self.llm.clone())
    }

    pub fn instruction_llm_name(&self) -> Option<String> {
        self.instruction_llm.as_ref().map(|l| l.name())
    }
}
