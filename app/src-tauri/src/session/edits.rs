//! Changing what was already done: redo with another preset, rewrite
//! selected text with a transform or translate it, retry a failed
//! recording, and undo.

use super::*;

impl Controller {
    pub(super) fn redo(&self, r: &Redo) -> Result<String, String> {
        let engine = self.engine.as_ref().map_err(Clone::clone)?;
        let preset = engine
            .presets
            .iter()
            .find(|p| p.id == r.preset_id)
            .ok_or("that preset no longer exists")?;
        let mut context = engine.context.clone();
        context.preset_instructions = preset.ai_instructions();
        context.cleanup = preset.cleanup.unwrap_or_default();
        context.snippets =
            crate::replacements::fill_in(&context.snippets, chrono::Local::now(), None);
        // A translating preset turns plain dictation into an instruction.
        let instruction = r
            .instruction
            .clone()
            .or_else(|| preset.ai.then(|| preset.standing_instruction()).flatten());
        let mode = match (&instruction, preset.ai) {
            (Some(_), _) => Mode::DictateInstruct,
            (None, true) => Mode::Dictate,
            (None, false) => Mode::Raw,
        };
        let pipeline = engine.pipeline_for(preset);
        let outcome = tauri::async_runtime::block_on(pipeline.process_text(TextJob {
            mode,
            content: r.raw.clone(),
            instruction,
            selection: None,
            context,
        }))
        .map_err(|e| e.to_string())?;
        crate::spend::record_outcome(&outcome);
        Ok(engine.replacements.apply(&outcome.text))
    }

    /// Runs transform `id` on `text` with the AI, with `extra` (what the
    /// user asked for) on top; with no `id`, `extra` is the instruction.
    /// Returns its name and the result.
    pub(super) fn run_transform(
        &self,
        id: &str,
        text: &str,
        extra: Option<&str>,
    ) -> Result<(String, TransformOutcome), String> {
        let engine = self.engine.as_ref().map_err(Clone::clone)?;
        let extra = extra.map(str::trim).filter(|e| !e.is_empty());
        // A built-in the user's list doesn't have (it was edited before the
        // built-in existed): the meeting summary still works.
        let builtin;
        let mut transform = match engine.transforms.iter().find(|t| t.id == id) {
            Some(t) => t,
            None if id.is_empty() => {
                let instruction = extra.ok_or("Say how to change it.")?;
                builtin = splashdown_config::Transform {
                    id: String::new(),
                    name: "Rewrite".into(),
                    instruction: format!(
                        "{instruction}\n\nKeep every fact, name and number unless that asks otherwise, and don't add anything it doesn't ask for."
                    ),
                    place: splashdown_config::Place::Replace,
                    shortcut: None,
                };
                &builtin
            }
            None => {
                builtin = splashdown_config::builtin_transforms()
                    .into_iter()
                    .find(|t| t.id == id)
                    .ok_or("That transform no longer exists.")?;
                &builtin
            }
        };
        let asked;
        if let Some(extra) = extra.filter(|_| !id.is_empty()) {
            asked = splashdown_config::Transform {
                instruction: format!(
                    "{}\n\nThe user also asks for this, which comes first where the two disagree: {extra}",
                    transform.instruction
                ),
                ..transform.clone()
            };
            transform = &asked;
        }
        if text.trim().is_empty() {
            return Err("There's no text to change.".into());
        }
        let pipeline = &engine.pipeline;
        let outcome = tauri::async_runtime::block_on(pipeline.process_text(TextJob {
            mode: Mode::Transform,
            content: String::new(),
            instruction: Some(transform.instruction.clone()),
            selection: Some(text.to_string()),
            context: engine.context.clone(),
        }))
        .map_err(|e| errors::instruction(&e, pipeline))?;
        crate::spend::record_outcome(&outcome);
        let mut outcome = outcome;
        outcome.text = splashdown_config::tidy_answer(&transform.id, &outcome.text);
        let unchanged = match &outcome.fallback {
            Some(Fallback::Guard(splashdown_core::guard::GuardReason::Empty)) => {
                Some("nothing came back, so nothing changed".to_string())
            }
            Some(Fallback::Guard(reason)) => {
                Some(format!("left as it was ({})", reason.describe()))
            }
            Some(Fallback::ModelError(e)) => Some(format!("the AI didn't answer ({e})")),
            None if splashdown_config::is_nothing_to_do(&outcome.text) => {
                Some("found nothing to do".to_string())
            }
            None if outcome.text.trim() == text.trim() => Some("nothing to change".to_string()),
            None => None,
        };
        let text = if unchanged.is_some() {
            text.to_string()
        } else {
            outcome.text
        };
        Ok((transform.name.clone(), TransformOutcome { text, unchanged }))
    }

