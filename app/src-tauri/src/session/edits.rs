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
            context: splashdown_core::PromptContext {
                // A transform by name, with nothing said on top, is its own
                // saved instruction: not a spoken command to interpret.
                saved_instruction: !id.is_empty() && extra.is_none(),
                ..engine.context.clone()
            },
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
        let replaced = outcome
            .replaced
            .filter(|_| unchanged.is_none())
            .map(|r| r.summary());
        Ok((
            transform.name.clone(),
            TransformOutcome {
                text,
                unchanged,
                replaced,
            },
        ))
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
    /// opens in the Ask panel, which can read it aloud. With nothing
    /// selected, a hands-free dictation that comes out in that language;
    /// pressed again, it finishes the recording, like a preset's shortcut.
    pub(super) fn translate_shortcut(&mut self) {
        if self.lock().is_recording() {
            let finished = self.lock().finish();
            if let Some(a) = finished {
                self.handle(a);
            }
            return;
        }
        if self.active.is_some() {
            return;
        }
        let Some(language) = self.translate_language() else {
            return;
        };
        // ⌃⌥Z is about this now, not an earlier capture.
        self.last_capture = None;
        let Some(focus) = Focus::capture() else {
            return self.dictate_translated(&language);
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
            return self.dictate_translated(&language);
        };
        self.translate_selection(&focus, &language, text);
    }

    /// The language picked under Translate into. With none chosen yet,
    /// one is asked for there and then, and kept; `None` if it wasn't.
    fn translate_language(&mut self) -> Option<String> {
        let settings = self.ui.settings()?;
        if let Some(language) = settings.general.translate_target() {
            return Some(language.to_string());
        }
        // Not English for someone who speaks it: they'd translate out of it.
        let speaks_english = settings
            .general
            .language
            .as_deref()
            .is_some_and(|l| l.starts_with("en"));
        let choices: Vec<&str> = PICK_LANGUAGES
            .iter()
            .copied()
            .filter(|&l| !(speaks_english && l == "English"))
            .collect();
        const OTHER: &str = "Other…";
        let app = self.ui.app().clone();
        let picked = crate::prompt::choose(
            &app,
            "Translate into…",
            &choices,
            &["Cancel", OTHER],
            PICK_TIMEOUT,
        );
        let other = || {
            crate::tray::open_settings(&app);
        };
        match picked {
            Ok(Some(i)) if i < choices.len() => {
                let language = choices[i].to_string();
                keep_translate_language(&app, &language);
                Some(language)
            }
            // Other…, or another question is on screen: Settings has them all.
            Ok(Some(i)) if i == choices.len() + 1 => {
                other();
                self.notice(
                    Tone::Info,
                    "Choose a language in Settings → Words & language",
                );
                None
            }
            Err(()) => {
                other();
                self.notice(
                    Tone::Info,
                    "Choose languages to translate into: Settings → Words & language",
                );
                None
            }
            // Cancel, or it was left.
            _ => None,
        }
    }

    /// The translate key while holding the dictation key. With text
    /// selected, it's translated at once (nothing needs saying), as with
    /// ⌃⌥T. Otherwise this dictation comes out in the picked language.
    pub(super) fn translate_key(&mut self) {
        let Some(active) = &self.active else {
            return;
        };
        let selected = matches!(&active.selection, Selection::Text(t) if !t.trim().is_empty());
        let Some(language) = self.translate_language() else {
            // No language picked: a dictation carries on as it was; with a
            // selection there's nothing else to do.
            if selected {
                self.drop_recording();
                self.ui.hide();
            }
            return;
        };
        if selected {
            let Some(active) = self.drop_recording() else {
                return;
            };
            self.last_capture = None;
            match (active.focus, active.selection) {
                (Some(focus), Selection::Text(text)) => {
                    self.translate_selection(&focus, &language, text)
                }
                _ => self.notice(Tone::Info, "Select the text to translate"),
            }
            return;
        }
        let targets = self
            .ui
            .settings()
            .map(|s| s.general.translate_targets)
            .unwrap_or_default();
        let Some(active) = &mut self.active else {
            return;
        };
        // Pressed again: the next language ticked under Translate into,
        // which the menu bar then ticks too.
        let language = match (&active.untranslated, &active.preset.translate_to) {
            (Some(_), Some(now)) => match next_language(&targets, now) {
                Some(next) => {
                    keep_translate_language(self.ui.app(), &next);
                    next
                }
                None => return,
            },
            _ => language,
        };
        active.capture = false;
        active.ask = false;
        active.compose = false;
        let before = active
            .untranslated
            .get_or_insert_with(|| active.preset.clone())
            .clone();
        let preset = translating(before, &language);
        tracing::info!("dictating into {language}");
        self.use_preset(preset);
    }

    /// Stops the recording in progress without processing it or saying
    /// so, and hands it back.
    fn drop_recording(&mut self) -> Option<Active> {
        let _ = self.lock().cancel();
        let active = self.active.take();
        self.audio.stop();
        self.refresh_tray();
        active
    }

    /// `text`, selected in `focus`, translated: replaced where it can be
    /// changed, otherwise shown in the Ask panel.
    fn translate_selection(&mut self, focus: &Focus, language: &str, text: String) {
        if focus.is_secure_field() {
            return self.notice(Tone::Warning, "Can't translate in password fields");
        }
        // Only replaced where it's known to be editable: elsewhere a paste
        // would vanish.
        if focus.takes_text() != Some(true) {
            return self.show_translation(focus, language, text);
        }
        let name = format!("Translated into {language}");
        self.ui.show(OverlayState::Working {
            label: format!("Translating into {language}…"),
        });
        let instruction = splashdown_config::translate_instruction(language);
        let out = match self.run_transform("", &text, Some(&instruction)) {
            Ok((_, out)) => out,
            Err(e) => return self.notice(Tone::Warning, &short(&e)),
        };
        if let Some(why) = out.unchanged {
            return self.notice(Tone::Info, &format!("Translate: {why}"));
        }
        self.replace_selection(focus, out.text, &name);
    }

    /// ⌃⌥T with nothing selected: a hands-free dictation with the preset
    /// the app would use, but written in `language`. The dictation key or
    /// ⌃⌥T finishes it; Esc cancels.
    fn dictate_translated(&mut self, language: &str) {
        let has_model = self.engine.as_ref().is_ok_and(|e| {
            e.pipeline_for(&self.match_preset(None))
                .llm_name()
                .is_some()
        });
        if !has_model {
            return self.notice(
                Tone::Warning,
                "Translating needs an AI model: choose one in Settings → AI Models",
            );
        }
        if let Err(e) = self.start_external() {
            tracing::info!("translated dictation: {e}");
            return;
        }
        let Some(active) = &self.active else {
            return;
        };
        let preset = translating(active.preset.clone(), language);
        tracing::info!("dictating into {language}");
        self.use_preset(preset);
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
            if edit.in_place {
                return Ok(());
            }
            std::thread::sleep(UNDO_SETTLE);
            splashdown_platform::insert_text(&edit.said, InsertMethod::Paste)
        });
        match result {
            Ok(()) => {
                self.last_insert = Some((edit.pid, Instant::now()));
                self.last_text = edit.said;
                self.done(if edit.in_place {
                    "Put back as it was"
                } else {
                    "Replaced with what you said"
                });
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

/// The language after `now` among `targets` (the ones ticked under
/// Translate into), back to the first after the last; the first if `now`
/// isn't one of them. `None` when there's nothing else to switch to.
fn next_language(targets: &[String], now: &str) -> Option<String> {
    let targets: Vec<&str> = targets
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();
    let next = match targets.iter().position(|t| *t == now) {
        Some(i) => targets[(i + 1) % targets.len()],
        None => targets.first()?,
    };
    (next != now).then(|| next.to_string())
}

/// Offered when translating with no language chosen yet. The rest are in
/// Settings ("Other…"); names as Settings lists them.
const PICK_LANGUAGES: [&str; 9] = [
    "English",
    "Spanish",
    "French",
    "German",
    "Italian",
    "Portuguese",
    "Chinese (Simplified)",
    "Japanese",
    "Korean",
];

/// How long the language picker waits before it's taken as a no.
const PICK_TIMEOUT: Duration = Duration::from_secs(30);

/// Makes `language` the one to translate into, adding it to the menu bar's
/// list (Settings keeps at most five).
fn keep_translate_language(app: &tauri::AppHandle, language: &str) {
    use tauri::Manager;
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    let result = state.update(app, |s| {
        let targets = &mut s.general.translate_targets;
        if !targets.iter().any(|t| t == language)
            && targets.len() < splashdown_config::MOST_TRANSLATE_TARGETS
        {
            targets.push(language.to_string());
        }
        s.general.translate_to = Some(language.to_string());
    });
    if let Err(e) = result {
        tracing::warn!("couldn't keep the translation language: {e}");
    }
}

/// `preset` writing in `language`: its AI pass on (translating needs it),
/// and named for the overlay "→ Spanish", or "Email → Spanish" for a
/// preset of the user's own.
pub(super) fn translating(mut preset: Preset, language: &str) -> Preset {
    preset.name = match preset_label(&preset) {
        Some(name) => format!("{name} → {language}"),
        None => format!("→ {language}"),
    };
    preset.translate_to = Some(language.to_string());
    preset.ai = true;
    preset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_translate_key_again_moves_to_the_next_language() {
        let targets = ["Spanish".to_string(), "French".into(), " German ".into()];
        assert_eq!(
            next_language(&targets, "Spanish").as_deref(),
            Some("French")
        );
        assert_eq!(
            next_language(&targets, "German").as_deref(),
            Some("Spanish")
        );
        // From the conversation's language, not one of them: the first.
        assert_eq!(next_language(&targets, "Dutch").as_deref(), Some("Spanish"));
        assert_eq!(next_language(&targets[..1], "Spanish"), None);
        assert_eq!(next_language(&[], "Spanish"), None);
    }

    #[test]
    fn a_translated_dictation_keeps_the_preset_and_names_the_language() {
        let default = Preset {
            id: DEFAULT_PRESET_ID.into(),
            name: "Default".into(),
            ai: false,
            ..Preset::default()
        };
        let p = translating(default, "Spanish");
        assert_eq!(p.name, "→ Spanish");
        assert!(p.ai);
        assert_eq!(p.translate_to.as_deref(), Some("Spanish"));
        assert!(p.standing_instruction().unwrap().contains("Spanish"));

        let email = Preset {
            id: "email".into(),
            name: "Email".into(),
            instructions: "Sign off with Ben.".into(),
            translate_to: Some("French".into()),
            ..Preset::default()
        };
        let p = translating(email, "German");
        assert_eq!(p.name, "Email → German");
        assert_eq!(p.translate_to.as_deref(), Some("German"));
        assert_eq!(p.instructions, "Sign off with Ben.");
    }
}
