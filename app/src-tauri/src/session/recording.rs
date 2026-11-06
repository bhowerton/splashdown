//! Starting a recording and what happens while the key is held: the
//! preset, the live preview, and switching to a note, a question or a
//! draft (the note, ask and compose letters).

use super::*;

impl Controller {
    pub(super) fn on_tick(&mut self) {
        let now = Instant::now();
        let timed_out = self.lock().on_timeout(now);
        if let Some(a) = timed_out {
            self.handle(a);
        }
        if self.hide_at.is_some_and(|h| now >= h) {
            self.hide_at = None;
            self.ui.hide();
        }
        self.maybe_preview(now);
        self.check_screen();
        if self.correction_check.as_ref().is_some_and(|c| now >= c.due) {
            self.check_corrections(false);
        }
        if let Some(chunker) = self.active.as_mut().and_then(|a| a.chunker.as_mut()) {
            chunker.tick(&self.audio);
        }
        if let Some(live) = self.active.as_mut().and_then(|a| a.stream.as_mut()) {
            // In pieces of 100 ms or more, like the models expect.
            let new = self.audio.since(live.fed);
            if new.samples.len() >= new.sample_rate as usize / 10 {
                live.feed(&new, live.fed);
            }
            let partial = live.stream.partial().filter(|_| live.show);
            if let Some(text) = partial.filter(|t| !t.is_empty()) {
                self.ui.live_text(Some(text));
            }
        }
        if let (Some(active), Ok(engine)) = (self.active.as_mut(), &self.engine)
            && active.quiet_due.is_some_and(|due| now >= due)
        {
            active.quiet_due = None;
            active.quiet = engine
                .quiet_others
                .and_then(splashdown_platform::quiet_output);
        }
        let quiet = self.engine.as_ref().is_ok_and(|e| e.whisper_mode);
        let mut mic_silent = false;
        let mut mic_back = None;
        if self.active.as_ref().is_some_and(|a| a.level.lost()) {
            // Unplugged or switched off: finish with what it recorded rather
            // than recording silence until the key is released.
            tracing::warn!("the microphone disconnected; finishing with what was recorded");
            self.mic_lost = true;
            let finished = self.lock().finish();
            if let Some(a) = finished {
                self.handle(a);
            }
        }
        if let Some(active) = &mut self.active {
            let raw = active.level.get();
            // Sound arrived after the warning (unmuted, or switched): put
            // the recording display back.
            if active.mic_warned && active.loudest < MIC_SILENT && raw >= MIC_SILENT {
                mic_back = Some(active.shown.clone());
            }
            active.loudest = active.loudest.max(raw);
            // Nothing at all for a while: a muted or wrong microphone, not
            // a quiet room.
            if !active.mic_warned
                && active.started.elapsed() > MIC_CHECK_AFTER
                && active.loudest < MIC_SILENT
            {
                active.mic_warned = true;
                mic_silent = true;
            }
            // Whisper mode: quiet speech still moves the meter and counts
            // as speaking.
            let level = raw * if quiet { WHISPER_BOOST } else { 1.0 };
            self.ui.level(level);
            if level > SPEECH_LEVEL {
                active.last_voice = now;
            }
            let quiet_too_long = now.duration_since(active.last_voice) > HANDS_FREE_SILENCE;
            if active.hands_free && (active.started.elapsed() > HANDS_FREE_MAX || quiet_too_long) {
                let finished = self.lock().finish();
                if let Some(a) = finished {
                    self.handle(a);
                }
            }
        }
        if mic_silent {
            tracing::warn!("the microphone has picked up nothing");
            self.ui.show(OverlayState::Message {
                tone: Tone::Warning,
                text: MIC_SILENT_NOTICE.into(),
            });
        }
        if let Some(shown) = mic_back {
            self.ui.show(shown);
        }
        if self.listener.is_none() {
            self.try_listen();
        }
    }

