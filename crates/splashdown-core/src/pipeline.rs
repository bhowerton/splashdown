//! Turns a recording (or text) into the final text to insert:
//! speech-to-text → AI pass → sanity guard.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::guard::{self, GuardReason, Verdict};
use crate::prompt::{self, PromptContext, PromptTask};
use crate::provider::{
    OnText, ProviderError, SpeechToText, SttHints, TextModel, TextRequest, Usage,
};
use crate::spoken;
use crate::{AudioClip, Mode};

const TEMPERATURE: f32 = 0.2;

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
}

pub struct Pipeline {
    stt: Arc<dyn SpeechToText>,
    llm: Option<Arc<dyn TextModel>>,
    /// Handles instructions and transforms instead of `llm`, when set: a
    /// stronger cloud model behind a local one that does plain cleanup.
    instruction_llm: Option<Arc<dyn TextModel>>,
}

impl Pipeline {
    pub fn new(stt: Arc<dyn SpeechToText>, llm: Option<Arc<dyn TextModel>>) -> Self {
        Self {
            stt,
            llm,
            instruction_llm: None,
        }
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

    pub async fn run(&self, job: AudioJob) -> Result<Outcome, PipelineError> {
        self.run_streaming(job, None).await
    }

    /// `run`, passing the AI's reply so far to `on_text` as it arrives, for
    /// typing it while the rest is written. Only plain dictation and
    /// dictation with an instruction stream, and only when no snippet was
    /// said (the model writes placeholders for those). The outcome is still
    /// what to insert in the end: the guard may reject what was streamed.
    pub async fn run_streaming(
        &self,
        job: AudioJob,
        on_text: Option<&mut OnText<'_>>,
    ) -> Result<Outcome, PipelineError> {
        let started = Instant::now();
        let speech = self.transcribe_speech(&job.speech, job.prefix, &job.hints);
        let ((speech, stt_usage), instruction) = match &job.instruction_speech {
            Some(clip) => {
                let (a, b) =
                    futures_util::future::join(speech, self.transcribe(clip, &job.hints)).await;
                let (instruction, usage) = b?;
                let (speech, speech_usage) = a?;
                (
                    (speech, speech_usage + usage),
                    Some(instruction).filter(|s| !s.is_empty()),
                )
            }
            None => (speech.await?, None),
        };
        let stt_time = started.elapsed();

        if speech.is_empty() {
            return Err(PipelineError::NoSpeech);
        }

        let (content, instruction) = match job.mode {
            Mode::Transform => (String::new(), Some(speech)),
            // A marker with nothing said after it is just a dictation.
            Mode::DictateInstruct if instruction.is_none() => (speech, None),
            _ => (speech, instruction),
        };
        let mode = match (job.mode, &instruction) {
            (Mode::DictateInstruct, None) => Mode::Dictate,
            (m, _) => m,
        };

        let (mode, instruction) = match (mode, instruction, job.standing_instruction) {
            (Mode::Dictate, None, Some(standing)) => (Mode::DictateInstruct, Some(standing)),
            (mode, instruction, _) => (mode, instruction),
        };
        let mut outcome = self
            .process_text_streaming(
                TextJob {
                    mode,
                    content,
                    instruction,
                    selection: job.selection,
                    context: job.context,
                },
                on_text,
            )
            .await?;
        outcome.stt_time = stt_time;
        outcome.stt_name = Some(self.stt.name());
        outcome.usage += stt_usage;
        Ok(outcome)
    }

    /// Interprets a spoken command (see `crate::command`). `last` is the last
    /// dictation, if it's still just before the cursor. Uses the model for
    /// instructions when one is set.
    pub async fn command(
        &self,
        spoken: &str,
        last: Option<&str>,
        ctx: &PromptContext,
    ) -> Result<CommandOutcome, PipelineError> {
        let llm = self
            .instruction_llm
            .as_ref()
            .or(self.llm.as_ref())
            .ok_or(PipelineError::NoModel(Mode::DictateInstruct))?;
        let mut request = crate::command::request(spoken, last, ctx);
        if !llm.supports_json_schema() {
            request.json_schema = None;
        }
        let started = Instant::now();
        let completion = llm.complete(&request).await.map_err(PipelineError::Llm)?;
        let command = match crate::command::parse(&completion.text) {
            // Nothing to replace: it can only mean new text.
            crate::command::Command::ReplaceLast(text) if last.is_none() => {
                crate::command::Command::Write(text)
            }
            crate::command::Command::Ask(question) if question.is_empty() => {
                crate::command::Command::Ask(spoken.to_string())
            }
            other => other,
        };
        Ok(CommandOutcome {
            command,
            llm_name: llm.name(),
            usage: completion.usage,
            llm_time: started.elapsed(),
        })
    }

    /// Transcribes a recording on its own (a spoken command).
    pub async fn transcribe_only(
        &self,
        clip: &AudioClip,
        hints: &SttHints,
    ) -> Result<(String, Usage), PipelineError> {
        self.transcribe(clip, hints).await
    }

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

        // Snippets: the model sees placeholders, never the saved text.
        let snippets = std::mem::take(&mut job.context.snippets);
        let (marked, used) = if job.mode == Mode::Transform {
            (job.content.clone(), Vec::new())
        } else {
            spoken::mark_snippets(&job.content, &snippets)
        };
        let expanded =
            spoken::fill_snippets(&marked, &used, &snippets).unwrap_or_else(|| job.content.clone());
        job.content = marked;
        job.context.has_placeholders = !used.is_empty();

        let mut outcome = Outcome {
            mode: job.mode,
            raw: raw.clone(),
            instruction: job.instruction.clone(),
            text: expanded.clone(),
            fallback: None,
            stt_time: Duration::ZERO,
            llm_time: Duration::ZERO,
            stt_name: None,
            llm_name: None,
            usage: Usage::default(),
            llm_usage: Usage::default(),
        };

        // Raw mode, or nothing but a snippet was said: no AI needed.
        let only_snippet = used.len() == 1
            && job
                .content
                .trim()
                .trim_end_matches(|c: char| c.is_ascii_punctuation() && c != ']')
                == spoken::placeholder(used[0]);
        if job.mode == Mode::Raw || (only_snippet && job.mode == Mode::Dictate) {
            return Ok(outcome);
        }
        let follows_instruction = job.mode == Mode::Transform
            || (job.mode == Mode::DictateInstruct
                && job.instruction.as_deref().is_some_and(|i| !i.is_empty()));
        let llm = match &self.instruction_llm {
            Some(cloud) if follows_instruction && self.llm.is_some() => Some(cloud),
            _ => self.llm.as_ref(),
        };
        let Some(llm) = llm else {
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
        let started = Instant::now();
        let streams = matches!(job.mode, Mode::Dictate | Mode::DictateInstruct) && used.is_empty();
        let result = match on_text.filter(|_| streams) {
            Some(on_text) => {
                // Stops passing the reply on once the guard would reject it.
                let mut passing = true;
                let mut forward = |so_far: &str| {
                    if !passing {
                        return;
                    }
                    match guard::partial(task.mode(), &source, so_far) {
                        guard::Partial::Show => on_text(so_far),
                        guard::Partial::Wait => {}
                        guard::Partial::Stop => passing = false,
                    }
                };
                llm.complete_streaming(&request, &mut forward).await
            }
            None => llm.complete(&request).await,
        };
        outcome.llm_time = started.elapsed();

        let output = match result {
            Ok(completion) => {
                outcome.usage += completion.usage;
                outcome.llm_usage = completion.usage;
                completion.text
            }
            // Keep the user's words rather than losing them; instructions
            // and transforms can't be done without the model.
            Err(err) if matches!(job.mode, Mode::Dictate | Mode::DictateInstruct) => {
                // A reply that stopped part way still cost something.
                if let ProviderError::Interrupted { usage, .. } = &err {
                    outcome.usage += *usage;
                    outcome.llm_usage = *usage;
                }
                outcome.fallback = Some(Fallback::ModelError(err.to_string()));
                return Ok(outcome);
            }
            Err(err) => return Err(PipelineError::Llm(err)),
        };

        match guard::check(task.mode(), &source, &output) {
            Verdict::Accept(text) => match spoken::fill_snippets(&text, &used, &snippets) {
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

    /// The speech, joined to its prefix if there is one. A failed prefix
    /// means transcribing the whole recording again.
    async fn transcribe_speech(
        &self,
        clip: &AudioClip,
        prefix: Option<Prefix>,
        hints: &SttHints,
    ) -> Result<(String, Usage), PipelineError> {
        let Some(prefix) = prefix else {
            return self.transcribe(clip, hints).await;
        };
        let (head, tail) =
            futures_util::future::join(prefix.transcript, self.transcribe(clip, hints)).await;
        match (head, tail) {
            (Ok((head, head_usage)), Ok((tail, tail_usage))) => Ok((
                format!("{head} {tail}").trim().to_string(),
                head_usage + tail_usage,
            )),
            // (The app logs the prefix's error where it ran.)
            _ => self.transcribe(&prefix.whole, hints).await,
        }
    }

    async fn transcribe(
        &self,
        clip: &AudioClip,
        hints: &SttHints,
    ) -> Result<(String, Usage), PipelineError> {
        // A prefix can leave nothing after it.
        if clip.samples.is_empty() {
            return Ok((String::new(), Usage::FREE));
        }
        let transcript = self
            .stt
            .transcribe(clip, hints)
            .await
            .map_err(PipelineError::Stt)?;
        Ok((transcript.text.trim().to_string(), transcript.usage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Completion, ProviderResult, Transcript};
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct FakeStt(Vec<&'static str>, Mutex<usize>);

    #[async_trait]
    impl SpeechToText for FakeStt {
        fn name(&self) -> String {
            "fake-stt".into()
        }
        async fn transcribe(&self, _: &AudioClip, _: &SttHints) -> ProviderResult<Transcript> {
            let mut i = self.1.lock().unwrap();
            let text = self.0[*i].to_string();
            *i += 1;
            Ok(Transcript {
                text,
                usage: Usage::priced(0.001),
            })
        }
    }

    struct FakeLlm(Result<&'static str, ()>, Mutex<Option<TextRequest>>);

    #[async_trait]
    impl TextModel for FakeLlm {
        fn name(&self) -> String {
            "fake-llm".into()
        }
        async fn complete(&self, req: &TextRequest) -> ProviderResult<Completion> {
            *self.1.lock().unwrap() = Some(req.clone());
            self.0
                .map(|text| Completion {
                    text: text.to_string(),
                    usage: Usage::priced(0.0002),
                })
                .map_err(|_| ProviderError::Network("down".into()))
        }
        fn supports_json_schema(&self) -> bool {
            false
        }
        fn context_window(&self) -> usize {
            8192
        }
    }

    fn pipeline(
        stt: &[&'static str],
        llm: Option<Result<&'static str, ()>>,
    ) -> (Pipeline, Option<Arc<FakeLlm>>) {
        let llm = llm.map(|r| Arc::new(FakeLlm(r, Mutex::new(None))));
        let p = Pipeline::new(
            Arc::new(FakeStt(stt.to_vec(), Mutex::new(0))),
            llm.clone().map(|l| l as Arc<dyn TextModel>),
        );
        (p, llm)
    }

    fn audio_job(mode: Mode, instruction: bool) -> AudioJob {
        let clip = AudioClip::new(vec![0.0; 16], 16_000);
        AudioJob {
            mode,
            speech: clip.clone(),
            instruction_speech: instruction.then_some(clip),
            selection: None,
            context: PromptContext::default(),
            hints: SttHints::default(),
            standing_instruction: None,
            prefix: None,
        }
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        futures_executor::block_on(f)
    }

    #[test]
    fn dictate_cleans_up() {
        let (p, _) = pipeline(&["um hello"], Some(Ok("Hello.")));
        let out = block_on(p.run(audio_job(Mode::Dictate, false))).unwrap();
        assert_eq!(out.raw, "um hello");
        assert_eq!(out.text, "Hello.");
        assert_eq!(out.fallback, None);
    }

    #[test]
    fn raw_skips_model() {
        let (p, llm) = pipeline(&["um hello"], Some(Ok("Hello.")));
        let out = block_on(p.run(audio_job(Mode::Raw, false))).unwrap();
        assert_eq!(out.text, "um hello");
        assert!(llm.unwrap().1.lock().unwrap().is_none());
    }

    #[test]
    fn empty_speech_is_no_speech() {
        let (p, _) = pipeline(&["  "], Some(Ok("x")));
        let err = block_on(p.run(audio_job(Mode::Dictate, false))).unwrap_err();
        assert!(matches!(err, PipelineError::NoSpeech));
    }

    #[test]
    fn instruct_sends_both_parts() {
        let (p, llm) = pipeline(
            &["running late", "make it an email"],
            Some(Ok("Hi,\n\nRunning late.")),
        );
        let out = block_on(p.run(audio_job(Mode::DictateInstruct, true))).unwrap();
        assert_eq!(out.instruction.as_deref(), Some("make it an email"));
        // Two transcriptions (content and instruction) plus one AI call.
        assert!(
            (out.usage.cost_usd - 0.0022).abs() < 1e-9,
            "{:?}",
            out.usage
        );
        assert!(!out.usage.unpriced);
        let req = llm.unwrap().1.lock().unwrap().clone().unwrap();
        assert!(req.user.contains("<instruction>\nmake it an email"));
    }

    #[test]
    fn instructions_go_to_the_instruction_model() {
        let (p, local) = pipeline(
            &["running late", "make it an email", "um see you soon"],
            Some(Ok("See you soon.")),
        );
        let cloud = Arc::new(FakeLlm(Ok("Hi,\n\nRunning late."), Mutex::new(None)));
        let p = p.with_instruction_model(Some(cloud.clone()));
        let local = local.unwrap();

        block_on(p.run(audio_job(Mode::DictateInstruct, true))).unwrap();
        assert!(cloud.1.lock().unwrap().is_some(), "instruction → cloud");
        assert!(local.1.lock().unwrap().is_none());

        let out = block_on(p.run(audio_job(Mode::Dictate, false))).unwrap();
        assert!(local.1.lock().unwrap().is_some(), "plain dictation → local");
        assert_eq!(out.llm_name.as_deref(), Some("fake-llm"));
    }

    #[test]
    fn standing_instruction_applies_to_plain_dictation() {
        let (p, llm) = pipeline(&["see you at noon"], Some(Ok("Nos vemos al mediodía.")));
        let mut job = audio_job(Mode::Dictate, false);
        job.standing_instruction = Some("Translate into Spanish.".into());
        let out = block_on(p.run(job)).unwrap();
        assert_eq!(out.mode, Mode::DictateInstruct);
        let req = llm.unwrap().1.lock().unwrap().clone().unwrap();
        assert!(req.user.contains("<instruction>\nTranslate into Spanish."));
    }

    #[test]
    fn a_prefix_is_joined_to_the_rest() {
        let (p, llm) = pipeline(&["and then the second part."], Some(Ok("Done.")));
        let mut job = audio_job(Mode::Dictate, false);
        job.prefix = Some(Prefix {
            transcript: Box::pin(async {
                Ok(("The first part,".to_string(), Usage::priced(0.002)))
            }),
            whole: job.speech.clone(),
        });
        let out = block_on(p.run(job)).unwrap();
        assert_eq!(out.raw, "The first part, and then the second part.");
        assert!(
            (out.usage.cost_usd - 0.0032).abs() < 1e-12,
            "both parts counted"
        );
        let req = llm.unwrap().1.lock().unwrap().clone().unwrap();
        assert!(
            req.user
                .contains("The first part, and then the second part.")
        );

        // Nothing said after the prefix.
        let (p, _) = pipeline(&[], None);
        let mut job = audio_job(Mode::Raw, false);
        job.speech = AudioClip::new(vec![], 16_000);
        job.prefix = Some(Prefix {
            transcript: Box::pin(async { Ok(("Only this.".to_string(), Usage::FREE)) }),
            whole: AudioClip::new(vec![0.0; 16], 16_000),
        });
        assert_eq!(block_on(p.run(job)).unwrap().text, "Only this.");
    }

    #[test]
    fn a_failed_prefix_transcribes_the_whole_recording() {
        // The tail is transcribed first ("tail"), then the whole ("all of it").
        let (p, _) = pipeline(&["tail", "All of it."], None);
        let mut job = audio_job(Mode::Raw, false);
        job.prefix = Some(Prefix {
            transcript: Box::pin(async { Err("timed out".to_string()) }),
            whole: job.speech.clone(),
        });
        assert_eq!(block_on(p.run(job)).unwrap().text, "All of it.");
    }

    #[test]
    fn dictated_words_are_never_commands() {
        // Commands only come through `command`; said in a dictation, these
        // are just words for the AI to clean up.
        let (p, llm) = pipeline(&["Send it."], Some(Ok("Send it.")));
        let out = block_on(p.run(audio_job(Mode::Dictate, false))).unwrap();
        assert_eq!(out.text, "Send it.");
        assert!(llm.unwrap().1.lock().unwrap().is_some(), "the AI saw it");
    }

    #[test]
    fn commands_are_interpreted_by_the_model() {
        let (p, llm) = pipeline(
            &[],
            Some(Ok(
                r#"{"action":"replace_last","text":"Can Someone Review My PR"}"#,
            )),
        );
        let out = block_on(p.command(
            "capitalize that",
            Some("can someone review my pr"),
            &PromptContext::default(),
        ))
        .unwrap();
        assert_eq!(
            out.command,
            crate::command::Command::ReplaceLast("Can Someone Review My PR".into())
        );
        let req = llm.unwrap().1.lock().unwrap().clone().unwrap();
        assert!(req.user.contains("<last>can someone review my pr</last>"));
        // With nothing dictated before, a replacement can only be new text.
        let (p, _) = pipeline(
            &[],
            Some(Ok(r#"{"action":"replace_last","text":"No, thanks."}"#)),
        );
        let out = block_on(p.command("write a short no", None, &PromptContext::default())).unwrap();
        assert_eq!(
            out.command,
            crate::command::Command::Write("No, thanks.".into())
        );
        // No AI model: commands can't work.
        let (p, _) = pipeline(&[], None);
        assert!(matches!(
            block_on(p.command("send it", None, &PromptContext::default())),
            Err(PipelineError::NoModel(_))
        ));
    }

    #[test]
    fn snippets_survive_the_ai_pass() {
        let snippets = vec![crate::spoken::Snippet {
            trigger: "my email".into(),
            text: "ben@example.com".into(),
        }];
        let job = |content: &str| TextJob {
            mode: Mode::Dictate,
            content: content.into(),
            instruction: None,
            selection: None,
            context: PromptContext {
                snippets: snippets.clone(),
                ..PromptContext::default()
            },
        };

        let (p, llm) = pipeline(&[], Some(Ok("Write to [[s1]] today.")));
        let out = block_on(p.process_text(job("um write to my email today"))).unwrap();
        assert_eq!(out.text, "Write to ben@example.com today.");
        let req = llm.unwrap().1.lock().unwrap().clone().unwrap();
        assert!(req.user.contains("[[s1]]") && !req.user.contains("ben@example.com"));
        assert!(req.system.contains("[[s1]]"));

        let (p, llm) = pipeline(&[], Some(Ok("x")));
        let out = block_on(p.process_text(job("My email."))).unwrap();
        assert_eq!(out.text, "ben@example.com.");
        assert!(
            llm.unwrap().1.lock().unwrap().is_none(),
            "snippet alone skips the AI"
        );

        let (p, _) = pipeline(&[], Some(Ok("Write to me today.")));
        let out = block_on(p.process_text(job("write to my email today"))).unwrap();
        assert_eq!(
            out.fallback,
            Some(Fallback::Guard(GuardReason::LostSnippet))
        );
        assert_eq!(out.text, "write to ben@example.com today");
    }

    /// Streams its reply in two pieces.
    struct StreamingLlm(&'static str);

    #[async_trait]
    impl TextModel for StreamingLlm {
        fn name(&self) -> String {
            "streaming-llm".into()
        }
        async fn complete(&self, _: &TextRequest) -> ProviderResult<Completion> {
            Ok(Completion {
                text: self.0.to_string(),
                usage: Usage::FREE,
            })
        }
        async fn complete_streaming(
            &self,
            req: &TextRequest,
            on_text: &mut OnText<'_>,
        ) -> ProviderResult<Completion> {
            let half = self
                .0
                .char_indices()
                .nth(self.0.chars().count() / 2)
                .unwrap()
                .0;
            on_text(&self.0[..half]);
            on_text(self.0);
            self.complete(req).await
        }
        fn supports_json_schema(&self) -> bool {
            false
        }
        fn context_window(&self) -> usize {
            8192
        }
    }

    fn streamed(llm: &'static str, job: TextJob) -> (Outcome, Vec<String>) {
        let p = Pipeline::new(
            Arc::new(FakeStt(vec![], Mutex::new(0))),
            Some(Arc::new(StreamingLlm(llm))),
        );
        let mut seen = Vec::new();
        let mut on_text = |s: &str| seen.push(s.to_string());
        let out = block_on(p.process_text_streaming(job, Some(&mut on_text))).unwrap();
        (out, seen)
    }

    fn text_job(mode: Mode, content: &str) -> TextJob {
        TextJob {
            mode,
            content: content.into(),
            instruction: (mode != Mode::Dictate).then(|| "shorter".into()),
            selection: (mode == Mode::Transform).then(|| "a long text".into()),
            context: PromptContext::default(),
        }
    }

    #[test]
    fn dictation_streams_until_the_guard_would_reject_it() {
        let (out, seen) = streamed("Hello there.", text_job(Mode::Dictate, "um hello there"));
        assert_eq!(seen, ["Hello ", "Hello there."]);
        assert_eq!(out.text, "Hello there.");

        // Too long for a cleanup: the stream stops, and the guard rejects it.
        let poem = "Roses are red, violets are blue, sugar is sweet, and so are you.";
        let (out, seen) = streamed(poem, text_job(Mode::Dictate, "a poem"));
        assert!(seen.len() <= 1, "{seen:?}");
        assert!(out.fallback.is_some());
        assert_eq!(out.text, "a poem");

        // A transform replaces a selection: nothing to stream.
        let (_, seen) = streamed("Short.", text_job(Mode::Transform, ""));
        assert!(seen.is_empty());

        // A snippet: the model writes placeholders, never shown as they are.
        let mut job = text_job(Mode::Dictate, "write to my email today");
        job.context.snippets = vec![crate::spoken::Snippet {
            trigger: "my email".into(),
            text: "ben@example.com".into(),
        }];
        let (out, seen) = streamed("Write to [[s1]] today.", job);
        assert!(seen.is_empty());
        assert_eq!(out.text, "Write to ben@example.com today.");
    }

    #[test]
    fn empty_instruction_falls_back_to_dictate() {
        let (p, _) = pipeline(&["running late", ""], Some(Ok("Running late.")));
        let out = block_on(p.run(audio_job(Mode::DictateInstruct, true))).unwrap();
        assert_eq!(out.mode, Mode::Dictate);
    }

    #[test]
    fn model_error_keeps_raw_dictation() {
        let (p, _) = pipeline(&["um hello"], Some(Err(())));
        let out = block_on(p.run(audio_job(Mode::Dictate, false))).unwrap();
        assert_eq!(out.text, "um hello");
        assert!(matches!(out.fallback, Some(Fallback::ModelError(_))));
    }

    #[test]
    fn transform_needs_selection_and_model() {
        let (p, _) = pipeline(&["shorter"], Some(Ok("x")));
        let err = block_on(p.run(audio_job(Mode::Transform, false))).unwrap_err();
        assert!(matches!(err, PipelineError::NoSelection));

        let (p, _) = pipeline(&["shorter"], None);
        let mut job = audio_job(Mode::Transform, false);
        job.selection = Some("long text".into());
        let err = block_on(p.run(job)).unwrap_err();
        assert!(matches!(err, PipelineError::NoModel(Mode::Transform)));
    }

    #[test]
    fn guarded_transform_keeps_selection() {
        let (p, _) = pipeline(&["make it shorter"], Some(Ok("  ")));
        let mut job = audio_job(Mode::Transform, false);
        job.selection = Some("original text".into());
        let out = block_on(p.run(job)).unwrap();
        assert_eq!(out.text, "original text");
        assert_eq!(out.fallback, Some(Fallback::Guard(GuardReason::Empty)));
    }
}
