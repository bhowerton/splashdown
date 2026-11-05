//! Spoken commands and voice actions: what was asked for, from the model
//! for instructions when one is set.

use std::sync::Arc;
use std::time::Instant;

use super::{ActionOutcome, CommandOutcome, Pipeline, PipelineError};
use crate::Mode;
use crate::command::{self, Command};
use crate::prompt::PromptContext;
use crate::provider::{ProviderError, TextModel};

impl Pipeline {
    /// Interprets a spoken command (see `crate::command`). `last` is the last
    /// dictation, if it's still just before the cursor; `actions`: voice
    /// actions are on. Uses the model for instructions when one is set.
    pub async fn command(
        &self,
        spoken: &str,
        last: Option<&str>,
        ctx: &PromptContext,
        actions: bool,
    ) -> Result<CommandOutcome, PipelineError> {
        self.interpret(spoken, last, ctx, actions, None).await
    }

    /// `command`, with the reply cut off after `max_tokens` if given.
    pub(super) async fn interpret(
        &self,
        spoken: &str,
        last: Option<&str>,
        ctx: &PromptContext,
        actions: bool,
        max_tokens: Option<u32>,
    ) -> Result<CommandOutcome, PipelineError> {
        let llm = self.command_llm()?;
        let mut request = command::request(spoken, last, ctx, actions);
        if let Some(max) = max_tokens {
            request.max_tokens = request.max_tokens.min(max);
        }
        if !llm.supports_json_schema() {
            request.json_schema = None;
        }
        let started = Instant::now();
        let completion = llm.complete(&request).await.map_err(PipelineError::Llm)?;
        let command = match command::parse(&completion.text, actions) {
            // Nothing to replace: it can only mean new text.
            Command::ReplaceLast(text) if last.is_none() => Command::Write(text),
            Command::Ask(question) if question.is_empty() => Command::Ask(spoken.to_string()),
            // What to write as said: the interpreter's copy may be
            // translated or reworded.
            Command::Compose(_) => Command::Compose(spoken.to_string()),
            // The words as said: the interpreter's copy may be translated
            // or reworded, and names and times must survive as they were.
            Command::Do(_) => Command::Do(spoken.to_string()),
            other => other,
        };
        Ok(CommandOutcome {
            command,
            llm_name: llm.name(),
            usage: completion.usage,
            llm_time: started.elapsed(),
        })
    }

    /// Plans a `do` command into one action from the catalog (see
    /// `crate::action`), limited to `enabled` (all when `None`).
    pub async fn plan_action(
        &self,
        said: &str,
        ctx: &crate::action::PlanContext,
        enabled: Option<&[&str]>,
    ) -> Result<ActionOutcome, PipelineError> {
        let llm = self.command_llm()?;
        let mut request = crate::action::request(said, ctx, enabled);
        if !llm.supports_json_schema() {
            request.json_schema = None;
        }
        let started = Instant::now();
        // Some models refuse a schema with a choice of shapes, or answer it
        // with blank space until they run out of room: ask again without it
        // (the reply is checked either way).
        let mut completion = match llm.complete(&request).await {
            Err(ProviderError::Api(e)) if request.json_schema.is_some() && e.starts_with("400") => {
                request.json_schema = None;
                llm.complete(&request).await
            }
            other => other,
        }
        .map_err(PipelineError::Llm)?;
        let mut usage = completion.usage;
        if request.json_schema.is_some() && !crate::action::readable(&completion.text) {
            request.json_schema = None;
            completion = llm.complete(&request).await.map_err(PipelineError::Llm)?;
            usage += completion.usage;
        }
        Ok(ActionOutcome {
            plan: crate::action::plan(&completion.text, said, ctx, enabled),
            llm_name: llm.name(),
            usage,
            llm_time: started.elapsed(),
        })
    }

    /// The model for commands and actions: the one for instructions when
    /// one is set.
    fn command_llm(&self) -> Result<&Arc<dyn TextModel>, PipelineError> {
        self.instruction_llm
            .as_ref()
            .or(self.llm.as_ref())
            .ok_or(PipelineError::NoModel(Mode::DictateInstruct))
    }
}
