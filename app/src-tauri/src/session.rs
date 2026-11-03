//! The dictation session controller. Turns hotkey gestures into
//! recording → speech-to-text → AI pass → insertion, and drives the overlay.
//!
//! Runs on its own thread. The key listener (another thread) feeds gestures
//! through the shared `Recognizer`; everything slow (audio, network,
//! insertion) happens here, never on the key or UI threads.
//!
//! One dictation at a time: `finish` blocks this thread on the pipeline, so
//! messages that arrive meanwhile (gestures, new settings, redo requests)
//! queue and run afterward. State the user changes *during* processing
//! (auto-send) therefore lives in the `Recognizer`, and its overlay badge is
//! drawn from the key thread. See docs/ARCHITECTURE.md, "Threads".
//!
//! Requests from the control channel (`splashdown-cli`, links, agents) are in
//! `session/control.rs`.

mod control;

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use splashdown_audio::Level;
use splashdown_core::command::Command;
use splashdown_core::gesture::{Action, PhysicalKey, Recognizer, Timing};
use splashdown_core::{
    AudioClip, AudioJob, Fallback, Mode, OnText, Outcome, PipelineError, Prefix, PromptContext,
    TextJob, Usage,
};
use splashdown_platform::{Focus, InsertMethod, KeyAction, OverlayState, Selection, Sound, Tone};

use splashdown_config::{AppContext, DEFAULT_PRESET_ID, InsertMode, Preset, match_preset};
use splashdown_control::Request;

use crate::audio::AudioWorker;
use crate::chunks::Chunker;
use crate::engine::Engine;
use crate::errors;
use crate::history::{self, History};
use crate::preview::{self, PreviewWorker};
use crate::project::{self, Project};
use crate::spend::BudgetState;
use crate::stream_typing::StreamTyper;
use crate::tray::TrayIconState;
use crate::ui::Ui;

/// How often the overlay waveform refreshes.
const LEVEL_TICK: Duration = Duration::from_millis(50);
/// How often to retry the key listener while permission is missing.
const LISTENER_RETRY: Duration = Duration::from_secs(2);
/// Hands-free recordings stop on their own after this long.
const HANDS_FREE_MAX: Duration = Duration::from_secs(10 * 60);
/// …or after this long without speech.
const HANDS_FREE_SILENCE: Duration = Duration::from_secs(3 * 60);
/// Microphone level (RMS) above which we count it as speech, for the
/// silence auto-stop. Quiet rooms sit well below this.
const SPEECH_LEVEL: f32 = 0.01;
/// Below this the microphone is sending (near) digital silence: muted, or
/// not the microphone the user is speaking into. Even a quiet room is
/// louder.
const MIC_SILENT: f32 = 0.0004;
/// How long to listen before saying the microphone hears nothing.
const MIC_CHECK_AFTER: Duration = Duration::from_millis(2500);
const MIC_SILENT_NOTICE: &str =
    "Microphone hears nothing — muted, or the wrong one? (Settings → General)";
/// How much the level meter (and hands-free's speech check) is raised in
/// whisper mode.
const WHISPER_BOOST: f32 = 5.0;
/// Live preview: first pass after this much audio, then one pass this often.
const PREVIEW_START: Duration = Duration::from_millis(500);
const PREVIEW_EVERY: Duration = Duration::from_millis(600);
const SHOW_SUCCESS: Duration = Duration::from_millis(1200);
/// Before pressing Return after inserting, so the target app has handled
/// the paste first (otherwise some apps send the message before the text).
const RETURN_AFTER_INSERT: Duration = Duration::from_millis(120);
/// Undoing or editing "that" only applies to an insertion this recent.
const UNDO_WINDOW: Duration = Duration::from_secs(120);
/// Time for the app to apply Undo before pasting over it.
const UNDO_SETTLE: Duration = Duration::from_millis(120);
const SHOW_NOTICE: Duration = Duration::from_millis(3000);
/// Other sounds are quieted this long after the start sound begins, so it
/// isn't cut off.
const QUIET_AFTER_SOUND: Duration = Duration::from_millis(300);

/// An insertion to re-check for corrections a little later.
struct PendingCorrectionCheck {
    focus: Focus,
    inserted: String,
    due: Instant,
}

/// How long after an insertion to look for corrections.
const CORRECTION_CHECK_AFTER: Duration = Duration::from_secs(20);

pub struct Keys {
    pub dictation: PhysicalKey,
    pub instruction: PhysicalKey,
    /// Pressed while holding the dictation key: carry on hands-free.
    pub hands_free: PhysicalKey,
    /// A double-tap starts hands-free too.
    pub double_tap: bool,
    /// Pressed while holding the dictation key: the dictation goes into a
    /// note.
    pub note: PhysicalKey,
    /// Pressed while holding the dictation key: the dictation is a question
    /// for the Ask panel.
    pub ask: PhysicalKey,
}

enum Msg {
    Action(Action),
    /// New settings: replace the engine and hotkeys.
    Reconfigure(Box<Result<Engine, String>>, Keys),
    Partial(preview::Partial),
    /// Re-run the AI pass on earlier words with another preset.
    Redo(Redo),
    /// Use this preset for every dictation (`None`: choose by app).
    PinPreset(Option<String>),
    /// From the control channel; see `session/control.rs`.
    Control(Request, control::Reply),
    /// A preset's own shortcut: start a dictation with it, or finish one.
    PresetShortcut(String),
    /// Replace the last AI-edited insertion with what was said.
    UndoAiEdit,
    /// Run the kept recording that failed again (`rescue`).
    RetryLast,
    /// Run a transform on text from Notes, and answer.
    Transform(TransformRequest),
    /// A transform's shortcut, in another app: run it on the selection.
    TransformShortcut(String),
    /// The speech-to-text for transcribing a file.
    FileSpeech(mpsc::Sender<Result<FileSpeech, String>>),
    /// The model that answers questions, for Ask in Notes.
    AnswerModel(mpsc::Sender<Result<AnswerModel, String>>),
    /// A voice action finished (`voice_actions`): what to show, and what
    /// "undo that" would take back.
    ActionDone(Result<crate::voice_actions::Done, String>),
}

