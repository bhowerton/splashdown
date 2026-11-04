//! Spoken commands (the instruction key before anything is said): edits
//! of the last dictation, and voice actions planned from a fixed catalog.

use super::*;

impl Controller {
    /// A spoken command (the instruction key with nothing said before
    /// it): transcribe it, let the AI say what it means, and do that.
    pub(super) fn run_command(&mut self, active: &Active, clip: &AudioClip) {
        self.last_capture = None;
        let Ok(engine) = &self.engine else {
            return;
        };
        let pipeline = engine.pipeline_for(&active.preset);
        let hints = engine.hints_for(&active.preset);
        let spoken = tauri::async_runtime::block_on(pipeline.transcribe_only(clip, &hints));
        let (spoken, stt_usage) = match spoken {
            Ok((text, usage)) if !text.trim().is_empty() => (text, usage),
            Ok(_) => {
                tracing::info!(
                    "the speech model heard no words in a {:.1} s command",
                    clip.duration().as_secs_f32()
                );
                return self.notice(Tone::Info, "Didn't catch that");
            }
            Err(e) => {
                let message = errors::pipeline(&e, Some(&pipeline.stt_name()), None);
                return self.notice(Tone::Warning, &short(&message));
            }
        };
        // Answered with no AI, so it works with voice actions off too.
        if splashdown_core::action::asks_what_to_say(&spoken) {
            return self.what_to_say();
        }
        let target_pid = active.focus.as_ref().map(|f| f.app.pid);
        // "That" is the last dictation, while it's recent and in this app.
        let recent = self
            .last_insert
            .is_some_and(|(pid, at)| Some(pid) == target_pid && at.elapsed() < UNDO_WINDOW);
        let last = (recent && !self.last_text.trim().is_empty()).then(|| self.last_text.clone());
        let context = prompt_context(&engine.context, None, &active.preset, None);
        let result = tauri::async_runtime::block_on(pipeline.command(
            &spoken,
            last.as_deref(),
            &context,
            engine.actions.enabled,
        ));
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(e) => {
                let message = errors::instruction(&e, pipeline);
                return self.notice(Tone::Warning, &short(&message));
            }
        };
        tracing::info!("command → {:?} ({:?})", outcome.command, outcome.llm_time);
        crate::spend::shared().record(
            &[
                (pipeline.stt_name(), stt_usage),
                (outcome.llm_name.clone(), outcome.usage),
            ],
            chrono::Local::now(),
        );
        let still_focused = active.focus.as_ref().is_none_or(Focus::is_current);
        if let Command::Ask(question) = outcome.command {
            self.show_answer(active, question, None, false);
        } else if let Command::Read(what) = outcome.command {
            self.read_aloud(active, what, None, last);
        } else if let Command::Compose(asked) = outcome.command {
            self.show_draft(active, asked, None);
        } else if let Command::Write(_) = outcome.command {
            // Written by Compose from what was said, so it sees the
            // conversation and never makes things up.
            self.show_draft(active, spoken, None);
        } else if let Command::Do(request) = outcome.command {
            self.do_action(active, &request, None);
        } else if !still_focused {
            self.notice(Tone::Warning, "Focus changed — didn't do that");
        } else {
            self.carry_out(outcome.command, active.focus.as_ref(), target_pid);
        }
        self.check_budget();
    }

    /// Does what a spoken command asked, in the field it was spoken in.
    pub(super) fn carry_out(
        &mut self,
        command: Command,
        focus: Option<&Focus>,
        target_pid: Option<i32>,
    ) {
        let paste = |text: &str| splashdown_platform::insert_text(text, InsertMethod::Paste);
        match command {
            Command::Unclear => {
                return self.notice(
                    Tone::Info,
                    "Didn't understand that command · say “what can I say?”",
                );
            }
            // Answered in the Ask panel instead (`run_command`).
            Command::Ask(_) => return,
            // Read aloud by `read_aloud` instead (`run_command`).
            Command::Read(_) => return,
            // Drafted by `show_draft` instead (`run_command`).
            Command::Compose(_) | Command::Write(_) => return,
            // Carried out by `do_action` instead (`run_command`).
            Command::Do(_) => return,
            Command::PressEnter => {
                return self.finish_command(splashdown_platform::press(KeyAction::Return), "Sent");
            }
            Command::UndoLast => {
                // The last voice action, if it came after the last insertion.
                let inserted = self.last_insert.map(|(_, at)| at);
                if let Some((undo, _)) = self.last_action.take_if(|(_, at)| {
                    at.elapsed() < UNDO_WINDOW && inserted.is_none_or(|i| i < *at)
                }) {
                    let tx = self.tx.clone();
                    std::thread::spawn(move || {
                        let result = undo
                            .undo()
                            .map(|text| crate::voice_actions::Done { text, undo: None });
                        let _ = tx.send(Msg::ActionDone(result));
                    });
                    return;
                }
                return self.undo_last(target_pid, true);
            }
            Command::DeleteLastWord => {
                return self
                    .finish_command(splashdown_platform::press(KeyAction::DeleteWord), "Deleted");
            }
            _ => {}
        }
        // The rest select something first: the last dictation, or the
        // sentence before the cursor.
        let Some((focus, (text, cursor))) = focus.and_then(|f| Some((f, f.text_and_cursor()?)))
        else {
            // Can't read the field: replacing "that" can still undo the
            // paste and paste again.
            if let Command::ReplaceLast(new) = &command
                && self
                    .last_insert
                    .is_some_and(|(pid, at)| Some(pid) == target_pid && at.elapsed() < UNDO_WINDOW)
            {
                let result = splashdown_platform::press(KeyAction::Undo).and_then(|()| {
                    std::thread::sleep(UNDO_SETTLE);
                    paste(new)
                });
                if result.is_ok() {
                    self.last_text = new.clone();
                    self.last_insert = target_pid.map(|pid| (pid, Instant::now()));
                }
                return self.finish_command(result, "Done");
            }
            return self.notice(
                Tone::Warning,
                "This app doesn't let Splashdown edit its text",
            );
        };
        let range = if command == Command::DeleteLastSentence {
            Some((
                splashdown_core::spoken::last_sentence_start(&text, cursor),
                cursor,
            ))
        } else {
            last_dictation_range(&text, cursor, &self.last_text)
        };
        let Some((start, end)) = range.filter(|(s, e)| s < e) else {
            return self.notice(Tone::Info, "Nothing to change here");
        };
        if !focus.select(&text, start, end) {
            return self.notice(Tone::Warning, "This app doesn't let Splashdown select text");
        }
        let result = match &command {
            Command::DeleteLastSentence => splashdown_platform::press(KeyAction::Backspace),
            Command::SelectLast => Ok(()),
            Command::ReplaceLast(new) => paste(new),
            // Format, then put the cursor back after it.
            Command::BoldLast | Command::ItalicLast => {
                let key = if command == Command::BoldLast {
                    KeyAction::Bold
                } else {
                    KeyAction::Italic
                };
                splashdown_platform::press(key)
                    .and_then(|()| splashdown_platform::press(KeyAction::Right))
            }
            _ => unreachable!("handled above"),
        };
        if let (Ok(()), Command::ReplaceLast(new)) = (&result, &command) {
            // The new version is "that" now, and can be changed again.
            self.last_text = new.clone();
            self.last_insert = target_pid.map(|pid| (pid, Instant::now()));
        }
        let done = match command {
            Command::DeleteLastSentence => "Deleted",
            Command::SelectLast => "Selected",
            _ => "Done",
        };
        self.finish_command(result, done);
    }

    /// A command for a voice action: plans it into one action from the
    /// catalog (`splashdown_core::action`) and carries it out off this
    /// thread. `selection` is selected text the action may use (an email's
    /// body), never orders.
    pub(super) fn do_action(&mut self, active: &Active, request: &str, selection: Option<String>) {
        if splashdown_core::action::asks_what_to_say(request) {
            return self.what_to_say();
        }
        let Ok(engine) = &self.engine else {
            return;
        };
        if active.focus.as_ref().is_some_and(Focus::is_secure_field)
            || splashdown_platform::secure_input_active()
        {
            return self.notice(Tone::Warning, "Not while a password is being typed");
        }
        let enabled = engine.actions.enabled_ids();
        let ctx = splashdown_core::action::PlanContext {
            selection,
            shortcuts: if enabled.contains(&"run_shortcut") {
                crate::voice_actions::shortcuts()
            } else {
                Vec::new()
            },
            presets: engine.presets.iter().map(|p| p.name.clone()).collect(),
            mine: crate::my_actions::for_planner(&engine.actions),
            ..splashdown_core::action::PlanContext::at(chrono::Local::now().naive_local())
        };
        let pipeline = engine.pipeline_for(&active.preset);
        self.ui.show(OverlayState::Message {
            tone: Tone::Info,
            text: "Working on it…".into(),
        });
        let planned =
            tauri::async_runtime::block_on(pipeline.plan_action(request, &ctx, Some(&enabled)));
        let planned = match planned {
            Ok(planned) => planned,
            Err(e) => {
                let message = errors::instruction(&e, pipeline);
                return self.notice(Tone::Warning, &short(&message));
            }
        };
        tracing::info!(
            "action → {} ({:?})",
            plan_label(&planned.plan),
            planned.llm_time
        );
        crate::spend::shared().record(
            &[(planned.llm_name.clone(), planned.usage)],
            chrono::Local::now(),
        );
        let action = match planned.plan {
            splashdown_core::action::Plan::Do(action) => action,
            splashdown_core::action::Plan::Nothing(reason) => {
                let reason = if reason.is_empty() {
                    "Splashdown can't do that one".to_string()
                } else {
                    reason
                };
                return self.notice(
                    Tone::Info,
                    &format!("{} · say “what can I say?”", short(&reason)),
                );
            }
            splashdown_core::action::Plan::Invalid(why) => {
                tracing::info!("action refused: {why}");
                return self.notice(Tone::Warning, "Didn't do that: it wasn't clear what to do");
            }
        };
        let settings = engine.actions.clone();
        let language = engine.hints.language.clone();
        if let splashdown_core::action::Action::Splashdown { what, preset } = &action
            && self.own_action(what, preset.as_deref())
        {
            return;
        }
        let app = self.ui.app().clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result =
                crate::voice_actions::carry_out(&app, action, &settings, language.as_deref());
            let _ = tx.send(Msg::ActionDone(result));
        });
    }

    /// Splashdown's own actions that change the session (a preset, retry,
    /// hands-free). Whether it was one of those.
    pub(super) fn own_action(&mut self, what: &str, preset: Option<&str>) -> bool {
        match what {
            "switch_preset" => {
                let Ok(engine) = &self.engine else {
                    return true;
                };
                let names: Vec<String> = engine.presets.iter().map(|p| p.name.clone()).collect();
                match preset.and_then(|p| splashdown_core::action::best_name(p, &names)) {
                    Some(i) => {
                        let preset = engine.presets[i].clone();
                        self.pinned_preset = Some(preset.id.clone());
                        self.refresh_tray();
                        self.done(&format!("Using the {} preset", preset.name));
                    }
                    None => self.notice(
                        Tone::Info,
                        &format!("No preset called “{}”", preset.unwrap_or_default()),
                    ),
                }
                true
            }
            "retry_last_dictation" => {
                self.retry_last();
                true
            }
            "hands_free" => {
                self.handle(Action::Start);
                self.handle(Action::HandsFree);
                true
            }
            "show_commands" => {
                self.what_to_say();
                true
            }
            _ => false,
        }
    }

    /// "What can I say?": the list of commands, in its own window.
    pub(super) fn what_to_say(&mut self) {
        let app = self.ui.app().clone();
        let _ = self
            .ui
            .app()
            .run_on_main_thread(move || crate::help::open(&app));
        self.ui.hide();
    }
}