    /// A transform's shortcut in another app: the selection, or else the
    /// last dictation just before the cursor, is replaced by the result.
    pub(super) fn transform_shortcut(&mut self, id: &str) {
        if self.active.is_some() {
            return;
        }
        // ⌃⌥Z is about this now, not an earlier capture.
        self.last_capture = None;
        let Some(focus) = Focus::capture() else {
            return self.notice(Tone::Info, "Select some text first");
        };
        if focus.is_secure_field() {
            return self.notice(Tone::Warning, "Can't change text in password fields");
        }
        let mut selected = match focus.selection() {
            Selection::Text(s) => Some(s),
            Selection::Unknown => splashdown_platform::copy_selection(),
            Selection::Empty => None,
        }
        .filter(|s| !s.trim().is_empty());
        if selected.is_none() {
            let recent = self
                .last_insert
                .is_some_and(|(pid, at)| pid == focus.app.pid && at.elapsed() < UNDO_WINDOW);
            if recent
                && let Some((text, cursor)) = focus.text_and_cursor()
                && let Some((start, end)) = last_dictation_range(&text, cursor, &self.last_text)
                && focus.select(&text, start, end)
            {
                selected = Some(self.last_text.clone());
            }
        }
        let Some(text) = selected else {
            return self.notice(Tone::Info, "Select some text first");
        };
        let name = self
            .engine
            .as_ref()
            .ok()
            .and_then(|e| e.transforms.iter().find(|t| t.id == id))
            .map_or_else(|| "Transform".to_string(), |t| t.name.clone());
        self.ui.show(OverlayState::Working {
            label: format!("{name}…"),
        });
        let out = match self.run_transform(id, &text, None) {
            Ok((_, out)) => out,
            Err(e) => return self.notice(Tone::Warning, &short(&e)),
        };
        if let Some(why) = out.unchanged {
            return self.notice(Tone::Info, &format!("{name}: {why}"));
        }
        self.replace_selection(&focus, out.text, &name);
    }

    /// Pastes `text` over the selection in `focus`, unless focus moved on
    /// meanwhile (then it's left on the clipboard).
    fn replace_selection(&mut self, focus: &Focus, text: String, name: &str) {
        if !focus.is_current() {
            let _ = splashdown_platform::copy_to_clipboard(&text);
            return self.notice(Tone::Warning, "Focus changed — copied to clipboard");
        }
        let result = splashdown_platform::insert_text(&text, InsertMethod::Paste);
        if result.is_ok() {
            self.last_insert = Some((focus.app.pid, Instant::now()));
            self.last_text = text;
        }
        self.finish_command(result, name);
    }

    /// ⌃⌥T: the selection, translated into the language picked in the menu
    /// bar. Where it can be changed, the translation replaces it; where it
    /// can't (a web page, a message received, a PDF), the translation
    /// opens in the Ask panel, which can read it aloud.
    pub(super) fn translate_shortcut(&mut self) {
        if self.active.is_some() {
            return;
        }
        let language = self
            .ui
            .settings()
            .and_then(|s| s.general.translate_target().map(str::to_string));
        let Some(language) = language else {
            crate::tray::open_settings(self.ui.app());
            return self.notice(
                Tone::Info,
                "Choose languages to translate into: Settings → Words & language",
            );
        };
        // ⌃⌥Z is about this now, not an earlier capture.
        self.last_capture = None;
        let Some(focus) = Focus::capture() else {
            return self.notice(Tone::Info, "Select the text to translate");
        };
        if focus.is_secure_field() {
            return self.notice(Tone::Warning, "Can't translate in password fields");
        }
        let selected = match focus.selection() {
            Selection::Text(s) => Some(s),
            Selection::Unknown => splashdown_platform::copy_selection(),
            Selection::Empty => None,
        }
        .filter(|s| !s.trim().is_empty());
        let Some(text) = selected else {
            return self.notice(Tone::Info, "Select the text to translate");
        };
        // Only replaced where it's known to be editable: elsewhere a paste
        // would vanish.
        if focus.takes_text() != Some(true) {
            return self.show_translation(&focus, &language, text);
        }
        let name = format!("Translated into {language}");
        self.ui.show(OverlayState::Working {
            label: format!("Translating into {language}…"),
        });
        let instruction = splashdown_config::translate_instruction(&language);
        let out = match self.run_transform("", &text, Some(&instruction)) {
            Ok((_, out)) => out,
            Err(e) => return self.notice(Tone::Warning, &short(&e)),
        };
        if let Some(why) = out.unchanged {
            return self.notice(Tone::Info, &format!("Translate: {why}"));
        }
        self.replace_selection(&focus, out.text, &name);
    }

    /// A translation of text that can't be replaced, in the Ask panel.
    fn show_translation(&mut self, focus: &Focus, language: &str, text: String) {
        let Ok(engine) = &self.engine else {
            return self.notice(Tone::Warning, "Splashdown isn't ready: see Settings");
        };
        let preset = self.match_preset(Some(focus));
        let Some(model) = engine.pipeline_for(&preset).answer_model() else {
            return self.notice(
                Tone::Warning,
                "Translating needs an AI model: choose one in Settings → AI Models",
            );
        };
        let context = splashdown_core::ask::AskContext {
            selection: Some(text),
            app_name: engine.share_app_name.then(|| focus.app.name.clone()),
            user_name: engine.context.user_name.clone(),
            today: splashdown_core::prompt::today(chrono::Local::now().date_naive()),
            ..Default::default()
        };
        tracing::info!("translating a selection into {language} in the Ask panel");
        crate::ask::ask(
            self.ui.app(),
            crate::ask::Question {
                question: format!("Translate this into {language}. Write only the translation."),
                context,
                model,
                target: Some((focus.app.pid, focus.app.name.clone())),
                speak: false,
            },
        );
        self.ui.hide();
    }