/// What a question asked in Notes needs from the engine.
pub struct AnswerModel {
    pub model: Arc<dyn splashdown_core::TextModel>,
    pub user_name: Option<String>,
}

/// What a file transcription needs from the engine: dictation's
/// speech-to-text, hints and audio clean-up.
pub struct FileSpeech {
    pub stt: Arc<dyn splashdown_core::SpeechToText>,
    pub hints: splashdown_core::SttHints,
    pub cleanup: splashdown_audio::Cleanup,
}

pub struct TransformRequest {
    pub id: String,
    pub text: String,
    /// What the user asked for on top of the transform (or instead of one,
    /// with no `id`): "warmer", "in French".
    pub extra: Option<String>,
    pub reply: mpsc::Sender<Result<TransformOutcome, String>>,
}

/// Speech transcribed while it's recorded: fed the new audio on
/// each tick, finished on release.
struct LiveStream {
    stream: Box<dyn splashdown_core::SpeechStream>,
    /// Samples of the recording already fed.
    fed: usize,
    /// Show its partial text in the overlay (live preview is on).
    show: bool,
}

impl LiveStream {
    /// Feeds whatever of `clip` it hasn't had yet, as 16 kHz audio. `clip`
    /// is the recording (since the last take) from sample `start` on.
    fn feed(&mut self, clip: &AudioClip, start: usize) {
        let new = clip
            .samples
            .get(self.fed.saturating_sub(start)..)
            .unwrap_or_default();
        if new.is_empty() {
            return;
        }
        self.fed += new.len();
        self.stream.feed(&splashdown_audio::resample::resample(
            new,
            clip.sample_rate,
            splashdown_audio::TARGET_SAMPLE_RATE,
        ));
    }
}

/// A transform's result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransformOutcome {
    /// The new text, or the original when nothing changed.
    pub text: String,
    /// Why the text is as it was, when it is.
    pub unchanged: Option<String>,
}

pub struct Redo {
    pub raw: String,
    pub instruction: Option<String>,
    pub preset_id: String,
    pub reply: mpsc::Sender<Result<String, String>>,
}

/// Lets other threads (Settings commands) talk to the running session.
#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<Msg>,
    paused_until: Arc<Mutex<Option<Instant>>>,
}

/// Hotkeys are ignored for at most this long while Settings captures a new
/// key, so closing Settings mid-capture can't leave dictation switched off.
const PAUSE_LIMIT: Duration = Duration::from_secs(15);

impl Handle {
    pub fn reconfigure(&self, engine: Result<Engine, String>, keys: Keys) {
        let _ = self.tx.send(Msg::Reconfigure(Box::new(engine), keys));
    }

    pub fn pin_preset(&self, preset_id: Option<String>) {
        let _ = self.tx.send(Msg::PinPreset(preset_id));
    }

    /// "Undo AI edit" (⌃⌥Z or the menu bar).
    pub fn undo_ai_edit(&self) {
        let _ = self.tx.send(Msg::UndoAiEdit);
    }

    /// "Retry last dictation": transcribes the kept recording again.
    pub fn retry_last(&self) {
        let _ = self.tx.send(Msg::RetryLast);
    }

    /// A preset's shortcut was pressed.
    pub fn preset_shortcut(&self, preset_id: String) {
        let _ = self.tx.send(Msg::PresetShortcut(preset_id));
    }

    /// A transform's shortcut was pressed outside Notes.
    pub fn transform_shortcut(&self, id: String) {
        let _ = self.tx.send(Msg::TransformShortcut(id));
    }

    /// Runs a transform on `text` and waits for it (behind any dictation in
    /// progress), so call it from `spawn_blocking`.
    pub fn transform(&self, id: String, text: String) -> Result<TransformOutcome, String> {
        self.transform_with(id, text, None)
    }

    /// `transform`, with what the user asked for on top (or, with no `id`,
    /// only that).
    pub fn transform_with(
        &self,
        id: String,
        text: String,
        extra: Option<String>,
    ) -> Result<TransformOutcome, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::Transform(TransformRequest {
                id,
                text,
                extra,
                reply,
            }))
            .map_err(|_| "dictation isn't running".to_string())?;
        rx.recv()
            .map_err(|_| "dictation isn't running".to_string())?
    }

    /// The engine's speech-to-text, for transcribing a file. Waits behind
    /// any dictation in progress, so call it off the UI thread.
    pub fn file_speech(&self) -> Result<FileSpeech, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::FileSpeech(reply))
            .map_err(|_| "dictation isn't running".to_string())?;
        rx.recv()
            .map_err(|_| "dictation isn't running".to_string())?
    }

    /// The model that answers questions, or `None` without one. Waits
    /// behind any dictation in progress, so call it off the UI thread.
    pub fn answer_model(&self) -> Result<Option<AnswerModel>, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::AnswerModel(reply))
            .map_err(|_| "dictation isn't running".to_string())?;
        match rx.recv() {
            Ok(Ok(model)) => Ok(Some(model)),
            Ok(Err(e)) if e.is_empty() => Ok(None),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("dictation isn't running".to_string()),
        }
    }

    /// Ignore hotkeys (while Settings listens for a new key) or resume.
    pub fn pause_hotkeys(&self, paused: bool) {
        *self.paused_until.lock().unwrap_or_else(|e| e.into_inner()) =
            paused.then(|| Instant::now() + PAUSE_LIMIT);
    }

    /// Blocks until the session has re-run the AI pass (behind any
    /// dictation in progress), so call it from `spawn_blocking`.
    pub fn redo(
        &self,
        raw: String,
        instruction: Option<String>,
        preset_id: String,
    ) -> Result<String, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::Redo(Redo {
                raw,
                instruction,
                preset_id,
                reply,
            }))
            .map_err(|_| "dictation isn't running".to_string())?;
        rx.recv()
            .map_err(|_| "dictation isn't running".to_string())?
    }

    /// Runs a control request and waits for the answer: for `Listen`,
    /// until the user has spoken, so never call it on the UI thread.
    pub fn control(&self, request: Request) -> Result<Option<String>, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::Control(request, reply))
            .map_err(|_| "dictation isn't running".to_string())?;
        // A discarded recording (Esc, new settings) drops its reply.
        rx.recv().unwrap_or_else(|_| Err("Canceled".into()))
    }
}

