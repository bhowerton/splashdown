//! Finishing a recording: running the pipeline and putting the text where
//! it goes (the field, or a note), then the history, spending and the
//! correction watch.

use super::*;

impl Controller {
    /// Recording ended: build the job, run the pipeline, deliver the text.
    /// Runs on the session thread and blocks it until done (see
    /// docs/ARCHITECTURE.md, "Threads").
    pub(super) fn finish(&mut self, mut active: Active) {
        if let Some(reply) = active.reply_to.take() {
            self.answer(active, reply);
            return;
        }
        if active.ask {
            self.ask(active, Asked::Question);
            return;
        }
        if active.compose {
            self.ask(active, Asked::Draft);
            return;
        }
        // While a draft is open, what's said changes it ("shorter").
        if !active.capture && crate::compose::is_open(self.ui.app()) {
            self.ask(active, Asked::Change);
            return;
        }
        // Only an error from this dictation's own run counts below.
        self.pipeline_error = None;
        let spoke = active.started.elapsed();
        self.check_screen_at_end(&mut active);
        let job = self.prepare(&mut active);
        if job.is_none()
            && let Some(command) = self.pending_command.take()
        {
            // Return during a command does nothing; hand it back to apps.
            let _ = self.lock().take_send();
            self.run_command(&active, &command);
            return;
        }
        // Kept for "Keep the audio" (the job itself is consumed), in History
        // or with a note.
        let recording = job
            .as_ref()
            .filter(|_| {
                self.engine.as_ref().is_ok_and(|e| e.keep_audio)
                    || (active.capture && self.notes_settings().is_ok_and(|n| n.keep_audio))
            })
            .map(|j| {
                let speech = j.prefix.as_ref().map_or(&j.speech, |p| &p.whole);
                (speech.clone(), j.instruction_speech.clone())
            });
        // On disk until its text is in, so a failure (or the app closing)
        // doesn't lose it.
        // Not a transform: its selection isn't kept, so it couldn't be retried.
        if let Some(j) = job.as_ref().filter(|j| j.mode != Mode::Transform) {
            let speech = j.prefix.as_ref().map_or(&j.speech, |p| &p.whole);
            crate::rescue::start(
                speech,
                j.instruction_speech.as_ref(),
                &crate::rescue::Kept {
                    at: history::Entry::now_ms(),
                    app: active.focus.as_ref().map(|f| f.app.name.clone()),
                    preset: Some(active.preset.id.clone()),
                    reason: String::new(),
                },
            );
        }
        let files = active.project.take().and_then(project_files);
        let screen = active
            .screen
            .take()
            .or_else(|| {
                active
                    .screen_words
                    .take()
                    .and_then(|rx| rx.recv_timeout(Duration::from_millis(300)).ok())
            })
            .unwrap_or_default()
            .words;
        let mut typer = job
            .as_ref()
            .and_then(|_| self.stream_typer(&active, files.clone()));
        let focus = active.focus.as_ref();
        let mut on_text = typer
            .as_mut()
            .zip(focus)
            .map(|(typer, focus)| move |so_far: &str| typer.offer(focus, so_far));
        let on_text = on_text.as_mut().map(|f| f as &mut OnText<'_>);
        // For an instruction that asks to hear the selection read aloud.
        let selected = job.as_ref().and_then(|j| j.selection.clone());
        let outcome = job
            .map(|job| with_file_names(job, files.as_deref()))
            .map(|job| with_screen_words(job, &screen))
            .and_then(|job| self.run_pipeline(job, &active.preset, on_text))
            .map(|(mut outcome, took)| {
                if let Some(project) = &files {
                    outcome.text = splashdown_core::files::tag_files(&outcome.text, &project.files);
                }
                if let Ok(engine) = &self.engine
                    && !engine.replacements.is_empty()
                {
                    outcome.text = engine.replacements.apply(&outcome.text);
                }
                (outcome, took)
            });
        // Return pressed during or after the recording asks for a send.
        // Taking it also hands Return back to apps, however this ended.
        let send = self.lock().take_send();
        match (&outcome, self.pipeline_error.take()) {
            (None, Some(error)) => {
                crate::rescue::failed(&error);
                crate::tray::show_retry(true);
                self.notice(
                    Tone::Warning,
                    &format!(
                        "{} · Kept: Retry last dictation in the menu bar",
                        short(&error)
                    ),
                );
            }
            // The words went in as heard (or an instruction was dropped):
            // kept, so Retry can run the AI on them once it's back.
            (Some((outcome, _)), _)
                if matches!(outcome.fallback, Some(Fallback::ModelError(_))) =>
            {
                if let Some(Fallback::ModelError(error)) = &outcome.fallback {
                    crate::rescue::failed(&format!("AI unavailable: {error}"));
                }
                crate::tray::show_retry(true);
            }
            _ => crate::rescue::done(),
        }
        if let Some((outcome, _)) = &outcome
            && let Some(request) = &outcome.action
        {
            // An instruction for the selection that asked to do something:
            // the selection is the action's material.
            crate::spend::record_outcome(outcome);
            let selection = match &active.selection {
                Selection::Text(text) => Some(text.clone()),
                _ => None,
            };
            self.do_action(&active, request, selection);
            return;
        }
        if let Some((outcome, _)) = &outcome
            && let Some(asked) = &outcome.compose
        {
            // An instruction for the selection that asked for a reply to it.
            crate::spend::record_outcome(outcome);
            self.show_draft(&active, asked.clone(), selected);
            return;
        }
        if let Some((outcome, _)) = &outcome
            && let Some(what) = &outcome.read
        {
            // An instruction for the selection that asked to hear it.
            crate::spend::record_outcome(outcome);
            self.read_aloud(&active, what.clone(), selected, None);
            return;
        }
        if let Some((mut outcome, took)) = outcome {
            crate::spend::record_outcome(&outcome);
            self.pending_audio = recording;
            let typer = typer.filter(StreamTyper::started);
            let unchanged = self.change_by_hooks(&active, &mut outcome);
            self.deliver(active, outcome, took, send, spoke, typer);
            if let Some(why) = unchanged {
                self.notice(Tone::Warning, &format!("Inserted as said · {why}"));
            }
            self.check_budget();
        }
    }

    /// The user's hooks that change this preset's dictations, run before
    /// it's inserted. Why it's inserted as it was, if one failed.
    fn change_by_hooks(&self, active: &Active, outcome: &mut Outcome) -> Option<String> {
        if active.capture || outcome.mode == Mode::Transform {
            return None;
        }
        let settings = self.ui.settings()?;
        if !crate::hooks::changes_text(&settings, &active.preset.id) {
            return None;
        }
        let happened = crate::hooks::Happened::dictation(
            splashdown_config::HookEvent::BeforeInsert,
            &outcome.text,
            active.focus.as_ref().map(|f| f.app.name.as_str()),
            (&active.preset.id, &active.preset.name),
        );
        match crate::hooks::change(self.ui.app(), &settings, happened) {
            Ok(text) => {
                outcome.text = text;
                None
            }
            Err(why) => {
                tracing::warn!("a hook didn't change the dictation: {why}");
                Some(why)
            }
        }
    }

    /// Runs the pipeline (blocking this thread), or shows what went wrong.
    /// The AI's text goes to `on_text` as it streams in.
    pub(super) fn run_pipeline(
        &mut self,
        job: AudioJob,
        preset: &Preset,
        on_text: Option<&mut OnText<'_>>,
    ) -> Option<(Outcome, Duration)> {
        self.pipeline_error = None;
        let engine = self.engine.as_ref().ok()?;
        let started = Instant::now();
        let pipeline = engine.pipeline_for(preset);
        let result = tauri::async_runtime::block_on(pipeline.run_streaming(job, on_text));
        let took = started.elapsed();
        tracing::debug!("processing took {:.2}s", took.as_secs_f32());

        match result {
            Ok(outcome) => Some((outcome, took)),
            Err(PipelineError::NoSpeech) => {
                self.notice(Tone::Info, "Didn't catch that");
                None
            }
            Err(e) => {
                tracing::warn!("pipeline failed: {e}");
                let stt = pipeline.stt_name();
                let llm = pipeline.llm_name();
                let message = errors::pipeline(&e, Some(&stt), llm.as_deref());
                self.notice(Tone::Warning, &short(&message));
                self.pipeline_error = Some(message);
                None
            }
        }
    }

    /// Types the AI's text as it streams in, when that's on and the
    /// dictation goes into a field (not a note).
    pub(super) fn stream_typer(
        &self,
        active: &Active,
        files: Option<Arc<Project>>,
    ) -> Option<StreamTyper> {
        let engine = self.engine.as_ref().ok()?;
        if !engine.stream_typing || active.capture {
            return None;
        }
        // A hook changes the text before it goes in.
        if self
            .ui
            .settings()
            .is_some_and(|s| crate::hooks::changes_text(&s, &active.preset.id))
        {
            return None;
        }
        let focus = active.focus.as_ref().filter(|f| f.is_current())?;
        StreamTyper::new(focus, files, engine.replacements.clone())
    }

    /// After spending: warn once a period nears its limit, and switch to
    /// on-device models when it's reached. At the start of a recording,
    /// also notices a new day or month lifting the limit.
    pub(super) fn check_budget(&mut self) {
        let Ok(engine) = &self.engine else {
            return;
        };
        let now = chrono::Local::now();
        match crate::spend::shared().budget_state(&engine.budget, now) {
            BudgetState::Reached if !engine.over_budget => {
                self.notice(
                    Tone::Warning,
                    "Spending limit reached — using on-device models",
                );
                self.ui.rebuild_engine();
            }
            BudgetState::Near => {
                let period = now.format("%Y-%m-%d").to_string();
                if self.budget_warned.as_ref() != Some(&period) {
                    self.budget_warned = Some(period);
                    self.notice(Tone::Warning, "80% of your spending limit used");
                }
            }
            BudgetState::Fine if engine.over_budget => self.ui.rebuild_engine(),
            _ => {}
        }
    }

    /// Inserts the result into the field it was dictated into, records it,
    /// and presses Return if asked.
    pub(super) fn deliver(
        &mut self,
        active: Active,
        outcome: Outcome,
        took: Duration,
        send: bool,
        spoke: Duration,
        mut typer: Option<StreamTyper>,
    ) {
        let still_focused = active.focus.as_ref().is_none_or(Focus::is_current);
        let target_pid = active.focus.as_ref().map(|f| f.app.pid);

        if outcome.text.trim().is_empty() {
            if let Some((typer, focus)) = typer
                .as_mut()
                .filter(|t| still_focused && !t.stopped())
                .zip(active.focus.as_ref())
            {
                let _ = typer.finish(focus, "");
            }
            self.notice(Tone::Info, "Didn't catch that");
            return;
        }
        // History takes the recording; a note may want it too.
        let note_audio = self
            .pending_audio
            .as_ref()
            .filter(|_| active.capture && self.notes_settings().is_ok_and(|n| n.keep_audio))
            .map(|(speech, _)| splashdown_audio::wav::encode(speech));

        self.record(&outcome, active.focus.as_ref(), &active.preset, took);
        if active.capture {
            self.deliver_capture(&active, &outcome.text, spoke, note_audio.as_deref());
            return;
        }

        let app_id = active.focus.as_ref().map_or("?", |f| f.app.id.as_str());
        // Typing the stream stopped part way (focus moved, or typing
        // failed): what's in the field can't be finished from.
        let broken = typer.as_ref().is_some_and(StreamTyper::stopped);
        if !still_focused || broken {
            tracing::info!("focus moved away from {app_id} before inserting: copied instead");
            let _ = splashdown_platform::copy_to_clipboard(&outcome.text);
            let note = if send { " (not sent)" } else { "" };
            let what = if typer.is_some() {
                "part of it was typed there; all of it is on the clipboard"
            } else {
                "copied to clipboard"
            };
            self.notice(Tone::Warning, &format!("Focus changed — {what}{note}"));
            return;
        }
        let method = match active.preset.insert {
            _ if typer.is_some() => InsertMethod::Type { soft_breaks: true },
            InsertMode::Paste => InsertMethod::Paste,
            InsertMode::Type => InsertMethod::Type {
                soft_breaks: !splashdown_platform::is_terminal(app_id),
            },
        };
        // Right after other text, a space first ("…now?" + "Does…"), judged
        // from the character before the cursor (before anything was
        // streamed in). Not for a transform, which replaces a selection in
        // place.
        let space = match &typer {
            Some(typer) => typer.space(),
            None => {
                outcome.mode != Mode::Transform
                    && active
                        .focus
                        .as_ref()
                        .and_then(Focus::text_and_cursor)
                        .is_some_and(|(field, cursor)| {
                            splashdown_core::spoken::needs_space(
                                field[..cursor].chars().next_back(),
                                outcome.text.chars().next(),
                            )
                        })
            }
        };
        let text = if space {
            format!(" {}", outcome.text)
        } else {
            outcome.text.clone()
        };
        // Chrome now and then ignores a ⌘V: check the field changed, and
        // paste once more if it didn't.
        let check = (method == InsertMethod::Paste && splashdown_platform::is_chromium(app_id))
            .then(|| {
                active
                    .focus
                    .as_ref()
                    .and_then(|f| f.field_text().map(|t| (f, t)))
            })
            .flatten();
        // Nothing to type into (a web page outside any field, a list): the
        // keys go nowhere, so the text stays on the clipboard to paste.
        let takes_text = active.focus.as_ref().and_then(Focus::takes_text);
        let no_field = takes_text == Some(false);
        // Nothing says whether what has focus takes text (an app that
        // reports nothing, a part of one it has no name for). Where its
        // text can be read, that's what shows whether this went in.
        let held = takes_text
            .is_none()
            .then(|| {
                active
                    .focus
                    .as_ref()
                    .and_then(|f| f.field_text().map(|t| (f, t)))
            })
            .flatten();
        let mut lost = false;
        let mut inserted = match &mut typer {
            Some(typer) => match active.focus.as_ref() {
                Some(focus) => typer.finish(focus, &text),
                None => splashdown_platform::insert_text(&text, method),
            },
            None => splashdown_platform::insert_text(&text, method),
        };
        if inserted.is_ok()
            && let Some((focus, before)) = &check
            && !field_changed(focus, before)
        {
            tracing::info!("paste into {app_id} didn't show up; pasting again");
            inserted = splashdown_platform::insert_text(&text, method);
            if inserted.is_ok() {
                let landed = field_changed(focus, before);
                tracing::info!(
                    "second paste into {app_id}: {}",
                    if landed { "landed" } else { "still not seen" }
                );
                lost = !landed;
            }
        }
        if let Err(e) = inserted {
            tracing::warn!("insert into {app_id} failed: {e}");
            let _ = splashdown_platform::copy_to_clipboard(&outcome.text);
            self.notice(Tone::Warning, "Couldn't type it — copied to clipboard");
            return;
        }

        if no_field || lost {
            tracing::info!(
                "{} in {app_id}: copied instead",
                if no_field {
                    "no text field"
                } else {
                    "the paste didn't show up"
                }
            );
            let _ = splashdown_platform::copy_to_clipboard(&outcome.text);
            self.last_text = outcome.text.clone();
            let note = if send { " (not sent)" } else { "" };
            self.notice(
                Tone::Warning,
                &format!("No text box — copied to clipboard{note}"),
            );
            return;
        }

        // It may have gone nowhere, and a paste puts the clipboard back as
        // it was: unless the field is seen to change, the text stays on the
        // clipboard and the overlay says so, not "Inserted".
        let unconfirmed = takes_text.is_none()
            && !held
                .as_ref()
                .is_some_and(|(focus, before)| field_changed(focus, before));
        if unconfirmed {
            let _ = splashdown_platform::copy_to_clipboard(&outcome.text);
        }
        tracing::info!(
            "inserted {} characters into {app_id} ({method:?}{})",
            outcome.text.chars().count(),
            if unconfirmed {
                "; nothing says it went in, so it's on the clipboard too"
            } else {
                ""
            }
        );
        self.last_insert = target_pid.map(|pid| (pid, Instant::now()));
        self.last_text = outcome.text.clone();
        self.last_capture = None;
        if outcome.mode != Mode::Transform {
            crate::hooks::fire(
                self.ui.app(),
                crate::hooks::Happened::dictation(
                    splashdown_config::HookEvent::Dictation,
                    &outcome.text,
                    active.focus.as_ref().map(|f| f.app.name.as_str()),
                    (&active.preset.id, &active.preset.name),
                ),
            );
        }
        if outcome.mode != Mode::Transform && self.engine.as_ref().is_ok_and(|e| e.share_recent) {
            self.recent.push(app_id, &outcome.text, Instant::now());
        }
        if self.engine.as_ref().is_ok_and(|e| e.keep_insights) {
            crate::insights::shared().record(
                &outcome.text,
                spoke,
                active.focus.as_ref().map(|f| f.app.name.as_str()),
                chrono::Local::now(),
            );
        }
        // Only a single paste is one undo step, and a sent message is gone.
        let ai_changed = outcome.llm_name.is_some()
            && outcome.fallback.is_none()
            && outcome.mode != Mode::Transform
            && !outcome.raw.trim().is_empty()
            && outcome.raw.trim() != outcome.text.trim();
        self.last_ai_edit = target_pid
            .filter(|_| ai_changed && method == InsertMethod::Paste && !send)
            .map(|pid| AiEdit {
                pid,
                at: Instant::now(),
                // The undone paste included the space, so this does too.
                said: if space {
                    format!(" {}", outcome.raw)
                } else {
                    outcome.raw.clone()
                },
                in_place: false,
            });
        // Words replaced in the selection: undoing the paste puts the
        // selection back as it was.
        if outcome.replaced.is_some()
            && method == InsertMethod::Paste
            && !send
            && let (Some(pid), Selection::Text(before)) = (target_pid, &active.selection)
        {
            self.last_ai_edit = Some(AiEdit {
                pid,
                at: Instant::now(),
                said: before.clone(),
                in_place: true,
            });
        }
        // Into an app of the user's, not one of Splashdown's own windows
        // (the setup guide's practice box).
        let elsewhere = target_pid.is_some_and(|pid| pid as u32 != std::process::id());
        if self.engine.as_ref().is_ok_and(|e| e.learn_vocabulary)
            && let Some(focus) = active.focus
        {
            match focus.field_text() {
                Some(_) => tracing::info!("watching the field in {} for corrections", focus.app.id),
                None => tracing::info!(
                    "{} doesn't let Splashdown read the field: corrections there can't be learned",
                    focus.app.id
                ),
            }
            self.correction_check = Some(PendingCorrectionCheck {
                focus,
                inserted: outcome.text.clone(),
                due: Instant::now() + CORRECTION_POLL,
                until: Instant::now() + CORRECTION_WATCH,
                last: None,
                changed: Instant::now(),
                again: false,
                draft: None,
            });
        }
        let mut sent = false;
        if send {
            std::thread::sleep(RETURN_AFTER_INSERT);
            sent = splashdown_platform::press(KeyAction::Return).is_ok();
        }
        crate::did::note(
            match outcome.mode {
                Mode::Transform => crate::did::REWROTE,
                Mode::DictateInstruct => crate::did::INSTRUCTED,
                Mode::Dictate | Mode::Raw => crate::did::DICTATED,
            },
            &outcome.text,
        );
        if sent {
            crate::did::note(crate::did::SENT, &outcome.text);
        }
        if elsewhere {
            crate::did::note(crate::did::ELSEWHERE, &outcome.text);
        }
        let mut status = inserted_status(
            took,
            sent,
            outcome.fallback.as_ref(),
            outcome.llm_name.as_deref(),
            active.preset.translate_to.is_some(),
        );
        if outcome.usage.on_device_fallback {
            tracing::warn!("a cloud provider failed; a model on this computer stood in");
            status.push_str(" · done on this computer (cloud didn't answer)");
        } else if self.local_is_slow(&outcome) {
            status = "Inserted · tidying up is slow here: a cloud key is faster".into();
        }
        if let Some(replaced) = &outcome.replaced {
            status = short(&replaced.summary());
        }
        if unconfirmed {
            status = unconfirmed_status(&status);
        }
        self.done(&status);
    }

    /// Whether the built-in model has now been slow several dictations in a
    /// row, on a computer where a cloud key hasn't been suggested yet (and
    /// the user hasn't chosen to keep it anyway). Says so once per run.
    pub(super) fn local_is_slow(&mut self, outcome: &splashdown_core::Outcome) -> bool {
        const IN_A_ROW: u8 = 3;
        let local = outcome
            .llm_name
            .as_deref()
            .is_some_and(|n| n.starts_with("local/"));
        if !local || self.suggested_cloud {
            return false;
        }
        let slow = outcome.llm_time.as_millis() > u128::from(splashdown_config::LOCAL_SLOW_MS);
        self.slow_local = if slow { self.slow_local + 1 } else { 0 };
        if self.slow_local < IN_A_ROW {
            return false;
        }
        let keep = tauri::Manager::try_state::<crate::state::AppState>(self.ui.app())
            .is_some_and(|s| s.settings().ai.slow_local_ok);
        self.suggested_cloud = true;
        tracing::info!(
            "the built-in model took {} ms to tidy up, {IN_A_ROW} dictations in a row{}",
            outcome.llm_time.as_millis(),
            if keep {
                " (kept anyway)"
            } else {
                "; suggesting a cloud key"
            }
        );
        !keep
    }

    pub(super) fn record(
        &mut self,
        outcome: &splashdown_core::Outcome,
        focus: Option<&Focus>,
        preset: &Preset,
        took: Duration,
    ) {
        let Ok(engine) = &self.engine else {
            return;
        };
        let mut entry = history::Entry {
            at: history::Entry::now_ms(),
            app: focus.map(|f| f.app.name.clone()),
            app_id: focus.map(|f| f.app.id.clone()),
            site: focus.and_then(|f| f.app.url.as_deref()).and_then(host),
            mode: outcome.mode,
            raw: outcome.raw.clone(),
            instruction: outcome.instruction.clone(),
            text: outcome.text.clone(),
            note: outcome.fallback.as_ref().map(|f| match f {
                Fallback::Guard(r) => r.describe().to_string(),
                Fallback::ModelError(e) => format!("AI unavailable: {e}"),
            }),
            stt: outcome.stt_name.clone(),
            llm: outcome.llm_name.clone(),
            millis: took.as_millis() as u64,
            cost_usd: (outcome.usage.cost_usd > 0.0).then_some(outcome.usage.cost_usd),
            audio: false,
            preset: Some(preset.name.clone()),
            pinned: false,
        };
        if engine.keep_history {
            // Kept only for a note, the recording stays out of History.
            if let Some((speech, instruction)) =
                self.pending_audio.take().filter(|_| engine.keep_audio)
            {
                entry.audio = self
                    .history
                    .save_audio(entry.at, &speech, instruction.as_ref())
                    .inspect_err(|e| tracing::warn!("couldn't keep the audio: {e}"))
                    .is_ok();
            }
            self.history.add(entry);
        } else {
            self.history.remember(entry);
        }
    }

    /// A captured dictation: added to the note instead of the field.
    /// ⌃⌥Z takes it back while it's recent.
    pub(super) fn deliver_capture(
        &mut self,
        active: &Active,
        text: &str,
        spoke: Duration,
        audio: Option<&[u8]>,
    ) {
        let Ok(notes) = self.notes_settings() else {
            return;
        };
        match crate::notes::capture(&notes, None, text, audio) {
            Ok(captured) => {
                if self.engine.as_ref().is_ok_and(|e| e.keep_insights) {
                    crate::insights::shared().record(
                        text,
                        spoke,
                        active.focus.as_ref().map(|f| f.app.name.as_str()),
                        chrono::Local::now(),
                    );
                }
                let undo = if cfg!(target_os = "macos") {
                    "⌃⌥Z"
                } else {
                    "Ctrl+Alt+Z"
                };
                let message = format!("Added to {} · {undo} undoes it", captured.label);
                self.last_ai_edit = None;
                self.last_capture = Some((captured, Instant::now()));
                self.done(&message);
            }
            Err(e) => {
                tracing::warn!("capture failed: {e}");
                let _ = splashdown_platform::copy_to_clipboard(text);
                self.notice(
                    Tone::Warning,
                    &format!("{} — copied to clipboard", short(&e)),
                );
            }
        }
    }
}

/// How long to wait for a paste to show in a field's text.
pub(super) const PASTE_CHECK: Duration = Duration::from_millis(800);

/// Whether the field's text moved on from `before` within `PASTE_CHECK`.
pub(super) fn field_changed(focus: &Focus, before: &str) -> bool {
    let until = Instant::now() + PASTE_CHECK;
    loop {
        if focus.field_text().is_some_and(|now| now != before) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// `status` (an "Inserted · …" message) for text that was sent to an app
/// with nothing to confirm a field took it. It doesn't say the text went
/// in, since it may not have: only that it's on the clipboard as well.
pub(super) fn unconfirmed_status(status: &str) -> String {
    let rest = status.strip_prefix("Inserted").unwrap_or(status);
    format!("Couldn't check it went in{rest} · it's on the clipboard too")
}

/// The overlay message after a successful insertion. When the words were
/// to be `translated` but went in as said, it says so.
pub(super) fn inserted_status(
    took: Duration,
    sent: bool,
    fallback: Option<&Fallback>,
    llm_name: Option<&str>,
    translated: bool,
) -> String {
    let mut text = format!("Inserted · {:.1} s", took.as_secs_f32());
    if sent {
        text.push_str(" · sent");
    }
    let kept = if translated {
        "not translated"
    } else {
        "raw text"
    };
    match fallback {
        Some(Fallback::Guard(_)) => text.push_str(&format!(" · {kept} (AI output looked wrong)")),
        Some(Fallback::ModelError(e)) => {
            let reason = errors::fallback_reason(e, llm_name);
            text.push_str(&format!(" · {kept} ({reason})"));
        }
        None => {}
    }
    text
}

/// "https://mail.google.com/mail/u/0" → "mail.google.com".
pub(super) fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next()?.trim();
    (!host.is_empty()).then(|| host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use splashdown_core::guard::GuardReason;

    #[test]
    fn inserted_status_mentions_send_and_fallbacks() {
        let took = Duration::from_millis(1240);
        assert_eq!(
            inserted_status(took, false, None, None, false),
            "Inserted · 1.2 s"
        );
        assert_eq!(
            inserted_status(took, true, None, None, false),
            "Inserted · 1.2 s · sent"
        );
        assert_eq!(
            inserted_status(
                took,
                false,
                Some(&Fallback::Guard(GuardReason::AnsweredQuestion)),
                None,
                false
            ),
            "Inserted · 1.2 s · raw text (AI output looked wrong)"
        );
        assert_eq!(
            inserted_status(
                took,
                false,
                Some(&Fallback::ModelError("network error".into())),
                Some("openrouter/x"),
                false
            ),
            "Inserted · 1.2 s · raw text (offline?)"
        );
        assert_eq!(
            inserted_status(
                took,
                false,
                Some(&Fallback::ModelError("network error".into())),
                Some("openrouter/x"),
                true
            ),
            "Inserted · 1.2 s · not translated (offline?)"
        );
    }

    #[test]
    fn an_unconfirmed_insertion_doesn_t_say_it_went_in() {
        let took = Duration::from_millis(1240);
        let status = inserted_status(took, false, None, None, false);
        assert_eq!(
            unconfirmed_status(&status),
            "Couldn't check it went in · 1.2 s · it's on the clipboard too"
        );
        assert_eq!(
            unconfirmed_status(&inserted_status(took, true, None, None, false)),
            "Couldn't check it went in · 1.2 s · sent · it's on the clipboard too"
        );
    }
}