    pub(super) fn start(&mut self) {
        self.mic_lost = false;
        // The user has moved on: look at the previous insertion now.
        if self.correction_check.is_some() {
            self.check_corrections(true);
        }
        // The layout may have changed since the last dictation; this runs
        // on the main thread long before we paste.
        self.ui.refresh_shortcut_keys();
        // A fresh press replaces anything unfinished.
        if self.active.take().is_some() {
            self.audio.stop();
        }
        self.hide_at = None;

        if let Err(e) = &self.engine {
            let msg = format!("Not set up: {e}");
            self.notice(Tone::Warning, &msg);
            return;
        }
        // A new day or month may have lifted the spending limit.
        if self.engine.as_ref().is_ok_and(|e| e.over_budget) {
            self.check_budget();
        }
        if splashdown_platform::secure_input_active() {
            self.notice(Tone::Warning, "Can't dictate into password fields");
            return;
        }

        // Start the mic first so the first word isn't lost while we look
        // at focus and selection.
        let microphones = self
            .engine
            .as_ref()
            .map(|e| e.microphones.clone())
            .unwrap_or_default();
        let level = match self.audio.start(microphones) {
            Ok(level) => level,
            Err(e) => {
                self.notice(Tone::Warning, &format!("Microphone unavailable: {e}"));
                return;
            }
        };

        if let Ok(e) = &self.engine
            && e.sounds
        {
            self.ui.sound(Sound::Start, e.sound_volume);
        }

        let focus = Focus::capture();
        if focus.as_ref().is_some_and(Focus::is_secure_field) {
            self.audio.stop();
            self.notice(Tone::Warning, "Can't dictate into password fields");
            return;
        }
        let selection = focus.as_ref().map_or(Selection::Unknown, Focus::selection);
        let near_cursor = self.engine.as_ref().is_ok_and(|e| e.overlay_near_cursor);
        self.ui.anchor(
            focus
                .as_ref()
                .filter(|_| near_cursor)
                .and_then(Focus::caret_rect),
        );
        let preset = self.match_preset(focus.as_ref());
        let field_text = preset
            .field_context
            .then(|| focus.as_ref().and_then(read_field_context))
            .flatten();
        let clipboard = preset.clipboard_context.then(read_clipboard).flatten();
        // The speech model the user chose may still be loading (the first
        // load after an update takes a while): say so.
        let loading = self
            .engine
            .as_ref()
            .is_ok_and(|e| !e.pipeline_for(&preset).stt().ready());
        let shown = match &selection {
            Selection::Text(s) => OverlayState::Transform {
                words: s.split_whitespace().count(),
            },
            _ if loading => OverlayState::Listening {
                preset: Some(match preset_label(&preset) {
                    Some(name) => format!("{name} · speech model loading…"),
                    None => "speech model loading…".into(),
                }),
            },
            _ => OverlayState::Listening {
                preset: preset_label(&preset),
            },
        };
        self.ui.show(shown.clone());
        // A local text model loads while the user speaks.
        if let Ok(engine) = &self.engine {
            engine.pipeline_for(&preset).warm_up();
        }
        let project = preset
            .file_tags
            .then(|| focus.as_ref().and_then(|f| f.app.document.clone()))
            .flatten()
            .map(list_project);
        let spoken = self.wants_their_language(focus.as_ref(), &selection, &preset);
        let screen_words = self
            .engine
            .as_ref()
            .is_ok_and(|e| e.screen_words)
            .then(|| focus.as_ref().map(|f| f.app.pid))
            .flatten()
            .map(|pid| read_screen_words(pid, spoken));
        // Transforms replace a selection, so they never press Return.
        let auto_send = preset.auto_send && !matches!(selection, Selection::Text(_));

        let stream = self
            .engine
            .as_ref()
            .ok()
            .filter(|_| !matches!(selection, Selection::Text(_)))
            .and_then(|e| live_stream(e, &preset));
        let chunker = self
            .engine
            .as_ref()
            .ok()
            .filter(|_| !matches!(selection, Selection::Text(_)))
            .filter(|_| stream.is_none())
            .map(|e| Chunker::new(e.pipeline.stt(), e.hints_for(&preset), e.audio_cleanup()));

        self.next_session += 1;
        let now = Instant::now();
        self.active = Some(Active {
            session: self.next_session,
            preview_due: now + PREVIEW_START,
            preview_busy: false,
            focus,
            selection,
            level,
            content: None,
            started: now,
            last_voice: now,
            loudest: 0.0,
            mic_warned: false,
            capture: false,
            ask: false,
            compose: false,
            quiet_due: self
                .engine
                .as_ref()
                .ok()
                .filter(|e| e.quiet_others.is_some())
                .map(|e| {
                    now + if e.sounds {
                        QUIET_AFTER_SOUND
                    } else {
                        Duration::ZERO
                    }
                }),
            quiet: None,
            hands_free: false,
            shown,
            preset,
            field_text,
            clipboard,
            chunker,
            stream,
            reply_to: None,
            answer_to: None,
            project,
            screen_words,
            screen: None,
            untranslated: None,
        });
        if auto_send {
            self.lock().set_send(true);
            self.ui.set_send(true);
        }
    }