pub fn spawn(ui: Ui, engine: Result<Engine, String>, keys: Keys, history: Arc<History>) -> Handle {
    let controller = Controller::new(ui, engine, keys, history);
    let handle = Handle {
        tx: controller.tx.clone(),
        paused_until: controller.paused_until.clone(),
    };
    std::thread::Builder::new()
        .name("splashdown-session".into())
        .spawn(move || controller.run())
        .expect("spawn session thread");
    handle
}

/// An insertion the AI changed: where it went and what was actually said.
struct AiEdit {
    pid: i32,
    at: Instant,
    said: String,
}

/// A recording in progress.
struct Active {
    /// Distinguishes this recording's preview results from older ones.
    session: u64,
    preview_due: Instant,
    preview_busy: bool,
    focus: Option<Focus>,
    selection: Selection,
    level: Level,
    /// Content recorded before the instruction-key tap.
    content: Option<AudioClip>,
    started: Instant,
    /// The last time the microphone level looked like speech.
    last_voice: Instant,
    /// The loudest the microphone has been in this recording (raw level).
    loudest: f32,
    /// The "microphone hears nothing" warning was shown.
    mic_warned: bool,
    /// The note key was pressed: the text goes into a note, not the field.
    capture: bool,
    /// The ask key was pressed: it's a question for the Ask panel.
    ask: bool,
    /// When to quiet other sounds (after the start sound has played).
    quiet_due: Option<Instant>,
    /// Other sounds, quieted while recording; dropping it restores them.
    quiet: Option<splashdown_platform::QuietOutput>,
    hands_free: bool,
    /// What the overlay shows for this recording, to restore after a
    /// cancel confirmation.
    shown: OverlayState,
    /// The preset in effect: matched from the focused app, or picked with a
    /// number key.
    preset: Preset,
    /// The text around the cursor when recording started, read only for a
    /// preset with field context on.
    field_text: Option<String>,
    /// The clipboard's text when recording started (presets with clipboard
    /// context on).
    clipboard: Option<String>,
    /// Transcribes a long dictation ahead, up to each pause. Plain
    /// dictation only: dropped when the instruction key is tapped.
    chunker: Option<Chunker>,
    /// Transcribes the recording as it's made, with a streaming model.
    /// Dropped when the instruction key is tapped.
    stream: Option<LiveStream>,
    /// The focused terminal's or editor's project, being listed in the
    /// background (presets with file tags on).
    project: Option<mpsc::Receiver<Option<Arc<Project>>>>,
    /// A control request's `listen`: the text goes here, not into a field.
    reply_to: Option<control::Reply>,
}

struct Controller {
    ui: Ui,
    engine: Result<Engine, String>,
    keys: Keys,
    recognizer: Arc<Mutex<Recognizer>>,
    audio: AudioWorker,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    listener: Option<splashdown_platform::KeyListener>,
    logged_listener_error: bool,
    active: Option<Active>,
    hide_at: Option<Instant>,
    preview: Option<PreviewWorker>,
    next_session: u64,
    /// The app we last inserted into, and when, so an undo or edit only
    /// touches our own recent insertion.
    last_insert: Option<(i32, Instant)>,
    /// The text of that insertion, for edits of "that".
    last_text: String,
    /// Recent dictations, sent with the next one into the same app.
    recent: crate::recent::Recent,
    /// The last insertion the AI changed, for "Undo AI edit".
    last_ai_edit: Option<AiEdit>,
    /// The last dictation added to a note, which "Undo AI edit" (⌃⌥Z)
    /// takes back too.
    last_capture: Option<(crate::notes::Captured, Instant)>,
    /// A spoken command found while preparing a recording (see `prepare`).
    pending_command: Option<AudioClip>,
    history: Arc<History>,
    correction_check: Option<PendingCorrectionCheck>,
    paused_until: Arc<Mutex<Option<Instant>>>,
    /// A preset chosen from the menu bar, overriding the per-app match.
    pinned_preset: Option<String>,
    /// The day the "near the limit" warning was last shown.
    budget_warned: Option<String>,
    /// The recording being delivered, to keep with its history entry.
    pending_audio: Option<(AudioClip, Option<AudioClip>)>,
    /// The last overlay notice, so a control request can say what failed.
    last_notice: Option<String>,
    /// The microphone went away mid-recording; the result says so.
    mic_lost: bool,
    /// Why the last pipeline run failed (not "no speech").
    pipeline_error: Option<String>,
    /// What the last voice action added, and when, for "undo that".
    last_action: Option<(crate::voice_actions::Undo, Instant)>,
    /// Dictations in a row the built-in model was slow to tidy up, and
    /// whether a cloud key was suggested (once per run).
    slow_local: u8,
    suggested_cloud: bool,
}

