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
//! This file holds the controller's state, its message loop (`run`,
//! `handle`) and the overlay messages (`notice`, `done`). The rest is one
//! `impl Controller` per job, in `session/`:
//!
//! | File | What it does |
//! |---|---|
//! | `recording.rs` | Starting a recording; the preset, live preview, and the note, ask and compose letters while the key is held |
//! | `job.rs` | Turning the recording into a pipeline job, with its context |
//! | `deliver.rs` | Running the pipeline and inserting the text (or adding it to a note) |
//! | `assist.rs` | Ask, Compose and reading aloud |
//! | `spoken.rs` | Spoken commands and voice actions |
//! | `edits.rs` | Redo, transforms on selected text, retry and undo |
//! | `corrections.rs` | Watching for corrected names and learning them |
//! | `control.rs` | Requests from the control channel (`splashdown-cli`, links, agents) |

mod assist;
mod control;
mod corrections;
mod deliver;
mod edits;
mod job;
mod recording;
mod spoken;

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

use self::assist::*;
use self::corrections::*;
use self::job::*;
use self::recording::*;
use self::spoken::*;

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
/// At most this much text is read aloud, or summarized to be: about 45
/// minutes of listening.
const READ_CHARS: usize = 40_000;
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
    /// Pressed while holding the dictation key: the dictation says what to
    /// write, and Compose writes it. `Other` when there's no compose letter
    /// (the default: the instruction key asks for drafts too).
    pub compose: PhysicalKey,
    /// Pressed while holding the dictation key: the dictation, or the
    /// selection, is translated.
    pub translate: PhysicalKey,
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
    /// ⌃⌥T: translate the selection.
    Translate,
    /// The speech-to-text for transcribing a file.
    FileSpeech(mpsc::Sender<Result<FileSpeech, String>>),
    /// The model that answers questions, for Ask in Notes.
    AnswerModel(mpsc::Sender<Result<AnswerModel, String>>),
    /// A voice action finished (`voice_actions`): what to show, and what
    /// "undo that" would take back.
    ActionDone(Result<crate::voice_actions::Done, String>),
    /// A draft written for the field (`show_draft`), and the app it's for.
    Composed(Result<String, String>, Option<i32>),
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

    /// ⌃⌥T was pressed: translate the selection.
    pub fn translate(&self) {
        let _ = self.tx.send(Msg::Translate);
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
    /// The compose key was pressed: it says what to write, for Compose.
    compose: bool,
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
    /// Names and terms in the window around the field (and the
    /// conversation's language, when it's wanted), being read in the
    /// background, for speech-to-text and the AI to spell.
    screen_words: Option<mpsc::Receiver<Screen>>,
    /// What that read found, once it's in.
    screen: Option<Screen>,
    /// The preset before this dictation was made to translate (the
    /// translate key, or the conversation's language), so another
    /// language can be put in its place.
    untranslated: Option<Preset>,
    /// A control request's `listen`: the text goes here, not into a field.
    reply_to: Option<control::Reply>,
    /// …and where it ends up, for the overlay ("Sent to Shortcuts").
    answer_to: Option<String>,
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
                    .with_ask(keys.ask)
                    .with_compose(keys.compose)
                    .with_translate(keys.translate),
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
            if let Some(check) = &self.correction_check {
                wake = wake.min(check.due);
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
                Ok(Msg::Translate) => self.translate_shortcut(),
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
                Ok(Msg::Composed(result, pid)) => self.composed(result, pid),
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
        let handler = Box::new(move |ev: splashdown_platform::KeyEvent| {
            let paused = paused_until
                .lock()
                .is_ok_and(|p| p.is_some_and(|until| Instant::now() < until));
            if paused {
                return false;
            }
            let mut r = recognizer.lock().unwrap_or_else(|e| e.into_inner());
            // Esc stops a reading aloud, unless it's for a recording.
            if ev.key == PhysicalKey::Escape
                && !r.is_recording()
                && crate::reading::on_escape(ev.down)
            {
                return true;
            }
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
            .with_ask(keys.ask)
            .with_compose(keys.compose)
            .with_translate(keys.translate);
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

    fn handle(&mut self, action: Action) {
        match action {
            Action::Start => {
                // The app mustn't hear itself, and a new dictation means
                // the user has moved on.
                crate::reading::stop();
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
            Action::Compose => self.start_compose(),
            Action::Translate => self.translate_key(),
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

/// The first `chars` characters of `text`.
fn head_chars(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => text[..at].to_string(),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
