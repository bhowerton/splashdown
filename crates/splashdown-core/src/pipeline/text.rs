//! The AI pass on text (cleanup, an instruction, or a transform of the
//! selection), then the sanity guard.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Fallback, Outcome, Pipeline, PipelineError, TextJob};
use crate::Mode;
use crate::command::Command;
use crate::guard::{self, GuardReason, Verdict};
use crate::prompt::{self, PromptTask};
use crate::provider::{
    Completion, OnText, ProviderError, ProviderResult, TextModel, TextRequest, Usage,
};
use crate::spoken::{self, Snippet};

const TEMPERATURE: f32 = 0.2;

/// Checking an instruction for selected text (is it "read this to me", or
/// an action?) only needs the start of the selection, and a short reply:
/// the answers that matter are short, and a cut-off one counts as neither.
const CHECK_SELECTION_CHARS: usize = 300;
const CHECK_TOKENS: u32 = 120;

impl Pipeline {
    /// Runs the AI pass and sanity guard on already-transcribed text.
    pub async fn process_text(&self, job: TextJob) -> Result<Outcome, PipelineError> {
        self.process_text_streaming(job, None).await
    }

    /// `process_text`, streaming as `run_streaming` describes.
    pub async fn process_text_streaming(
        &self,
        mut job: TextJob,
        on_text: Option<&mut OnText<'_>>,
    ) -> Result<Outcome, PipelineError> {
        let raw = match job.mode {
            Mode::Transform => job.instruction.clone().unwrap_or_default(),
            _ => job.content.clone(),
        };
        let snippets = Snippets::mark(&mut job);
        let mut outcome = Outcome::start(&job, raw, snippets.expanded.clone());

        // Raw mode, or nothing but a snippet was said: no AI needed.
        if job.mode == Mode::Raw || (job.mode == Mode::Dictate && snippets.only_one(&job.content)) {
            return Ok(outcome);
        }
        let Some(llm) = self.text_llm(&job) else {
            return match job.mode {
                // No model configured: dictation still works, unpolished.
                Mode::Dictate => Ok(outcome),
                m => Err(PipelineError::NoModel(m)),
            };
        };
        outcome.llm_name = Some(llm.name());

        let selection = job.selection.as_deref().unwrap_or_default();
        let instruction = job.instruction.as_deref().unwrap_or_default();
        let task = match job.mode {
            Mode::Transform if selection.is_empty() => return Err(PipelineError::NoSelection),
            Mode::Transform => PromptTask::Transform {
                selection,
                instruction,
            },
            Mode::DictateInstruct if !instruction.is_empty() => PromptTask::Instruct {
                content: &job.content,
                instruction,
            },
            _ => PromptTask::Cleanup {
                transcript: &job.content,
            },
        };
        let prompt = prompt::build(task, &job.context);
        let request = TextRequest {
            system: prompt.system,
            user: prompt.user,
            temperature: TEMPERATURE,
            max_tokens: prompt.max_tokens,
            json_schema: None,
        };
        let source = [job.content.as_str(), instruction, selection].join("\n");
        // A translation is judged by its length against what it translates.
        let translated = match job.mode {
            Mode::Transform if guard::is_translation(instruction) => Some(selection),
            Mode::DictateInstruct if guard::is_translation(instruction) => {
                Some(job.content.as_str())
            }
            _ => None,
        };

        // An instruction for selected text may ask to hear it read aloud,
        // or (with voice actions on) to do something with it ("email this
        // to Sam") rather than to change it. That's checked alongside the
        // transform, so it takes no longer.
        let check = job.mode == Mode::Transform
            && !job.context.saved_instruction
            && !selection.trim().is_empty()
            && !instruction.trim().is_empty();
        let started = Instant::now();
        let result = if check {
            let (command, result) = futures_util::future::join(
                self.interpret(
                    instruction,
                    Some(&head(selection, CHECK_SELECTION_CHARS)),
                    &job.context,
                    self.actions,
                    Some(CHECK_TOKENS),
                ),
                llm.complete(&request),
            )
            .await;
            // An error here leaves it a transform.
            if let Ok(command) = command {
                outcome.add_llm_usage(command.usage);
                if outcome.take_request(command.command) {
                    if let Ok(transformed) = &result {
                        outcome.add_llm_usage(transformed.usage);
                    }
                    outcome.llm_time = started.elapsed();
                    outcome.text = String::new();
                    return Ok(outcome);
                }
            }
            result
        } else {
            let streams =
                matches!(job.mode, Mode::Dictate | Mode::DictateInstruct) && snippets.none();
            complete(
                llm,
                &request,
                on_text.filter(|_| streams),
                task.mode(),
                &source,
                &job.context.earlier,
                translated,
            )
            .await
        };
        outcome.llm_time = started.elapsed();

        let output = match result {
            Ok(completion) => {
                outcome.add_llm_usage(completion.usage);
                completion.text
            }
            // Keep the user's words rather than losing them; instructions
            // and transforms can't be done without the model.
            Err(err) if matches!(job.mode, Mode::Dictate | Mode::DictateInstruct) => {
                // A reply that stopped part way still cost something.
                if let ProviderError::Interrupted { usage, .. } = &err {
                    outcome.add_llm_usage(*usage);
                }
                outcome.fallback = Some(Fallback::ModelError(err.to_string()));
                return Ok(outcome);
            }
            Err(err) => return Err(PipelineError::Llm(err)),
        };

        // Small models sometimes write out the earlier dictations they were
        // given for reference before the new text.
        let output = match task.mode() {
            Mode::Dictate | Mode::DictateInstruct => {
                guard::drop_repeated_earlier(&output, &source, &job.context.earlier)
                    .unwrap_or(output)
            }
            _ => output,
        };
        let verdict = match translated.and_then(|m| guard::check_translation(m, &output)) {
            Some(reason) => Verdict::UseRaw(reason),
            None => guard::check(task.mode(), &source, &output),
        };
        match verdict {
            Verdict::Accept(text) => match snippets.fill(&text) {
                Some(filled) => outcome.text = filled,
                None => outcome.fallback = Some(Fallback::Guard(GuardReason::LostSnippet)),
            },
            Verdict::UseRaw(reason) => {
                outcome.fallback = Some(Fallback::Guard(reason));
                // For a transform the "raw" text is the untouched selection.
                if job.mode == Mode::Transform {
                    outcome.text = selection.to_string();
                }
            }
        }
        Ok(outcome)
    }

