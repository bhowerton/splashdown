//! Turning a finished recording into a pipeline job: the audio, the mode
//! (dictation, an instruction, a rewrite), and the context the AI is told
//! (the app, the field, the clipboard, names on screen, a project's files).

use super::*;

impl Controller {
    /// Stops recording and turns it into a pipeline job, or shows why not.
    pub(super) fn prepare(&mut self, active: &mut Active) -> Option<AudioJob> {
        let rest = self.audio.finish();
        // A stream of the whole dictation gets the rest of the audio now,
        // and its text is ready a moment later (below).
        let mut stream = active.stream.take();
        if let Some(live) = &mut stream {
            live.feed(&rest, 0);
        }
        // Other sounds come back before the stop sound plays.
        active.quiet_due = None;
        active.quiet = None;
        let Ok(engine) = &self.engine else {
            self.ui.hide();
            return None;
        };
        if engine.sounds {
            self.ui.sound(Sound::Stop, engine.sound_volume);
        }
        let label = if engine.pipeline_for(&active.preset).stt().ready() {
            "Working…"
        } else {
            "Loading speech model…"
        };
        self.ui.show(OverlayState::Working {
            label: label.into(),
        });

        // A note capture is what was said: text selected in the app isn't
        // something to change.
        let mut selection = match &active.selection {
            Selection::Text(s) if !active.capture => Some(s.clone()),
            _ => None,
        };
        // The app didn't report its selection: try copying it. Only after a
        // hold (the dictation key is up again), so our Cmd+C isn't combined
        // with a held modifier.
        if active.selection == Selection::Unknown
            && !active.capture
            && !active.hands_free
            && active.focus.as_ref().is_some_and(Focus::is_current)
        {
            selection = splashdown_platform::copy_selection();
        }

        let (mode, speech, instruction) = split_recording(
            selection.is_some(),
            active.content.take(),
            rest,
            active.preset.ai,
        );
        let heard_nothing = active.loudest < MIC_SILENT;
        let Some(whole) = engine.prepare_audio(&speech) else {
            // Nothing said before the instruction key: a spoken command.
            if mode == Mode::DictateInstruct
                && let Some(command) = instruction.as_ref().and_then(|i| engine.prepare_audio(i))
            {
                self.pending_command = Some(command);
                return None;
            }
            // Why nothing happened, for the log: lengths and levels only.
            let secs = |c: &AudioClip| c.duration().as_secs_f32();
            tracing::info!(
                "no speech found ({:?}): {:.1} s{}, loudest {:.4}",
                mode,
                secs(&speech),
                instruction.as_ref().map_or(String::new(), |i| format!(
                    " + {:.1} s after the instruction key",
                    secs(i)
                )),
                active.loudest
            );
            if heard_nothing {
                self.notice(
                    Tone::Warning,
                    &mic_silent_notice(active.microphone.as_deref()),
                );
                // Offered once the notice is gone and nothing is recording.
                self.silent_microphone = active.microphone.clone();
            } else {
                self.notice(Tone::Info, "Didn't catch that");
            }
            return None;
        };
        // A long dictation: what came before the last pause is already
        // being transcribed; only the rest is left.
        let chunks = active
            .chunker
            .take()
            .filter(|_| matches!(mode, Mode::Dictate | Mode::Raw))
            .and_then(|c| c.finish(whole.clone()));
        let (speech, prefix) = match chunks {
            Some((prefix, cut)) => {
                let tail = AudioClip::new(
                    speech.samples.get(cut..).unwrap_or_default().to_vec(),
                    speech.sample_rate,
                );
                let tail = engine
                    .prepare_audio(&tail)
                    .unwrap_or_else(|| AudioClip::new(Vec::new(), whole.sample_rate));
                (tail, Some(prefix))
            }
            None => (whole, None),
        };
        // Streamed as it was recorded: that text is the transcript, and the
        // recording is only transcribed if the stream failed.
        let streamed = stream
            .filter(|_| matches!(mode, Mode::Dictate | Mode::Raw))
            .and_then(|live| match live.stream.finish() {
                Ok(text) => Some(text),
                Err(e) => {
                    tracing::warn!("stream failed; transcribing the recording instead: {e}");
                    None
                }
            });
        let (speech, prefix) = match streamed {
            Some(text) => (
                AudioClip::new(Vec::new(), speech.sample_rate),
                Some(Prefix {
                    transcript: Box::pin(std::future::ready(Ok((text, Usage::FREE)))),
                    whole: speech,
                }),
            ),
            None => (speech, prefix),
        };
        let app_name = active
            .focus
            .as_ref()
            .filter(|_| engine.share_app_name)
            .map(|f| f.app.name.clone());
        // Earlier dictations into this app, unless the field's own text
        // (which already shows them) goes along.
        let earlier = match &active.focus {
            Some(focus)
                if engine.share_recent
                    && !active.capture
                    && matches!(mode, Mode::Dictate | Mode::DictateInstruct)
                    && active.field_text.is_none() =>
            {
                self.recent.for_app(&focus.app.id, Instant::now())
            }
            _ => Vec::new(),
        };
        Some(AudioJob {
            mode,
            speech,
            instruction_speech: instruction.as_ref().and_then(|i| engine.prepare_audio(i)),
            selection,
            context: PromptContext {
                clipboard: active
                    .clipboard
                    .clone()
                    .filter(|_| active.preset.clipboard_context),
                earlier,
                snippets: crate::replacements::fill_in(
                    &engine.context.snippets,
                    chrono::Local::now(),
                    active.clipboard.as_deref(),
                ),
                ..prompt_context(
                    &engine.context,
                    app_name,
                    &active.preset,
                    active.field_text.clone(),
                )
            },
            hints: engine.hints_for(&active.preset),
            standing_instruction: active.preset.standing_instruction(),
            prefix,
        })
    }
}

