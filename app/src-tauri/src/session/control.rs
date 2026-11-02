//! Requests from the control channel: the `splash` CLI, `splashdown://`
//! links, and coding agents through `splashdown-mcp` (see
//! `crates/splashdown-control`). Handled on the session thread like the
//! dictation key's gestures, and through the same `Recognizer`, so a
//! recording started here can be finished with the key or Esc.

use std::sync::mpsc;
use std::time::Instant;

use splashdown_control::Request;
use splashdown_core::gesture::Action;
use splashdown_platform::{OverlayState, Selection, Tone};

use super::{Active, Controller};

/// Where a control request's answer goes: `Ok(Some(text))` for `Listen`.
pub(super) type Reply = mpsc::Sender<Result<Option<String>, String>>;

const NOT_RECORDING: &str = "Not recording";

impl Controller {
    pub(super) fn control(&mut self, request: Request, reply: Reply) {
        let recording = self.lock().is_recording();
        let result = match request {
            Request::Toggle | Request::Stop if recording => {
                let finished = self.lock().finish();
                if let Some(a) = finished {
                    self.handle(a);
                }
                Ok(None)
            }
            Request::Toggle | Request::Start if !recording => self.start_external().map(|()| None),
            // Already recording.
            Request::Toggle | Request::Start => Ok(None),
            Request::Stop => Err(NOT_RECORDING.into()),
            Request::Cancel => {
                let canceled = self.lock().cancel();
                match canceled {
                    Some(a) => {
                        self.handle(a);
                        Ok(None)
                    }
                    None => Err(NOT_RECORDING.into()),
                }
            }
            Request::Note { text: None } if recording => {
                let finished = self.lock().finish();
                if let Some(a) = finished {
                    self.handle(a);
                }
                Ok(None)
            }
            Request::Note { text: None } => self.start_external().map(|()| {
                self.handle(Action::Capture);
                None
            }),
            Request::Note { text: Some(text) } => self.capture_text(&text),
            Request::SearchNotes { query, limit } => self
                .notes_settings()
                .and_then(|n| crate::notes::agent_search(&n, &query, limit))
                .map(Some),
            Request::Status => serde_json::to_string(&self.status(recording))
                .map(Some)
                .map_err(|e| e.to_string()),
            Request::MeetingStart => crate::meeting::start(self.ui.app()).map(Some),
            Request::MeetingStop => crate::meeting::stop()
                .map(|()| Some("Stopping; the note is being finished.".to_string())),
            Request::MeetingStatus => serde_json::to_string(&crate::meeting::status())
                .map(Some)
                .map_err(|e| e.to_string()),
            Request::ReloadSettings => {
                self.ui.reload_settings();
                Ok(None)
            }
            Request::ReadNote { path } => self
                .notes_settings()
                .and_then(|n| crate::notes::agent_read(&n, &path))
                .map(Some),
            Request::AppendNote { text, to } => self
                .notes_settings()
                .and_then(|n| crate::notes::agent_append(&n, &text, to.as_deref()))
                .map(Some),
            Request::Settings => {
                tracing::info!("opened again while running: showing Settings");
                let app = self.ui.app().clone();
                let _ = self
                    .ui
                    .app()
                    .run_on_main_thread(move || crate::tray::open_settings(&app));
                Ok(None)
            }
            Request::Listen { .. } if recording => Err("Already recording".into()),
            Request::Listen { prompt } => match self.start_external() {
                Ok(()) => {
                    // Answered when the recording ends (`answer`), or the
                    // reply is dropped if it's discarded.
                    self.listen(prompt, reply);
                    return;
                }
                Err(e) => Err(e),
            },
        };
        let _ = reply.send(result);
    }