    /// "Retry last dictation": runs the kept recording again with the
    /// preset it was dictated with, and puts the text on the clipboard (the
    /// field it was meant for may be gone).
    pub(super) fn retry_last(&mut self) {
        if self.active.is_some() {
            return;
        }
        let (Some(kept), Some((speech, instruction))) =
            (crate::rescue::kept(), crate::rescue::audio())
        else {
            crate::tray::show_retry(false);
            return self.notice(Tone::Info, "No dictation to retry");
        };
        let Ok(engine) = &self.engine else {
            return self.notice(Tone::Warning, "Splashdown isn't ready: see Settings");
        };
        let preset = kept
            .preset
            .as_ref()
            .and_then(|id| engine.presets.iter().find(|p| &p.id == id))
            .cloned()
            .unwrap_or_else(|| self.match_preset(None));
        let mode = match (&instruction, preset.ai) {
            (Some(_), _) => Mode::DictateInstruct,
            (None, true) => Mode::Dictate,
            (None, false) => Mode::Raw,
        };
        let app_name = kept.app.clone().filter(|_| engine.share_app_name);
        let job = AudioJob {
            mode,
            speech,
            instruction_speech: instruction,
            selection: None,
            context: PromptContext {
                snippets: crate::replacements::fill_in(
                    &engine.context.snippets,
                    chrono::Local::now(),
                    None,
                ),
                ..prompt_context(&engine.context, app_name, &preset, None)
            },
            hints: engine.hints_for(&preset),
            standing_instruction: preset.standing_instruction(),
            prefix: None,
        };
        self.ui.show(OverlayState::Working {
            label: "Retrying…".into(),
        });
        let Some((mut outcome, took)) = self.run_pipeline(job, &preset, None) else {
            // Still kept; the notice says why.
            self.pipeline_error = None;
            return;
        };
        if let Ok(engine) = &self.engine
            && !engine.replacements.is_empty()
        {
            outcome.text = engine.replacements.apply(&outcome.text);
        }
        crate::spend::record_outcome(&outcome);
        self.record(&outcome, None, &preset, took);
        if let Err(e) = splashdown_platform::copy_to_clipboard(&outcome.text) {
            tracing::warn!("couldn't copy the retried dictation: {e}");
            return self.notice(Tone::Warning, "Retried, but couldn't copy it: see History");
        }
        crate::rescue::forget();
        crate::tray::show_retry(false);
        self.done("Retried · copied to the clipboard");
        self.check_budget();
    }

    /// Replaces the last AI-edited insertion with what was said: undo the
    /// paste, then paste the transcript. Only right after it, in the same
    /// app, like other edits of "that". A dictation into a note is taken
    /// out of the note instead.
    pub(super) fn undo_ai_edit(&mut self) {
        if let Some((captured, at)) = self.last_capture.take()
            && at.elapsed() < UNDO_WINDOW
        {
            return match crate::notes::undo_capture(&captured) {
                Ok(true) => self.done(&format!("Removed from {}", captured.label)),
                Ok(false) => self.notice(
                    Tone::Info,
                    &format!("{} has changed since; nothing removed", captured.label),
                ),
                Err(e) => self.notice(Tone::Warning, &short(&e)),
            };
        }
        let Some(edit) = self.last_ai_edit.take() else {
            self.notice(Tone::Info, "No AI edit to undo");
            return;
        };
        let here = Focus::capture().map(|f| f.app.pid);
        if here != Some(edit.pid) || edit.at.elapsed() > UNDO_WINDOW {
            self.notice(Tone::Info, "No AI edit to undo here");
            return;
        }
        let result = splashdown_platform::press(KeyAction::Undo).and_then(|()| {
            std::thread::sleep(UNDO_SETTLE);
            splashdown_platform::insert_text(&edit.said, InsertMethod::Paste)
        });
        match result {
            Ok(()) => {
                self.last_insert = Some((edit.pid, Instant::now()));
                self.last_text = edit.said;
                self.done("Replaced with what you said");
            }
            Err(e) => self.notice(Tone::Warning, &short(&e.to_string())),
        }
    }

    /// Undoes our last insertion (a spoken command) if it was recent and into the
    /// app that has focus now. One paste is a single undo step.
    pub(super) fn undo_last(&mut self, target_pid: Option<i32>, still_focused: bool) {
        let recent = self
            .last_insert
            .is_some_and(|(pid, at)| Some(pid) == target_pid && at.elapsed() < UNDO_WINDOW);
        if !recent || !still_focused {
            self.notice(Tone::Info, "Nothing to undo");
            return;
        }
        match splashdown_platform::press(KeyAction::Undo) {
            Ok(()) => {
                self.last_insert = None;
                self.done("Undone");
            }
            Err(e) => self.notice(Tone::Warning, &short(&e.to_string())),
        }
    }
}