/// Splits a finished recording into what was said and what it's for.
/// With text selected, everything is an instruction for it (Transform);
/// after an instruction-key tap, speech before it is content and after it
/// is the instruction; otherwise it's plain dictation, inserted as heard
/// when the preset has the AI pass off.
pub(super) fn split_recording(
    has_selection: bool,
    before_mark: Option<AudioClip>,
    rest: AudioClip,
    preset_ai: bool,
) -> (Mode, AudioClip, Option<AudioClip>) {
    match (has_selection, before_mark) {
        (true, Some(content)) => (Mode::Transform, concat(content, rest), None),
        (true, None) => (Mode::Transform, rest, None),
        (false, Some(content)) => (Mode::DictateInstruct, content, Some(rest)),
        (false, None) if !preset_ai => (Mode::Raw, rest, None),
        (false, None) => (Mode::Dictate, rest, None),
    }
}

/// The engine's prompt context with this dictation's app, preset and (for
/// presets with field context on) the text around the cursor.
pub(super) fn prompt_context(
    base: &PromptContext,
    app_name: Option<String>,
    preset: &Preset,
    field_text: Option<String>,
) -> PromptContext {
    PromptContext {
        app_name,
        preset_instructions: preset.ai_instructions(),
        cleanup: preset.cleanup.unwrap_or_default(),
        field_text: field_text.filter(|_| preset.field_context),
        ..base.clone()
    }
}

/// Characters of field text sent before and after the cursor.
pub(super) const CONTEXT_BEFORE: usize = 1000;

pub(super) const CONTEXT_AFTER: usize = 200;

pub(super) fn read_field_context(focus: &Focus) -> Option<String> {
    focus.text_around_cursor(CONTEXT_BEFORE, CONTEXT_AFTER)
}

/// Most of the clipboard's text sent as context; more is cut.
pub(super) const CLIPBOARD_CHARS: usize = 2000;

/// The clipboard's text for a preset with clipboard context on.
pub(super) fn read_clipboard() -> Option<String> {
    let text = splashdown_platform::clipboard_text()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(CLIPBOARD_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    })
}

/// Lists the project around `document` on another thread.
pub(super) fn list_project(document: std::path::PathBuf) -> mpsc::Receiver<Option<Arc<Project>>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(project::for_document(&document));
    });
    rx
}

/// The project's files, if the listing finished in time.
pub(super) fn project_files(rx: mpsc::Receiver<Option<Arc<Project>>>) -> Option<Arc<Project>> {
    rx.recv_timeout(Duration::from_millis(400)).ok().flatten()
}

/// Project terms added to the vocabulary, at most. The user's own words
/// come first: speech models only read so much of the hint.
pub(super) const PROJECT_TERMS: usize = 40;

/// Names and terms on screen added to the vocabulary, at most.
pub(super) const SCREEN_TERMS: usize = 30;

/// What was read from the window around the field.
#[derive(Debug, Default)]
pub(super) struct Screen {
    /// Names and terms, for speech-to-text and the AI to spell.
    pub words: Vec<String>,
    /// The conversation's language (a two-letter code), when it was asked
    /// for and is clearly not one the user speaks.
    pub their_language: Option<&'static str>,
}