/// A plan, for the log: the action's id, never what it was asked to do.
pub(super) fn plan_label(plan: &splashdown_core::action::Plan) -> &'static str {
    match plan {
        splashdown_core::action::Plan::Do(action) => action.id(),
        splashdown_core::action::Plan::Nothing(_) => "none",
        splashdown_core::action::Plan::Invalid(_) => "refused",
    }
}

/// Where the last dictation sits in a field's text: just before the cursor
/// (spaces between them allowed). Byte offsets.
pub(super) fn last_dictation_range(
    text: &str,
    cursor: usize,
    last: &str,
) -> Option<(usize, usize)> {
    let last = last.trim();
    let before = text.get(..cursor)?.trim_end();
    if last.is_empty() || !before.ends_with(last) {
        return None;
    }
    Some((before.len() - last.len(), before.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_last_dictation_before_the_cursor() {
        let text = "Hi team. The build is green. ";
        let at = |s: &str| text.find(s).unwrap();
        assert_eq!(
            last_dictation_range(text, text.len(), "The build is green."),
            Some((at("The"), at("The") + "The build is green.".len()))
        );
        // Typed after it: no longer "that".
        assert_eq!(
            last_dictation_range("The build is green. ok", 22, "The build is green."),
            None
        );
        assert_eq!(last_dictation_range(text, text.len(), ""), None);
    }
}