    /// The languages the user dictates in, when this dictation should come
    /// out in the conversation's language if that's another one: the
    /// setting is on, it's email or chat, and nothing else decides the
    /// language (a selection, a preset that translates).
    fn wants_their_language(
        &self,
        focus: Option<&Focus>,
        selection: &Selection,
        preset: &Preset,
    ) -> Option<Vec<String>> {
        use splashdown_core::style::Kind;
        let engine = self.engine.as_ref().ok()?;
        if !engine.reply_in_their_language
            || matches!(selection, Selection::Text(_))
            || preset.translate_to.is_some()
        {
            return None;
        }
        let focus = focus?;
        let kind = splashdown_config::style_kind(&AppContext {
            app_id: &focus.app.id,
            app_name: &focus.app.name,
            title: focus.app.window_title.as_deref(),
            url: focus.app.url.as_deref(),
        });
        if !matches!(kind, Kind::Email | Kind::Chat | Kind::Texts) {
            return None;
        }
        let hints = engine.hints_for(preset);
        let mut spoken: Vec<String> = hints
            .language
            .into_iter()
            .chain(hints.languages)
            .filter(|l| !l.trim().is_empty())
            .collect();
        // Automatic: the language the computer is set to.
        if spoken.is_empty() {
            spoken.extend(splashdown_platform::system_language());
        }
        (!spoken.is_empty()).then_some(spoken)
    }

    /// Once the window has been read: when the conversation is in another
    /// language, the dictation comes out in it, and the overlay says so.
    pub(super) fn check_screen(&mut self) {
        let Some(active) = &mut self.active else {
            return;
        };
        if active.screen.is_some() {
            return;
        }
        let Some(screen) = active
            .screen_words
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        else {
            return;
        };
        active.screen_words = None;
        active.screen = Some(screen);
        if let Some(preset) = their_preset(active) {
            active.untranslated = Some(active.preset.clone());
            self.use_preset(preset);
        }
    }

    /// At the end of a dictation whose window read came in only now (a
    /// short one): the same, without the overlay.
    pub(super) fn check_screen_at_end(&self, active: &mut Active) {
        if active.screen.is_none() {
            active.screen = active
                .screen_words
                .take()
                .and_then(|rx| rx.recv_timeout(Duration::from_millis(300)).ok());
        }
        if let Some(preset) = their_preset(active) {
            active.untranslated = Some(std::mem::replace(&mut active.preset, preset));
        }
    }

