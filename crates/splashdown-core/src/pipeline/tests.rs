use super::*;
use crate::provider::{Completion, OnText, ProviderResult, TextRequest, Transcript};
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
            words: Vec::new(),
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
fn dictation_loses_dashes_nobody_said() {
    let (p, _) = pipeline(
        &["it is great but the commute is long"],
        Some(Ok("It is great \u{2014} but the commute is long.")),
    );
    let out = block_on(p.run(audio_job(Mode::Dictate, false))).unwrap();
    assert_eq!(out.text, "It is great, but the commute is long.");
    // Said out loud: kept.
    let (p, _) = pipeline(&["Ana dash Marco"], Some(Ok("Ana \u{2014} Marco")));
    let out = block_on(p.run(audio_job(Mode::Dictate, false))).unwrap();
    assert_eq!(out.text, "Ana \u{2014} Marco");
    // An instruction may ask for one: not touched.
    let (p, _) = pipeline(
        &["running late", "add a dash"],
        Some(Ok("Running late \u{2014} sorry.")),
    );
    let out = block_on(p.run(audio_job(Mode::DictateInstruct, true))).unwrap();
    assert_eq!(out.text, "Running late \u{2014} sorry.");
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
fn a_translation_that_did_what_it_said_keeps_the_words_as_said() {
    let poem = "El mar es un poema eterno, sus olas cantan historias sin fin.\n\
                Sus aguas reflejan cielos de azul, sus corrientes llevan sueños.\n\
                Las estrellas brillan en su vastedad, sus olas acarician la orilla.";
    let (p, _) = pipeline(&["write me a poem about the sea"], Some(Ok(poem)));
    let mut job = audio_job(Mode::Dictate, false);
    job.standing_instruction = Some("Translate this into Spanish.".into());
    let out = block_on(p.run(job)).unwrap();
    assert_eq!(out.text, "write me a poem about the sea");
    assert_eq!(
        out.fallback,
        Some(Fallback::Guard(GuardReason::LengthOutOfRange))
    );
}

#[test]
fn a_prefix_is_joined_to_the_rest() {
    let (p, llm) = pipeline(&["and then the second part."], Some(Ok("Done.")));
    let mut job = audio_job(Mode::Dictate, false);
    job.prefix = Some(Prefix {
        transcript: Box::pin(async { Ok(("The first part,".to_string(), Usage::priced(0.002))) }),
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
        false,
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
    let out =
        block_on(p.command("write a short no", None, &PromptContext::default(), false)).unwrap();
    assert_eq!(
        out.command,
        crate::command::Command::Write("No, thanks.".into())
    );
    // No AI model: commands can't work.
    let (p, _) = pipeline(&[], None);
    assert!(matches!(
        block_on(p.command("send it", None, &PromptContext::default(), false)),
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
fn an_instruction_for_a_selection_can_be_an_action() {
    let job = || {
        let mut job = audio_job(Mode::Transform, false);
        job.selection = Some("Q3 numbers are in.".into());
        job
    };
    let reply = r#"{"action": "do", "text": "email this to Sam"}"#;
    // With actions on, the words as said come back for planning.
    let (p, _) = pipeline(&["email this to Sam"], Some(Ok(reply)));
    let out = block_on(p.with_actions(true).run(job())).unwrap();
    assert_eq!(out.action.as_deref(), Some("email this to Sam"));
    assert_eq!(out.text, "");
    // Off, it's a transform as always: the model's reply is the text.
    let (p, llm) = pipeline(&["email this to Sam"], Some(Ok(reply)));
    let out = block_on(p.run(job())).unwrap();
    assert_eq!(out.action, None);
    assert_eq!(out.text, reply);
    let asked = llm.unwrap().1.lock().unwrap().clone().unwrap();
    assert!(!asked.system.contains("- do:"), "no command interpreter");
}

#[test]
fn an_instruction_for_a_selection_can_ask_to_hear_it() {
    let mut job = audio_job(Mode::Transform, false);
    job.selection = Some("Q3 numbers are in.".into());
    // Without voice actions too.
    let reply = r#"{"action": "read", "text": "Summarize this"}"#;
    let (p, _) = pipeline(&["read me a summary"], Some(Ok(reply)));
    let out = block_on(p.run(job)).unwrap();
    assert_eq!(out.read.as_deref(), Some("Summarize this"));
    assert_eq!(out.action, None);
    assert_eq!(out.text, "");
}

/// Answers the command check with `check`, and the rewrite with
/// `rewrite`, or never when that's `None`.
struct CheckingLlm {
    check: &'static str,
    rewrite: Option<&'static str>,
}

#[async_trait]
impl TextModel for CheckingLlm {
    fn name(&self) -> String {
        "checking-llm".into()
    }
    async fn complete(&self, req: &TextRequest) -> ProviderResult<Completion> {
        let text = if req.system.contains("You interpret a spoken command") {
            self.check
        } else {
            match self.rewrite {
                Some(text) => text,
                None => std::future::pending().await,
            }
        };
        Ok(Completion {
            text: text.to_string(),
            usage: Usage::priced(0.0002),
        })
    }
    fn supports_json_schema(&self) -> bool {
        false
    }
    fn context_window(&self) -> usize {
        8192
    }
}

fn checked(check: &'static str, rewrite: Option<&'static str>, selection: &str) -> Outcome {
    let p = Pipeline::new(
        Arc::new(FakeStt(vec![], Mutex::new(0))),
        Some(Arc::new(CheckingLlm { check, rewrite })),
    );
    block_on(p.process_text(TextJob {
        mode: Mode::Transform,
        content: String::new(),
        instruction: Some("change every Tuesday to Thursday".into()),
        selection: Some(selection.into()),
        context: PromptContext::default(),
    }))
    .unwrap()
}

#[test]
fn a_replacement_for_a_selection_is_made_exactly() {
    let check = r#"{"action": "find_replace", "text": "", "edits": [{"find": "tuesday", "replace": "thursday"}]}"#;
    // The rewrite never answers: it isn't waited for.
    let out = checked(check, None, "Tuesday at 3. TUESDAY! Not Tuesdays.");
    assert_eq!(out.text, "Thursday at 3. THURSDAY! Not Tuesdays.");
    let replaced = out.replaced.unwrap();
    assert_eq!(replaced.counts, vec![2]);
    assert_eq!(replaced.summary(), "Changed 2 · tuesday → thursday");
    assert_eq!(out.fallback, None);

    // None of it in the selection: the rewrite is used after all.
    let out = checked(check, Some("Wednesday it is."), "Weds it is.");
    assert_eq!(out.text, "Wednesday it is.");
    assert_eq!(out.replaced, None);

    // Not a replacement, or one the check got wrong: the rewrite.
    let out = checked(
        r#"{"action": "find_replace", "text": "", "edits": "Tuesday"}"#,
        Some("Thursday at 3."),
        "Tuesday at 3.",
    );
    assert_eq!(out.text, "Thursday at 3.");
    assert_eq!(out.replaced, None);
}

#[test]
fn a_saved_transform_s_instruction_is_never_a_command() {
    // The meeting summary, run by name on notes that talk of sending and
    // booking: nothing was said, so there's no command to find in it. Its
    // result used to be thrown away when the interpreter found one.
    let job = |saved| TextJob {
        mode: Mode::Transform,
        content: String::new(),
        instruction: Some("Summarize this meeting transcript.".into()),
        selection: Some("[3:09] You: \"I'll send Dana the numbers.\"".into()),
        context: PromptContext {
            saved_instruction: saved,
            ..PromptContext::default()
        },
    };
    let reply = r#"{"action": "read", "text": "Summarize this"}"#;
    let (p, _) = pipeline(&[], Some(Ok(reply)));
    let out = block_on(p.with_actions(true).process_text(job(true))).unwrap();
    assert_eq!((out.read, out.action), (None, None));
    assert_eq!(out.text, reply);
    // Said by the user, the same words are still listened to.
    let (p, _) = pipeline(&[], Some(Ok(reply)));
    let out = block_on(p.with_actions(true).process_text(job(false))).unwrap();
    assert_eq!(out.read.as_deref(), Some("Summarize this"));
    assert_eq!(out.text, "");
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

#[test]
fn an_earlier_dictation_the_model_repeats_is_left_out() {
    let said = "can we point dev to production so we can do some testing";
    let reply = "Tell me why we need that backfill.\n\nCan we point dev to production so we can do some testing?";
    let mut job = text_job(Mode::Dictate, said);
    job.context.earlier = vec!["Tell me why we need that backfill.".into()];
    let (out, seen) = streamed(reply, job);
    // Never typed as it streamed…
    assert!(seen.iter().all(|s| !s.contains("backfill")), "{seen:?}");
    // …and not in what goes in at the end.
    assert_eq!(
        out.text,
        "Can we point dev to production so we can do some testing?"
    );
    assert!(out.fallback.is_none());
}