    /// For `splashdown-cli status`.
    fn status(&self, recording: bool) -> serde_json::Value {
        let (ready, problem, speech, ai) = match &self.engine {
            Ok(engine) => (
                true,
                None,
                Some(engine.pipeline.stt_name()),
                engine.pipeline.llm_name(),
            ),
            Err(e) => (false, Some(e.clone()), None, None),
        };
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "ready": ready,
            "problem": problem,
            "recording": recording,
            "working": self.active.is_some() && !recording,
            "speech": speech,
            "ai": ai,
            "meeting": crate::meeting::status(),
        })
    }

    /// Notes settings as last saved. Not the engine's copy: notes work
    /// without a usable engine, and a folder picked in Notes applies at once.
    pub(super) fn notes_settings(&self) -> Result<splashdown_config::NotesSettings, String> {
        self.ui
            .settings()
            .map(|s| s.notes)
            .ok_or_else(|| "Splashdown is still starting.".to_string())
    }

    /// `splashdown-cli note <text>`: straight into the capture note, with
    /// no recording or AI. ⌃⌥Z takes it back like a spoken capture.
    fn capture_text(&mut self, text: &str) -> Result<Option<String>, String> {
        if text.trim().is_empty() {
            return Err("There's no text to add.".into());
        }
        let captured = crate::notes::capture(&self.notes_settings()?, None, text, None)?;
        let message = format!("Added to {}", captured.label);
        self.last_capture = Some((captured, Instant::now()));
        self.done(&message);
        Ok(Some(message))
    }

    /// A preset's shortcut: finish the recording in progress, or start a
    /// hands-free one with that preset (whatever the app would match).
    pub(super) fn preset_shortcut(&mut self, preset_id: &str) {
        if self.lock().is_recording() {
            let finished = self.lock().finish();
            if let Some(a) = finished {
                self.handle(a);
            }
            return;
        }
        let preset = self
            .engine
            .as_ref()
            .ok()
            .and_then(|e| e.presets.iter().find(|p| p.id == preset_id))
            .cloned();
        let Some(preset) = preset else {
            return;
        };
        if let Err(e) = self.start_external() {
            tracing::info!("preset shortcut: {e}");
            return;
        }
        self.use_preset(preset);
    }

    /// Starts a hands-free recording into the focused field, as if the
    /// dictation key had been double-tapped.
    fn start_external(&mut self) -> Result<(), String> {
        let started = self.lock().start_hands_free(Instant::now());
        let Some(start) = started else {
            return Err("Already recording".into());
        };
        self.last_notice = None;
        self.handle(start);
        if self.active.is_none() {
            // It didn't start, and the overlay says why. Don't leave the
            // recognizer waiting for a tap.
            let _ = self.lock().cancel();
            return Err(self
                .last_notice
                .take()
                .unwrap_or_else(|| "Couldn't start recording".into()));
        }
        self.handle(Action::HandsFree);
        Ok(())
    }

    /// Turns the recording just started into an answer for `reply`: plain
    /// dictation (no field, no selection), with the question on screen.
    fn listen(&mut self, prompt: Option<String>, reply: Reply) {
        // The pinned preset, or the default one: not the focused app's.
        let preset = self.match_preset(None);
        let Some(active) = &mut self.active else {
            return;
        };
        active.focus = None;
        active.field_text = None;
        active.selection = Selection::Empty;
        active.preset = preset;
        active.reply_to = Some(reply);
        active.shown = match prompt.filter(|p| !p.trim().is_empty()) {
            Some(text) => OverlayState::Question { text },
            None => OverlayState::HandsFree { preset: None },
        };
        self.ui.show(active.shown.clone());
    }

    /// `finish` for a `Listen`: runs the pipeline and replies with the text
    /// instead of inserting it.
    pub(super) fn answer(&mut self, mut active: Active, reply: Reply) {
        self.last_notice = None;
        let outcome = self
            .prepare(&mut active)
            .and_then(|job| self.run_pipeline(job, &active.preset, None));
        // Return can't send anything here; hand it back to apps.
        let _ = self.lock().take_send();
        let result = match outcome {
            Some((outcome, took)) => {
                crate::spend::record_outcome(&outcome);
                let result = if outcome.text.trim().is_empty() {
                    self.notice(Tone::Info, "Didn't catch that");
                    Err("Didn't catch that".to_string())
                } else {
                    self.record(&outcome, None, &active.preset, took);
                    self.done("Answer sent");
                    Ok(Some(outcome.text))
                };
                self.check_budget();
                result
            }
            None => Err(self
                .last_notice
                .take()
                .unwrap_or_else(|| "Didn't catch that".into())),
        };
        let _ = reply.send(result);
    }
}