    /// Sends the audio so far to the preview model when a pass is due.
    pub(super) fn maybe_preview(&mut self, now: Instant) {
        let (Some(worker), Some(active)) = (&self.preview, &mut self.active) else {
            return;
        };
        if active.preview_busy || now < active.preview_due || active.stream.is_some() {
            return;
        }
        active.preview_busy = true;
        worker.submit(preview::Job {
            session: active.session,
            clip: self.audio.snapshot(),
        });
    }

    pub(super) fn on_partial(&mut self, partial: preview::Partial) {
        let Some(active) = &mut self.active else {
            return;
        };
        if active.session != partial.session {
            return;
        }
        active.preview_busy = false;
        active.preview_due = Instant::now() + PREVIEW_EVERY;
        if !partial.text.is_empty() {
            self.ui.live_text(Some(partial.text));
        }
    }

    /// The note key while recording: the dictation will go into a note.
    pub(super) fn start_capture(&mut self) {
        let Ok(notes) = self.notes_settings() else {
            return;
        };
        let label = format!("Note · {}", crate::notes::capture_label(&notes));
        let Some(active) = &mut self.active else {
            return;
        };
        active.capture = true;
        active.ask = false;
        active.compose = false;
        match &mut active.shown {
            OverlayState::Listening { preset } | OverlayState::HandsFree { preset } => {
                *preset = Some(label);
                self.ui.show(active.shown.clone());
            }
            // Selected text is left alone: this is a note.
            OverlayState::Transform { .. } => {
                active.shown = if active.hands_free {
                    OverlayState::HandsFree {
                        preset: Some(label),
                    }
                } else {
                    OverlayState::Listening {
                        preset: Some(label),
                    }
                };
                self.ui.show(active.shown.clone());
            }
            _ => {}
        }
        // Nothing is typed, so there's nothing to send.
        self.lock().set_send(false);
        self.ui.set_send(false);
    }

    /// The ask key: what's said is a question, answered in the Ask panel.
    pub(super) fn start_ask(&mut self) {
        let Some(active) = &mut self.active else {
            return;
        };
        active.ask = true;
        active.capture = false;
        active.compose = false;
        let label = Some("Ask".to_string());
        active.shown = if active.hands_free {
            OverlayState::HandsFree { preset: label }
        } else {
            OverlayState::Listening { preset: label }
        };
        self.ui.show(active.shown.clone());
        // Nothing is typed, so there's nothing to send.
        self.lock().set_send(false);
        self.ui.set_send(false);
    }

    /// The compose key while holding the dictation key: what's said is what
    /// to write, for Compose.
    pub(super) fn start_compose(&mut self) {
        let Some(active) = &mut self.active else {
            return;
        };
        active.compose = true;
        active.capture = false;
        active.ask = false;
        let label = Some("Compose".to_string());
        active.shown = if active.hands_free {
            OverlayState::HandsFree { preset: label }
        } else {
            OverlayState::Listening { preset: label }
        };
        self.ui.show(active.shown.clone());
        // Nothing is typed, so there's nothing to send.
        self.lock().set_send(false);
        self.ui.set_send(false);
    }

    /// Number key while recording: switch to preset `n` (1-based).
    pub(super) fn select_preset(&mut self, n: u8) {
        let preset = self
            .engine
            .as_ref()
            .ok()
            .and_then(|e| e.presets.get(usize::from(n) - 1))
            .cloned();
        if let Some(preset) = preset {
            self.use_preset(preset);
        }
    }