/// Reads the names and terms in the window around the focused field of
/// app `pid` in the background (Accessibility can be slow in big pages).
/// With `spoken` (the languages the user dictates in), it also tells the
/// conversation's language if it's another one.
pub(super) fn read_screen_words(pid: i32, spoken: Option<Vec<String>>) -> mpsc::Receiver<Screen> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let text = Focus::capture().filter(|f| f.app.pid == pid).and_then(|f| {
            f.surrounding_text(
                splashdown_core::compose::WINDOW_MIN_CHARS,
                splashdown_core::compose::WINDOW_MAX_CHARS,
            )
        });
        let words = text
            .as_deref()
            .map(|text| splashdown_core::vocab::terms_on_screen(text, SCREEN_TERMS))
            .unwrap_or_default();
        let their_language = spoken.zip(text.as_deref()).and_then(|(spoken, text)| {
            splashdown_core::conversation::reply_language(text, &spoken)
        });
        let _ = tx.send(Screen {
            words,
            their_language,
        });
    });
    rx
}

/// Adds names and terms from the screen to what speech-to-text and the AI
/// are told to spell, after the user's own words.
pub(super) fn with_screen_words(mut job: AudioJob, words: &[String]) -> AudioJob {
    let mut added = 0;
    for word in words {
        if !job
            .context
            .vocabulary
            .iter()
            .any(|v| v.eq_ignore_ascii_case(word))
        {
            job.hints.vocabulary.push(word.clone());
            job.context.vocabulary.push(word.clone());
            added += 1;
        }
    }
    if added > 0 {
        tracing::info!("{added} word(s) from the screen added to the vocabulary");
    }
    job
}

/// Adds the project's recently changed file names and its most-used code
/// names to what speech-to-text and the AI are told to spell.
pub(super) fn with_file_names(mut job: AudioJob, project: Option<&Project>) -> AudioJob {
    let Some(project) = project else {
        return job;
    };
    let (names, idents) = (&project.recent_names, &project.identifiers);
    // Alternate, so both kinds fit within the limit.
    let mixed = (0..names.len().max(idents.len()))
        .flat_map(|i| [names.get(i), idents.get(i)])
        .flatten();
    for term in mixed.take(PROJECT_TERMS) {
        if !job.hints.vocabulary.contains(term) {
            job.hints.vocabulary.push(term.clone());
            job.context.vocabulary.push(term.clone());
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(samples: usize) -> AudioClip {
        AudioClip::new(vec![0.0; samples], 16_000)
    }

    #[test]
    fn recordings_split_by_selection_and_instruction_mark() {
        let (mode, speech, instruction) = split_recording(false, None, clip(10), true);
        assert_eq!((mode, speech.samples.len()), (Mode::Dictate, 10));
        assert!(instruction.is_none());

        let (mode, speech, instruction) = split_recording(false, Some(clip(4)), clip(6), true);
        assert_eq!(mode, Mode::DictateInstruct);
        assert_eq!(
            speech.samples.len(),
            4,
            "content is what came before the mark"
        );
        assert_eq!(instruction.map(|c| c.samples.len()), Some(6));

        // With a selection, all of it is one instruction for the selection.
        let (mode, speech, _) = split_recording(true, Some(clip(4)), clip(6), true);
        assert_eq!((mode, speech.samples.len()), (Mode::Transform, 10));

        // A preset with the AI off only affects plain dictation.
        assert_eq!(split_recording(false, None, clip(1), false).0, Mode::Raw);
        assert_eq!(
            split_recording(true, None, clip(1), false).0,
            Mode::Transform
        );
    }

    #[test]
    fn prompt_context_takes_app_and_preset() {
        let base = PromptContext {
            user_name: Some("Alex".into()),
            ..PromptContext::default()
        };
        let preset = Preset {
            instructions: "Casual.".into(),
            ..Preset::default()
        };
        let ctx = prompt_context(&base, Some("Slack".into()), &preset, Some("Hi".into()));
        assert_eq!(ctx.app_name.as_deref(), Some("Slack"));
        assert_eq!(ctx.preset_instructions.as_deref(), Some("Casual."));
        assert_eq!(ctx.user_name.as_deref(), Some("Alex"));
        assert_eq!(ctx.field_text, None, "field context is off for this preset");
        let with_context = Preset {
            field_context: true,
            ..preset.clone()
        };
        assert_eq!(
            prompt_context(&base, None, &with_context, Some("Hi".into()))
                .field_text
                .as_deref(),
            Some("Hi")
        );

        let blank = Preset {
            instructions: "  ".into(),
            ..Preset::default()
        };
        assert_eq!(
            prompt_context(&base, None, &blank, None).preset_instructions,
            None
        );
    }
}
