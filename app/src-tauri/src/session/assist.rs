//! Asking and writing: a question for Ask, a draft for Compose (straight
//! into the field, or in its panel), and reading text aloud.

use super::*;

impl Controller {
    /// `finish` for a question, a draft or a change to one: transcribes it
    /// as said and hands it to Ask or Compose, which work in the
    /// background.
    pub(super) fn ask(&mut self, mut active: Active, kind: Asked) {
        let job = self.prepare(&mut active);
        let _ = self.lock().take_send();
        let Some(mut job) = job else {
            // The instruction key came before anything was said: what came
            // after it is the question.
            if let Some(clip) = self.pending_command.take() {
                self.ask_from(&active, &clip, kind);
            }
            return;
        };
        // Only the words: the question goes to the AI as it was said.
        let selection = job.selection.take();
        job.mode = Mode::Raw;
        if let Some(instruction) = job.instruction_speech.take() {
            job.speech = concat(job.speech, instruction);
        }
        let Some((outcome, _)) = self.run_pipeline(job, &active.preset, None) else {
            return;
        };
        crate::spend::record_outcome(&outcome);
        let question = outcome.raw.trim().to_string();
        if question.is_empty() {
            return self.notice(Tone::Info, "Didn't catch that");
        }
        self.asked(&active, kind, question, selection);
        self.check_budget();
    }

    /// Where something asked goes.
    pub(super) fn asked(
        &mut self,
        active: &Active,
        kind: Asked,
        said: String,
        selection: Option<String>,
    ) {
        match kind {
            Asked::Question => self.show_answer(active, said, selection, false),
            Asked::Draft => self.show_draft(active, said, selection),
            Asked::Change => {
                if let Err(message) = crate::compose::change(self.ui.app(), &said) {
                    return self.notice(Tone::Info, &message);
                }
                self.ui.hide();
            }
        }
    }

    /// Writes what was asked (`asked`) for `active`'s app: straight into the
    /// field, or in the Compose panel when that's on, there's a selection
    /// (`selection`, the message it answers) or there's no field.
    pub(super) fn show_draft(&mut self, active: &Active, asked: String, selection: Option<String>) {
        let Ok(engine) = &self.engine else {
            return;
        };
        let Some(model) = engine.pipeline_for(&active.preset).answer_model() else {
            return self.notice(
                Tone::Warning,
                "Compose needs an AI model: choose one in Settings → AI Models",
            );
        };
        let notes = self
            .notes_settings()
            .map(|n| crate::notes::excerpts_for(&n, &asked))
            .unwrap_or_default();
        // The field's text is what's being answered when nothing is
        // selected (a reply with the thread quoted under it).
        let field_text = active
            .focus
            .as_ref()
            .and_then(Focus::field_text)
            .or_else(|| active.field_text.clone())
            .map(|t| head_chars(&t, READ_CHARS));
        // Nothing selected: the conversation around the field, read from
        // the window (an email thread, a chat), unless turned off.
        let window_text = active
            .focus
            .as_ref()
            .filter(|_| selection.is_none() && engine.compose_reads_window)
            .and_then(|f| {
                f.surrounding_text(
                    splashdown_core::compose::WINDOW_MIN_CHARS,
                    splashdown_core::compose::WINDOW_MAX_CHARS,
                )
            });
        match &window_text {
            Some(text) => tracing::info!(
                "compose read {} characters from the window",
                text.chars().count()
            ),
            None if selection.is_none() => tracing::info!("compose read nothing from the window"),
            None => {}
        }
        let tone = active.preset.tone.map(|t| match t {
            splashdown_config::Tone::Formal => "formal",
            splashdown_config::Tone::Casual => "casual",
            splashdown_config::Tone::VeryCasual => "very-casual",
        });
        let context = splashdown_core::compose::ComposeContext {
            replying_to: selection.map(|s| head_chars(&s, READ_CHARS)),
            field_text,
            window_text,
            clipboard: active
                .clipboard
                .clone()
                .filter(|_| active.preset.clipboard_context),
            notes,
            app_name: active
                .focus
                .as_ref()
                .filter(|_| engine.share_app_name)
                .map(|f| f.app.name.clone()),
            user_name: engine.context.user_name.clone(),
            today: splashdown_core::prompt::today(chrono::Local::now().date_naive()),
            tone: tone.and_then(splashdown_core::compose::tone),
            match_style: engine.compose_style,
            // Learned from how the user edited earlier drafts here.
            style: active
                .focus
                .as_ref()
                .filter(|_| engine.compose_style)
                .and_then(|f| {
                    crate::style::profile(splashdown_config::style_kind(&AppContext {
                        app_id: &f.app.id,
                        app_name: &f.app.name,
                        title: f.app.window_title.as_deref(),
                        url: f.app.url.as_deref(),
                    }))
                }),
        };
        tracing::info!("composing, with {} matching note(s)", context.notes.len());
        let target = active.focus.as_ref().map(|f| f.app.pid);
        // Written straight into the field, unless the panel is on, or text
        // is selected (pasting would replace it) or there's no field.
        if !engine.compose_panel && context.replying_to.is_none() && target.is_some() {
            self.ui.show(OverlayState::Working {
                label: "Writing…".into(),
            });
            let tx = self.tx.clone();
            tauri::async_runtime::spawn(async move {
                let (context, removed) = context.defanged();
                if removed {
                    tracing::info!("compose left instructions to an AI out of the material");
                }
                let request = splashdown_core::compose::request(&asked, &context);
                let result = model.complete(&request).await;
                if let Ok(completion) = &result {
                    crate::spend::shared()
                        .record(&[(model.name(), completion.usage)], chrono::Local::now());
                }
                let result = result
                    .map(|completion| splashdown_core::compose::clean(&completion.text))
                    .map_err(|e| {
                        errors::pipeline(
                            &splashdown_core::PipelineError::Llm(e),
                            None,
                            Some(&model.name()),
                        )
                    });
                let _ = tx.send(Msg::Composed(result, target));
            });
            return;
        }
        crate::compose::start(
            self.ui.app(),
            crate::compose::Draft {
                asked,
                context,
                model,
                target: active
                    .focus
                    .as_ref()
                    .map(|f| (f.app.pid, f.app.name.clone())),
            },
        );
        self.ui.hide();
    }