    /// Switches the recording in progress to `preset`.
    pub(super) fn use_preset(&mut self, preset: Preset) {
        let Ok(engine) = &self.engine else {
            return;
        };
        let Some(active) = &mut self.active else {
            return;
        };
        let old = std::mem::replace(&mut active.preset, preset.clone());
        if preset.field_context && active.field_text.is_none() {
            active.field_text = active.focus.as_ref().and_then(read_field_context);
        }
        if preset.clipboard_context && active.clipboard.is_none() {
            active.clipboard = read_clipboard();
        }
        // A stream, and pieces already transcribed, were in the old preset's
        // language.
        if old.language != preset.language {
            active.stream = None;
        }
        if old.language != preset.language && active.chunker.is_some() {
            active.chunker = Some(Chunker::new(
                engine.pipeline.stt(),
                engine.hints_for(&preset),
                engine.audio_cleanup(),
            ));
        }
        engine.pipeline_for(&preset).warm_up();
        let transform = matches!(active.selection, Selection::Text(_));
        if old.auto_send != preset.auto_send && !transform {
            self.lock().set_send(preset.auto_send);
            self.ui.set_send(preset.auto_send);
        }
        let Some(active) = &mut self.active else {
            return;
        };
        let label = Some(preset.name.clone());
        match &mut active.shown {
            OverlayState::Listening { preset } | OverlayState::HandsFree { preset } => {
                *preset = label;
                self.ui.show(active.shown.clone());
            }
            // Mid-instruction or transform: the overlay keeps its state.
            _ => {}
        }
    }

    pub(super) fn match_preset(&self, focus: Option<&Focus>) -> Preset {
        let presets = self
            .engine
            .as_ref()
            .map(|e| e.presets.as_slice())
            .unwrap_or(&[]);
        if let Some(pinned) = self
            .pinned_preset
            .as_ref()
            .and_then(|id| presets.iter().find(|p| &p.id == id))
        {
            return pinned.clone();
        }
        let ctx = focus.map(|f| AppContext {
            app_id: &f.app.id,
            app_name: &f.app.name,
            title: f.app.window_title.as_deref(),
            url: f.app.url.as_deref(),
        });
        ctx.and_then(|c| match_preset(presets, &c))
            .or_else(|| presets.iter().find(|p| p.id == DEFAULT_PRESET_ID))
            .cloned()
            .unwrap_or_default()
    }
}

pub(super) fn preview_worker(
    engine: &Result<Engine, String>,
    tx: &mpsc::Sender<Msg>,
) -> Option<PreviewWorker> {
    let engine = engine.as_ref().ok()?;
    let stt = engine.preview.clone()?;
    let tx = tx.clone();
    Some(PreviewWorker::spawn(stt, engine.hints.clone(), move |p| {
        let _ = tx.send(Msg::Partial(p));
    }))
}

/// The stream for a new recording, if the user's speech model can
/// transcribe while they speak: its text is the dictation's. Not with noise
/// reduction or whisper mode, which work on the whole recording.
pub(super) fn live_stream(engine: &Engine, preset: &Preset) -> Option<LiveStream> {
    if engine.audio_cleanup() != splashdown_audio::Cleanup::default() {
        return None;
    }
    let stream = engine
        .pipeline_for(preset)
        .stt()
        .stream(&engine.hints_for(preset))?;
    Some(LiveStream {
        stream,
        fed: 0,
        show: engine.preview.is_some(),
    })
}

/// The overlay names a preset unless it's the default one.
pub(super) fn preset_label(preset: &Preset) -> Option<String> {
    (preset.id != DEFAULT_PRESET_ID && !preset.name.is_empty()).then(|| preset.name.clone())
}

/// The preset that writes `active`'s dictation in the conversation's
/// language, when the window read found one and nothing chose otherwise
/// since (T, a note, a question, a draft, a selection).
fn their_preset(active: &Active) -> Option<Preset> {
    let code = active.screen.as_ref()?.their_language?;
    if active.capture
        || active.ask
        || active.compose
        || active.preset.translate_to.is_some()
        || matches!(active.selection, Selection::Text(_))
    {
        return None;
    }
    let language = splashdown_core::conversation::english_name(code)?;
    let mut preset = super::edits::translating(active.preset.clone(), language);
    preset.name = match preset_label(&active.preset) {
        Some(name) => format!("{name} → {language} (their language)"),
        None => format!("→ {language} (their language)"),
    };
    tracing::info!("the conversation is in {language}: dictating into it");
    Some(preset)
}