    /// The model for this job: the one for instructions when it follows
    /// one and both are set, otherwise the main one.
    fn text_llm(&self, job: &TextJob) -> Option<&Arc<dyn TextModel>> {
        let follows_instruction = job.mode == Mode::Transform
            || (job.mode == Mode::DictateInstruct
                && job.instruction.as_deref().is_some_and(|i| !i.is_empty()));
        match &self.instruction_llm {
            Some(cloud) if follows_instruction && self.llm.is_some() => Some(cloud),
            _ => self.llm.as_ref(),
        }
    }
}

/// The model's reply. With `on_text`, it streams, and the reply so far is
/// passed on until the guard would reject it.
async fn complete(
    llm: &Arc<dyn TextModel>,
    request: &TextRequest,
    on_text: Option<&mut OnText<'_>>,
    mode: Mode,
    source: &str,
    earlier: &[String],
    translated: Option<&str>,
) -> ProviderResult<Completion> {
    let Some(on_text) = on_text else {
        return llm.complete(request).await;
    };
    let mut passing = true;
    let mut forward = |so_far: &str| {
        if !passing {
            return;
        }
        // Already far longer than a translation would be.
        if translated.is_some_and(|m| so_far.chars().count() as f32 > guard::longest_translation(m))
        {
            passing = false;
            return;
        }
        match guard::partial(mode, source, earlier, so_far) {
            guard::Partial::Show => on_text(so_far),
            guard::Partial::Wait => {}
            guard::Partial::Stop => passing = false,
        }
    };
    llm.complete_streaming(request, &mut forward).await
}

/// The snippets said in a dictation. The model sees placeholders, never the
/// saved text, which is filled back in afterwards.
struct Snippets {
    saved: Vec<Snippet>,
    /// Which of `saved` were said.
    used: Vec<usize>,
    /// The dictation with them filled in.
    expanded: String,
}

impl Snippets {
    /// Swaps the snippets said in `job.content` for placeholders.
    fn mark(job: &mut TextJob) -> Self {
        let saved = std::mem::take(&mut job.context.snippets);
        let (marked, used) = if job.mode == Mode::Transform {
            (job.content.clone(), Vec::new())
        } else {
            spoken::mark_snippets(&job.content, &saved)
        };
        let expanded =
            spoken::fill_snippets(&marked, &used, &saved).unwrap_or_else(|| job.content.clone());
        job.content = marked;
        job.context.has_placeholders = !used.is_empty();
        Self {
            saved,
            used,
            expanded,
        }
    }

    fn none(&self) -> bool {
        self.used.is_empty()
    }

    /// Nothing but one snippet was said.
    fn only_one(&self, marked: &str) -> bool {
        self.used.len() == 1
            && marked
                .trim()
                .trim_end_matches(|c: char| c.is_ascii_punctuation() && c != ']')
                == spoken::placeholder(self.used[0])
    }

    /// `text` with the placeholders filled in, or `None` if the model
    /// dropped one.
    fn fill(&self, text: &str) -> Option<String> {
        spoken::fill_snippets(text, &self.used, &self.saved)
    }
}

impl Outcome {
    /// Before the AI pass: `text` is what to insert without one.
    fn start(job: &TextJob, raw: String, text: String) -> Self {
        Outcome {
            mode: job.mode,
            raw,
            instruction: job.instruction.clone(),
            text,
            fallback: None,
            stt_time: Duration::ZERO,
            llm_time: Duration::ZERO,
            stt_name: None,
            llm_name: None,
            usage: Usage::default(),
            llm_usage: Usage::default(),
            action: None,
            read: None,
            compose: None,
        }
    }

    fn add_llm_usage(&mut self, usage: Usage) {
        self.usage += usage;
        self.llm_usage += usage;
    }

    /// Keeps an instruction that asked for something other than a change
    /// to the selection (see `action`, `read` and `compose`). False if it
    /// didn't.
    fn take_request(&mut self, command: Command) -> bool {
        match command {
            Command::Do(request) => self.action = Some(request),
            Command::Read(what) => self.read = Some(what),
            Command::Compose(asked) => self.compose = Some(asked),
            _ => return false,
        }
        true
    }
}

/// The first `chars` characters of `text`.
fn head(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}