    /// A draft written for the field: pasted there if its app is still in
    /// front, otherwise left on the clipboard.
    pub(super) fn composed(&mut self, result: Result<String, String>, pid: Option<i32>) {
        let text = match result {
            Ok(text) if !text.trim().is_empty() => text,
            Ok(_) => return self.notice(Tone::Warning, "The draft came back empty"),
            Err(e) => return self.notice(Tone::Warning, &short(&e)),
        };
        let here = splashdown_platform::frontmost_app().map(|(pid, ..)| pid);
        if pid.is_some() && here == pid {
            match splashdown_platform::insert_text(&text, InsertMethod::Paste) {
                Ok(()) => {
                    tracing::info!(
                        "composed {} characters into the field",
                        text.chars().count()
                    );
                    self.last_insert = pid.map(|pid| (pid, Instant::now()));
                    self.watch_draft(&text, pid);
                    self.last_text = text;
                    crate::did::note(crate::did::COMPOSED, "");
                    return self.done("Written");
                }
                Err(e) => tracing::warn!("couldn't paste the draft: {e}"),
            }
        }
        let _ = splashdown_platform::copy_to_clipboard(&text);
        crate::did::note(crate::did::COMPOSED, "");
        self.notice(Tone::Info, "Draft copied · paste it where it goes");
    }

    /// Watches the field a draft was just pasted into: names the user fixes
    /// are learned, and what they send instead of the draft is kept for
    /// their writing style in this kind of app.
    fn watch_draft(&mut self, draft: &str, pid: Option<i32>) {
        let Ok(engine) = &self.engine else {
            return;
        };
        if !engine.learn_style && !engine.learn_vocabulary {
            return;
        }
        let Some(focus) = Focus::capture().filter(|f| Some(f.app.pid) == pid) else {
            return;
        };
        let kind = splashdown_config::style_kind(&AppContext {
            app_id: &focus.app.id,
            app_name: &focus.app.name,
            title: focus.app.window_title.as_deref(),
            url: focus.app.url.as_deref(),
        });
        let now = Instant::now();
        self.correction_check = Some(PendingCorrectionCheck {
            focus,
            inserted: draft.to_string(),
            due: now + CORRECTION_POLL,
            until: now + DRAFT_WATCH,
            last: None,
            changed: now,
            // No second look: by then the draft has been sent or left.
            again: true,
            draft: engine.learn_style.then_some(kind),
        });
    }