impl Controller {
    fn new(ui: Ui, engine: Result<Engine, String>, keys: Keys, history: Arc<History>) -> Self {
        let (tx, rx) = mpsc::channel();
        let preview = preview_worker(&engine, &tx);
        Self {
            recognizer: Arc::new(Mutex::new(
                Recognizer::new(keys.dictation, keys.instruction, Timing::default())
                    .with_hands_free(keys.hands_free, keys.double_tap)
                    .with_capture(keys.note)
                    .with_ask(keys.ask),
            )),
            ui,
            engine,
            keys,
            audio: AudioWorker::spawn(),
            tx,
            rx,
            listener: None,
            logged_listener_error: false,
            active: None,
            hide_at: None,
            preview,
            next_session: 0,
            last_insert: None,
            last_text: String::new(),
            recent: crate::recent::Recent::default(),
            last_ai_edit: None,
            last_capture: None,
            pending_command: None,
            history,
            correction_check: None,
            paused_until: Arc::default(),
            pinned_preset: None,
            budget_warned: None,
            pending_audio: None,
            last_notice: None,
            mic_lost: false,
            slow_local: 0,
            suggested_cloud: false,
            pipeline_error: None,
            last_action: None,
        }
    }

    fn run(mut self) {
        self.try_listen();
        self.refresh_tray();
        loop {
            let now = Instant::now();
            let mut wake = now + LISTENER_RETRY;
            if let Some(d) = self.lock().deadline() {
                wake = wake.min(d);
            }
            if let Some(h) = self.hide_at {
                wake = wake.min(h);
            }
            if self.active.is_some() {
                wake = wake.min(now + LEVEL_TICK);
            }

            match self.rx.recv_timeout(wake.saturating_duration_since(now)) {
                Ok(Msg::Action(action)) => self.handle(action),
                Ok(Msg::Reconfigure(engine, keys)) => self.reconfigure(*engine, keys),
                Ok(Msg::Partial(p)) => self.on_partial(p),
                Ok(Msg::PinPreset(p)) => {
                    self.pinned_preset = p;
                    self.refresh_tray();
                }
                Ok(Msg::Redo(r)) => {
                    let result = self.redo(&r);
                    let _ = r.reply.send(result);
                }
                Ok(Msg::Control(request, reply)) => self.control(request, reply),
                Ok(Msg::PresetShortcut(id)) => self.preset_shortcut(&id),
                Ok(Msg::UndoAiEdit) => self.undo_ai_edit(),
                Ok(Msg::RetryLast) => self.retry_last(),
                Ok(Msg::Transform(t)) => {
                    let result = self
                        .run_transform(&t.id, &t.text, t.extra.as_deref())
                        .map(|(_, out)| out);
                    let _ = t.reply.send(result);
                }
                Ok(Msg::TransformShortcut(id)) => self.transform_shortcut(&id),
                Ok(Msg::FileSpeech(reply)) => {
                    let speech =
                        self.engine
                            .as_ref()
                            .map_err(Clone::clone)
                            .map(|engine| FileSpeech {
                                stt: engine.pipeline.stt(),
                                hints: engine.hints.clone(),
                                cleanup: engine.audio_cleanup(),
                            });
                    let _ = reply.send(speech);
                }
                Ok(Msg::ActionDone(result)) => match result {
                    Ok(done) => {
                        crate::did::note(crate::did::ACTED, &done.text);
                        if let Some(undo) = done.undo {
                            self.last_action = Some((undo, Instant::now()));
                        }
                        self.done(&done.text);
                    }
                    Err(e) => self.notice(Tone::Warning, &short(&e)),
                },
                Ok(Msg::AnswerModel(reply)) => {
                    let model = self
                        .engine
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|engine| {
                            // An empty error: there's no model, which isn't a failure.
                            let model = engine.pipeline.answer_model().ok_or_else(String::new)?;
                            Ok(AnswerModel {
                                model,
                                user_name: engine.context.user_name.clone(),
                            })
                        });
                    let _ = reply.send(model);
                }
                Err(RecvTimeoutError::Timeout) => self.on_tick(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Recognizer> {
        self.recognizer.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn try_listen(&mut self) {
        if self.listener.is_some() {
            return;
        }
        let recognizer = self.recognizer.clone();
        let tx = self.tx.clone();
        // The session thread is busy while a dictation is processed, so the
        // auto-send badge is updated from here.
        let ui = self.ui.clone();
        let paused_until = self.paused_until.clone();
        let handler = Box::new(move |ev| {
            let paused = paused_until
                .lock()
                .is_ok_and(|p| p.is_some_and(|until| Instant::now() < until));
            if paused {
                return false;
            }
            let mut r = recognizer.lock().unwrap_or_else(|e| e.into_inner());
            let (swallow, action) = r.on_event(ev, Instant::now());
            if let Some(Action::ToggleSend(on)) = action {
                ui.set_send(on);
            } else if let Some(a) = action {
                let _ = tx.send(Msg::Action(a));
            }
            swallow
        });
        match splashdown_platform::listen_keys(handler) {
            Ok(l) => {
                tracing::info!("listening for {}", self.keys.dictation.label());
                self.listener = Some(l);
                self.update_status();
            }
            Err(e) => {
                self.ui
                    .status("Needs Accessibility permission — open Settings");
                // Retried every couple of seconds; only log once. Settings
                // asks for the permission (it opens by itself at launch).
                if !self.logged_listener_error {
                    tracing::info!("key listener not started: {e}");
                    self.logged_listener_error = true;
                }
            }
        }
    }

    /// Menu bar icon and preset menu for the current state.
    fn refresh_tray(&self) {
        let ready = self.listener.is_some() && self.engine.is_ok();
        self.ui.tray_icon(match (self.active.is_some(), ready) {
            (true, _) => TrayIconState::Recording,
            (false, true) => TrayIconState::Idle,
            (false, false) => TrayIconState::Attention,
        });
        let presets = self
            .engine
            .as_ref()
            .map(|e| e.presets.as_slice())
            .unwrap_or(&[]);
        self.ui.tray_presets(presets, self.pinned_preset.as_deref());
    }

    fn update_status(&self) {
        if self.listener.is_none() {
            return;
        }
        self.ui.status(&match &self.engine {
            Ok(_) => format!("Ready · hold {} to dictate", self.keys.dictation.label()),
            Err(e) => format!("Not set up: {e}"),
        });
    }

    fn reconfigure(&mut self, engine: Result<Engine, String>, keys: Keys) {
        if self.active.take().is_some() {
            self.audio.stop();
            self.ui.hide();
        }
        *self.lock() = Recognizer::new(keys.dictation, keys.instruction, Timing::default())
            .with_hands_free(keys.hands_free, keys.double_tap)
            .with_capture(keys.note)
            .with_ask(keys.ask);
        self.preview = preview_worker(&engine, &self.tx);
        self.engine = engine;
        self.keys = keys;
        // Dictated under the old settings, which may not have allowed it to
        // go where the new ones send it.
        self.recent.clear();
        if let Ok(e) = &self.engine
            && self
                .pinned_preset
                .as_ref()
                .is_some_and(|id| !e.presets.iter().any(|p| &p.id == id))
        {
            self.pinned_preset = None;
        }
        self.refresh_tray();
        match &self.engine {
            Ok(_) => tracing::info!("settings applied"),
            Err(e) => tracing::warn!("dictation isn't set up: {e}"),
        }
        self.update_status();
    }

    fn on_tick(&mut self) {
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
        if self.correction_check.as_ref().is_some_and(|c| now >= c.due) {
            self.check_corrections();
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

    fn handle(&mut self, action: Action) {
        match action {
            Action::Start => {
                self.start();
                self.refresh_tray();
            }
            Action::MarkInstruction => {
                if let Some(active) = &mut self.active
                    && active.content.is_none()
                    && !matches!(active.selection, Selection::Text(_))
                {
                    active.content = Some(self.audio.take());
                    active.chunker = None;
                    active.stream = None;
                    // The preview now follows the instruction.
                    active.preview_due = Instant::now() + PREVIEW_START;
                    active.shown = OverlayState::Instruction;
                    self.ui.show(OverlayState::Instruction);
                }
            }
            Action::Capture => self.start_capture(),
            Action::Ask => self.start_ask(),
            Action::HandsFree => {
                if let Some(active) = &mut self.active {
                    active.hands_free = true;
                    active.shown = OverlayState::HandsFree {
                        preset: preset_label(&active.preset),
                    };
                    self.ui.show(active.shown.clone());
                }
            }
            Action::ConfirmCancel { seconds } => {
                if self.active.is_some() {
                    self.ui.show(OverlayState::Message {
                        tone: Tone::Warning,
                        text: format!("Press Esc again to discard {seconds} s of recording"),
                    });
                }
            }
            Action::SelectPreset(n) => self.select_preset(n),
            // Handled on the key thread; `finish` reads it.
            Action::ToggleSend(_) => {}
            Action::KeepRecording => {
                if let Some(active) = &self.active {
                    self.ui.show(active.shown.clone());
                }
            }
            Action::Cancel { silent } => {
                if self.active.take().is_some() {
                    self.refresh_tray();
                    self.audio.stop();
                    if silent {
                        self.ui.hide();
                    } else {
                        self.notice(Tone::Info, "Canceled");
                    }
                }
            }
            Action::Finish => {
                if let Some(active) = self.active.take() {
                    self.refresh_tray();
                    self.finish(active);
                }
            }
        }
    }

    fn start(&mut self) {
        self.mic_lost = false;
        // The user has moved on: look at the previous insertion now.
        if self.correction_check.is_some() {
            self.check_corrections();
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
            project,
        });
        if auto_send {
            self.lock().set_send(true);
            self.ui.set_send(true);
        }
    }

    /// Sends the audio so far to the preview model when a pass is due.
    fn maybe_preview(&mut self, now: Instant) {
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

    fn on_partial(&mut self, partial: preview::Partial) {
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

    /// Recording ended: build the job, run the pipeline, deliver the text.
    /// Runs on the session thread and blocks it until done (see
    /// docs/ARCHITECTURE.md, "Threads").
    fn finish(&mut self, mut active: Active) {
        if let Some(reply) = active.reply_to.take() {
            self.answer(active, reply);
            return;
        }
        if active.ask {
            self.ask(active);
            return;
        }
        // Only an error from this dictation's own run counts below.
        self.pipeline_error = None;
        let spoke = active.started.elapsed();
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
        let mut typer = job
            .as_ref()
            .and_then(|_| self.stream_typer(&active, files.clone()));
        let focus = active.focus.as_ref();
        let mut on_text = typer
            .as_mut()
            .zip(focus)
            .map(|(typer, focus)| move |so_far: &str| typer.offer(focus, so_far));
        let on_text = on_text.as_mut().map(|f| f as &mut OnText<'_>);
        let outcome = job
            .map(|job| with_file_names(job, files.as_deref()))
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
        if let Some((outcome, took)) = outcome {
            crate::spend::record_outcome(&outcome);
            self.pending_audio = recording;
            let typer = typer.filter(StreamTyper::started);
            self.deliver(active, outcome, took, send, spoke, typer);
            self.check_budget();
        }
    }

    /// Types the AI's text as it streams in, when that's on and the
    /// dictation goes into a field (not a note).
    fn stream_typer(&self, active: &Active, files: Option<Arc<Project>>) -> Option<StreamTyper> {
        let engine = self.engine.as_ref().ok()?;
        if !engine.stream_typing || active.capture {
            return None;
        }
        let focus = active.focus.as_ref().filter(|f| f.is_current())?;
        StreamTyper::new(focus, files, engine.replacements.clone())
    }

    /// After spending: warn once a period nears its limit, and switch to
    /// on-device models when it's reached. At the start of a recording,
    /// also notices a new day or month lifting the limit.
    fn check_budget(&mut self) {
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

    /// Stops recording and turns it into a pipeline job, or shows why not.
    fn prepare(&mut self, active: &mut Active) -> Option<AudioJob> {
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
                self.notice(Tone::Warning, MIC_SILENT_NOTICE);
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

    /// Runs the pipeline (blocking this thread), or shows what went wrong.
    /// The AI's text goes to `on_text` as it streams in.
    fn run_pipeline(
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

    /// Inserts the result into the field it was dictated into, records it,
    /// and presses Return if asked.
    fn deliver(
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
            }
        }
        if let Err(e) = inserted {
            tracing::warn!("insert into {app_id} failed: {e}");
            let _ = splashdown_platform::copy_to_clipboard(&outcome.text);
            self.notice(Tone::Warning, "Couldn't type it — copied to clipboard");
            return;
        }

        tracing::info!(
            "inserted {} characters into {app_id} ({method:?})",
            outcome.text.chars().count()
        );
        self.last_insert = target_pid.map(|pid| (pid, Instant::now()));
        self.last_text = outcome.text.clone();
        self.last_capture = None;
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
        let will_send = send;
        self.last_ai_edit = target_pid
            .filter(|_| ai_changed && method == InsertMethod::Paste && !will_send)
            .map(|pid| AiEdit {
                pid,
                at: Instant::now(),
                // The undone paste included the space, so this does too.
                said: if space {
                    format!(" {}", outcome.raw)
                } else {
                    outcome.raw.clone()
                },
            });
        if engine_learns(&self.engine)
            && let Some(focus) = active.focus
        {
            self.correction_check = Some(PendingCorrectionCheck {
                focus,
                inserted: outcome.text.clone(),
                due: Instant::now() + CORRECTION_CHECK_AFTER,
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
        let mut status = inserted_status(
            took,
            sent,
            outcome.fallback.as_ref(),
            outcome.llm_name.as_deref(),
        );
        if outcome.usage.on_device_fallback {
            tracing::warn!("a cloud provider failed; a model on this computer stood in");
            status.push_str(" · done on this computer (cloud didn't answer)");
        } else if self.local_is_slow(&outcome) {
            status = "Inserted · tidying up is slow here: a cloud key is faster".into();
        }
        self.done(&status);
    }

    /// Whether the built-in model has now been slow several dictations in a
    /// row, on a computer where a cloud key hasn't been suggested yet (and
    /// the user hasn't chosen to keep it anyway). Says so once per run.
    fn local_is_slow(&mut self, outcome: &splashdown_core::Outcome) -> bool {
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
        let keep = tauri::Manager::try_state::<crate::commands::AppState>(self.ui.app())
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

    fn record(
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

    fn match_preset(&self, focus: Option<&Focus>) -> Preset {
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

    fn redo(&self, r: &Redo) -> Result<String, String> {
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

    /// The note key while recording: the dictation will go into a note.
    fn start_capture(&mut self) {
        let Ok(notes) = self.notes_settings() else {
            return;
        };
        let label = format!("Note · {}", crate::notes::capture_label(&notes));
        let Some(active) = &mut self.active else {
            return;
        };
        active.capture = true;
        active.ask = false;
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
    fn start_ask(&mut self) {
        let Some(active) = &mut self.active else {
            return;
        };
        active.ask = true;
        active.capture = false;
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

    /// `finish` for a question: transcribes it and opens the Ask panel,
    /// which answers it in the background.
    fn ask(&mut self, mut active: Active) {
        let job = self.prepare(&mut active);
        let _ = self.lock().take_send();
        let Some(mut job) = job else {
            // The instruction key came before anything was said: what came
            // after it is the question.
            if let Some(clip) = self.pending_command.take() {
                self.ask_from(&active, &clip);
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
        self.show_answer(&active, question, selection);
        self.check_budget();
    }

    /// A question recorded on its own (after the instruction key).
    fn ask_from(&mut self, active: &Active, clip: &AudioClip) {
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
                self.show_answer(active, question.trim().to_string(), None);
            }
            Ok(_) => self.notice(Tone::Info, "Didn't catch that"),
            Err(e) => {
                let message = errors::pipeline(&e, Some(&stt), None);
                self.notice(Tone::Warning, &short(&message));
            }
        }
    }

    /// Opens the Ask panel on `question`, asked from `active`'s app.
    fn show_answer(&mut self, active: &Active, question: String, selection: Option<String>) {
        if splashdown_core::action::asks_what_to_say(&question) {
            return self.what_to_say();
        }
        let Ok(engine) = &self.engine else {
            return;
        };
        let Some(model) = engine.pipeline_for(&active.preset).answer_model() else {
            return self.notice(
                Tone::Warning,
                "Asking needs an AI model: choose one in Settings → Providers",
            );
        };
        let notes = self
            .notes_settings()
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
            today: chrono::Local::now().format("%A, %-d %B %Y").to_string(),
            earlier: Vec::new(),
            cite_numbers: false,
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
            },
        );
        self.ui.hide();
    }

    /// A captured dictation: added to the note instead of the field.
    /// ⌃⌥Z takes it back while it's recent.
    fn deliver_capture(
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

    /// Runs transform `id` on `text` with the AI, with `extra` (what the
    /// user asked for) on top; with no `id`, `extra` is the instruction.
    /// Returns its name and the result.
    fn run_transform(
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
        .map_err(|e| {
            let llm = pipeline
                .instruction_llm_name()
                .or_else(|| pipeline.llm_name());
            errors::pipeline(&e, None, llm.as_deref())
        })?;
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
    fn transform_shortcut(&mut self, id: &str) {
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
        if !focus.is_current() {
            let _ = splashdown_platform::copy_to_clipboard(&out.text);
            return self.notice(Tone::Warning, "Focus changed — copied to clipboard");
        }
        let result = splashdown_platform::insert_text(&out.text, InsertMethod::Paste);
        if result.is_ok() {
            self.last_insert = Some((focus.app.pid, Instant::now()));
            self.last_text = out.text;
        }
        self.finish_command(result, &name);
    }

    /// Number key while recording: switch to preset `n` (1-based).
    fn select_preset(&mut self, n: u8) {
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
    fn use_preset(&mut self, preset: Preset) {
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

    /// Compares the field now with what we inserted and records likely
    /// vocabulary corrections. Only the suggested words are kept.
    fn check_corrections(&mut self) {
        let Some(check) = self.correction_check.take() else {
            return;
        };
        let Ok(engine) = &self.engine else {
            return;
        };
        let Some(now) = check.focus.field_text() else {
            return;
        };
        let words =
            splashdown_core::vocab::suggest(&check.inserted, &now, &engine.context.vocabulary);
        if !words.is_empty() {
            tracing::info!("vocabulary suggestions: {}", words.len());
            self.history.add_suggestions(words);
        }
    }

    /// Undoes our last insertion (a spoken command) if it was recent and into the
    /// app that has focus now. One paste is a single undo step.
    fn undo_last(&mut self, target_pid: Option<i32>, still_focused: bool) {
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

    /// Replaces the last AI-edited insertion with what was said: undo the
    /// paste, then paste the transcript. Only right after it, in the same
    /// app, like other edits of "that".
    /// "Retry last dictation": runs the kept recording again with the
    /// preset it was dictated with, and puts the text on the clipboard (the
    /// field it was meant for may be gone).
    fn retry_last(&mut self) {
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

    fn undo_ai_edit(&mut self) {
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

    /// A spoken command (the instruction key with nothing said before
    /// it): transcribe it, let the AI say what it means, and do that.
    fn run_command(&mut self, active: &Active, clip: &AudioClip) {
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
                let llm = pipeline
                    .instruction_llm_name()
                    .or_else(|| pipeline.llm_name());
                let message = errors::pipeline(&e, None, llm.as_deref());
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
            self.show_answer(active, question, None);
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
    fn carry_out(&mut self, command: Command, focus: Option<&Focus>, target_pid: Option<i32>) {
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
            Command::Write(text) => {
                let result = paste(&text);
                if result.is_ok() {
                    self.last_insert = target_pid.map(|pid| (pid, Instant::now()));
                    self.last_text = text;
                }
                return self.finish_command(result, "Inserted");
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
    fn do_action(&mut self, active: &Active, request: &str, selection: Option<String>) {
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
        let now = chrono::Local::now().naive_local();
        let ctx = splashdown_core::action::PlanContext {
            now: now.format("%A %Y-%m-%d %H:%M").to_string(),
            coming_days: (1..=7)
                .map(|d| {
                    (now.date() + chrono::Days::new(d))
                        .format("%A %Y-%m-%d")
                        .to_string()
                })
                .collect(),
            selection,
            shortcuts: if enabled.contains(&"run_shortcut") {
                crate::voice_actions::shortcuts()
            } else {
                Vec::new()
            },
            presets: engine.presets.iter().map(|p| p.name.clone()).collect(),
            mine: crate::my_actions::for_planner(&engine.actions),
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
                let llm = pipeline
                    .instruction_llm_name()
                    .or_else(|| pipeline.llm_name());
                let message = errors::pipeline(&e, None, llm.as_deref());
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
    fn own_action(&mut self, what: &str, preset: Option<&str>) -> bool {
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
    fn what_to_say(&mut self) {
        let app = self.ui.app().clone();
        let _ = self
            .ui
            .app()
            .run_on_main_thread(move || crate::help::open(&app));
        self.ui.hide();
    }

    fn finish_command(&mut self, result: splashdown_platform::Result<()>, done: &str) {
        match result {
            Ok(()) => {
                crate::did::note(crate::did::EDITED, done);
                self.done(done);
            }
            Err(e) => self.notice(Tone::Warning, &short(&e.to_string())),
        }
    }

    fn done(&mut self, text: &str) {
        if std::mem::take(&mut self.mic_lost) {
            self.notice(Tone::Warning, &format!("Microphone disconnected · {text}"));
            return;
        }
        self.ui.show(OverlayState::Message {
            tone: Tone::Success,
            text: text.to_string(),
        });
        self.hide_at = Some(Instant::now() + SHOW_SUCCESS);
    }

    fn notice(&mut self, tone: Tone, text: &str) {
        let text = &if std::mem::take(&mut self.mic_lost) {
            format!("Microphone disconnected · {text}")
        } else {
            text.to_string()
        };
        self.last_notice = Some(text.to_string());
        self.ui.show(OverlayState::Message {
            tone,
            text: text.to_string(),
        });
        self.hide_at = Some(Instant::now() + SHOW_NOTICE);
    }
}

/// A plan, for the log: the action's id, never what it was asked to do.
fn plan_label(plan: &splashdown_core::action::Plan) -> &'static str {
    match plan {
        splashdown_core::action::Plan::Do(action) => action.id(),
        splashdown_core::action::Plan::Nothing(_) => "none",
        splashdown_core::action::Plan::Invalid(_) => "refused",
    }
}

fn preview_worker(
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

/// Splits a finished recording into what was said and what it's for.
/// With text selected, everything is an instruction for it (Transform);
/// after an instruction-key tap, speech before it is content and after it
/// is the instruction; otherwise it's plain dictation, inserted as heard
/// when the preset has the AI pass off.
fn split_recording(
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

/// How long to wait for a paste to show in a field's text.
const PASTE_CHECK: Duration = Duration::from_millis(800);

/// Whether the field's text moved on from `before` within `PASTE_CHECK`.
fn field_changed(focus: &Focus, before: &str) -> bool {
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

/// The stream for a new recording, if the user's speech model can
/// transcribe while they speak: its text is the dictation's. Not with noise
/// reduction or whisper mode, which work on the whole recording.
fn live_stream(engine: &Engine, preset: &Preset) -> Option<LiveStream> {
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

/// Where the last dictation sits in a field's text: just before the cursor
/// (spaces between them allowed). Byte offsets.
fn last_dictation_range(text: &str, cursor: usize, last: &str) -> Option<(usize, usize)> {
    let last = last.trim();
    let before = text.get(..cursor)?.trim_end();
    if last.is_empty() || !before.ends_with(last) {
        return None;
    }
    Some((before.len() - last.len(), before.len()))
}

/// Most of the clipboard's text sent as context; more is cut.
const CLIPBOARD_CHARS: usize = 2000;

/// The clipboard's text for a preset with clipboard context on.
fn read_clipboard() -> Option<String> {
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
fn list_project(document: std::path::PathBuf) -> mpsc::Receiver<Option<Arc<Project>>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(project::for_document(&document));
    });
    rx
}

/// The project's files, if the listing finished in time.
fn project_files(rx: mpsc::Receiver<Option<Arc<Project>>>) -> Option<Arc<Project>> {
    rx.recv_timeout(Duration::from_millis(400)).ok().flatten()
}

/// Project terms added to the vocabulary, at most. The user's own words
/// come first: speech models only read so much of the hint.
const PROJECT_TERMS: usize = 40;

/// Adds the project's recently changed file names and its most-used code
/// names to what speech-to-text and the AI are told to spell.
fn with_file_names(mut job: AudioJob, project: Option<&Project>) -> AudioJob {
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

/// The engine's prompt context with this dictation's app, preset and (for
/// presets with field context on) the text around the cursor.
fn prompt_context(
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
const CONTEXT_BEFORE: usize = 1000;
const CONTEXT_AFTER: usize = 200;

fn read_field_context(focus: &Focus) -> Option<String> {
    focus.text_around_cursor(CONTEXT_BEFORE, CONTEXT_AFTER)
}

/// The overlay message after a successful insertion.
fn inserted_status(
    took: Duration,
    sent: bool,
    fallback: Option<&Fallback>,
    llm_name: Option<&str>,
) -> String {
    let mut text = format!("Inserted · {:.1} s", took.as_secs_f32());
    if sent {
        text.push_str(" · sent");
    }
    match fallback {
        Some(Fallback::Guard(_)) => text.push_str(" · raw text (AI output looked wrong)"),
        Some(Fallback::ModelError(e)) => {
            let reason = errors::fallback_reason(e, llm_name);
            text.push_str(&format!(" · raw text ({reason})"));
        }
        None => {}
    }
    text
}

fn engine_learns(engine: &Result<Engine, String>) -> bool {
    engine.as_ref().is_ok_and(|e| e.learn_vocabulary)
}

/// "https://mail.google.com/mail/u/0" → "mail.google.com".
fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next()?.trim();
    (!host.is_empty()).then(|| host.to_string())
}

/// The overlay names a preset unless it's the default one.
fn preset_label(preset: &Preset) -> Option<String> {
    (preset.id != DEFAULT_PRESET_ID && !preset.name.is_empty()).then(|| preset.name.clone())
}

/// Joins two clips recorded back to back.
fn concat(mut a: AudioClip, b: AudioClip) -> AudioClip {
    a.samples.extend(b.samples);
    a
}

/// Keeps overlay messages to a glanceable length.
fn short(s: &str) -> String {
    const MAX: usize = 70;
    match s.char_indices().nth(MAX) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use splashdown_core::guard::GuardReason;

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

    #[test]
    fn inserted_status_mentions_send_and_fallbacks() {
        let took = Duration::from_millis(1240);
        assert_eq!(inserted_status(took, false, None, None), "Inserted · 1.2 s");
        assert_eq!(
            inserted_status(took, true, None, None),
            "Inserted · 1.2 s · sent"
        );
        assert_eq!(
            inserted_status(
                took,
                false,
                Some(&Fallback::Guard(GuardReason::AnsweredQuestion)),
                None
            ),
            "Inserted · 1.2 s · raw text (AI output looked wrong)"
        );
        assert_eq!(
            inserted_status(
                took,
                false,
                Some(&Fallback::ModelError("network error".into())),
                Some("openrouter/x")
            ),
            "Inserted · 1.2 s · raw text (offline?)"
        );
    }

    /// Records how much audio each `feed` got.
    struct CountingStream(Arc<Mutex<Vec<usize>>>);

    impl splashdown_core::SpeechStream for CountingStream {
        fn feed(&mut self, samples: &[f32]) {
            self.0.lock().unwrap().push(samples.len());
        }
        fn partial(&mut self) -> Option<String> {
            None
        }
        fn finish(self: Box<Self>) -> splashdown_core::ProviderResult<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn a_stream_gets_each_sample_once() {
        let fed = Arc::new(Mutex::new(Vec::new()));
        let mut live = LiveStream {
            stream: Box::new(CountingStream(fed.clone())),
            fed: 0,
            show: false,
        };
        let clip = |n| AudioClip::new(vec![0.0; n], 16_000);
        // While recording: what's new since the last feed.
        live.feed(&clip(1600), 0);
        live.feed(&clip(800), 1600);
        // On release: the whole recording, of which only the end is new.
        live.feed(&clip(3200), 0);
        live.feed(&clip(3200), 0);
        assert_eq!(*fed.lock().unwrap(), [1600, 800, 800]);
    }
}