    /// A question, draft or change recorded on its own (after the
    /// instruction key).
    pub(super) fn ask_from(&mut self, active: &Active, clip: &AudioClip, kind: Asked) {
        let Ok(engine) = &self.engine else {
            return self.ui.hide();
        };
        let pipeline = engine.pipeline_for(&active.preset);
        let hints = engine.hints_for(&active.preset);
        let heard = tauri::async_runtime::block_on(pipeline.transcribe_only(clip, &hints));
        let stt = pipeline.stt_name();
        match heard {
            Ok((question, usage)) if !question.trim().is_empty() => {
                crate::spend::shared().record(&[(stt, usage)], chrono::Local::now());
                self.asked(active, kind, question.trim().to_string(), None);
            }
            Ok(_) => self.notice(Tone::Info, "Didn't catch that"),
            Err(e) => {
                let message = errors::pipeline(&e, Some(&stt), None);
                self.notice(Tone::Warning, &short(&message));
            }
        }
    }

    /// Opens the Ask panel on `question`, asked from `active`'s app.
    /// `speak`: read the answer aloud as it's written ("read me a summary of
    /// this"), with `selection` as what it's about.
    pub(super) fn show_answer(
        &mut self,
        active: &Active,
        question: String,
        selection: Option<String>,
        speak: bool,
    ) {
        if splashdown_core::action::asks_what_to_say(&question) {
            return self.what_to_say();
        }
        let Ok(engine) = &self.engine else {
            return;
        };
        let Some(model) = engine.pipeline_for(&active.preset).answer_model() else {
            return self.notice(
                Tone::Warning,
                "Asking needs an AI model: choose one in Settings → AI Models",
            );
        };
        // Notes that match "summarize this" are beside the point.
        let notes = self
            .notes_settings()
            .ok()
            .filter(|_| !speak)
            .map(|n| crate::notes::excerpts_for(&n, &question))
            .unwrap_or_default();
        let context = splashdown_core::ask::AskContext {
            selection,
            field_text: active
                .field_text
                .clone()
                .filter(|_| active.preset.field_context),
            clipboard: active
                .clipboard
                .clone()
                .filter(|_| active.preset.clipboard_context),
            notes,
            app_name: active
                .focus
                .as_ref()
                .filter(|_| engine.share_app_name)
                .map(|f| f.app.name.clone()),
            user_name: engine.context.user_name.clone(),
            today: splashdown_core::prompt::today(chrono::Local::now().date_naive()),
            earlier: Vec::new(),
            cite_numbers: false,
            spoken: speak,
        };
        tracing::info!("asking, with {} matching note(s)", context.notes.len());
        crate::did::note(crate::did::ASKED, &question);
        crate::ask::ask(
            self.ui.app(),
            crate::ask::Question {
                question,
                context,
                model,
                target: active
                    .focus
                    .as_ref()
                    .map(|f| (f.app.pid, f.app.name.clone())),
                speak,
            },
        );
        // Reading shows its own pill.
        if !speak {
            self.ui.hide();
        }
    }

    /// "Read this to me" (`what` is "") or "read me a summary of this"
    /// (`what` is what to answer and read out): about the selection, or
    /// else the field's text, or else `last`, the last dictation.
    pub(super) fn read_aloud(
        &mut self,
        active: &Active,
        what: String,
        selection: Option<String>,
        last: Option<String>,
    ) {
        let text = selection
            .or_else(|| active.focus.as_ref().and_then(Focus::field_text))
            .or(last)
            .filter(|t| !t.trim().is_empty());
        let Some(text) = text else {
            return self.notice(Tone::Info, "Nothing to read · select the text first");
        };
        let text = head_chars(&text, READ_CHARS);
        if what.trim().is_empty() {
            tracing::info!("reading {} characters aloud", text.chars().count());
            crate::did::note(crate::did::READ, "");
            if let Err(message) = crate::reading::read(&text, "Reading aloud") {
                self.notice(Tone::Info, &message);
            }
            return;
        }
        crate::did::note(crate::did::READ, "");
        self.show_answer(active, what, Some(text), true);
    }
}

/// What a spoken request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Asked {
    /// A question for the Ask panel.
    Question,
    /// What to write, for a new draft.
    Draft,
    /// A change to the draft open in the Compose panel.
    Change,
}
