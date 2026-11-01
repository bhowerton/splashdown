//! Meeting notes: a call recorded as two sides, **You** (the microphone)
//! and **Them** (what the computer plays), transcribed a turn at a time into
//! a note in `Meetings/` while it goes, and summarized when it ends.
//!
//! The audio is spooled to the config folder as it's recorded, so a crash
//! or a forced quit loses nothing: the next launch offers to finish the
//! transcript. The spool is deleted once the note is done.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use splashdown_audio::file::{TurnCutter, timestamp};
use splashdown_audio::live::LiveAudio;
use splashdown_config::{Destination, MeetingSummary, Secrets, Settings};
use splashdown_core::meeting::{Meeting, Paragraph, Segment, Side};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};

use crate::commands::AppState;
use crate::notes::store::{Origin, Store};

/// Start or stop a meeting; shown in the menu, registered in `shortcuts.rs`.
pub const ACCELERATOR: &str = "Ctrl+Alt+M";
/// App event to the Notes window with the meeting in progress (`Status`),
/// or null when there's none.
const STATUS: &str = "meeting-status";
/// App event to the Notes window with something to tell the user.
const MESSAGE: &str = "meeting-message";

/// How often the recorded audio is taken and cut into turns.
const TICK: Duration = Duration::from_millis(500);
/// With nobody speaking this long, the meeting asks whether it's still on...
const QUIET: Duration = Duration::from_secs(10 * 60);
/// ...and stops this long after asking.
const GRACE: Duration = Duration::from_secs(60);
/// The longest a meeting records.
const LONGEST: Duration = Duration::from_secs(4 * 3600);
const RATE: usize = splashdown_audio::TARGET_SAMPLE_RATE as usize;

/// The meeting in progress, for the Notes window and the CLI.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    /// The note it's written to.
    pub note: String,
    /// Seconds recorded.
    pub elapsed: f64,
    /// Input levels (0–1).
    pub you: f32,
    pub them: f32,
    /// Why the other side isn't being recorded, if it isn't.
    pub them_problem: Option<String>,
    /// "on this computer", "with OpenAI".
    pub place: String,
    /// "Still in a meeting?", with Keep recording.
    pub warning: Option<String>,
    /// Stopped; the last turns and the summary are being done.
    pub finishing: bool,
}

#[derive(Default)]
struct Running {
    stop: Arc<AtomicBool>,
    /// The user answered "Keep recording" to a warning.
    keep: Arc<AtomicBool>,
    status: Status,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn running() -> MutexGuard<'static, Option<Running>> {
    static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
    RUNNING.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_running() -> bool {
    running().is_some()
}

/// The meeting in progress, if any.
pub fn status() -> Option<Status> {
    running().as_ref().map(|r| r.status.clone())
}

/// The shortcut, the menu item and Notes' button: start a meeting, or stop
/// the one in progress.
pub fn toggle(app: &AppHandle) {
    let result = if is_running() {
        stop().map(|()| String::new())
    } else {
        start(app)
    };
    if let Err(e) = result {
        tell(app, e, true);
    }
}

/// Starts a meeting. Returns at once; the recording starts on its own
/// thread (asking for speech-to-text waits behind any dictation).
pub fn start(app: &AppHandle) -> Result<String, String> {
    let mut slot = running();
    if slot.is_some() {
        return Err("A meeting is already recording.".into());
    }
    let run = Running::default();
    let (stop, keep) = (run.stop.clone(), run.keep.clone());
    *slot = Some(run);
    drop(slot);
    let handle = app.clone();
    let thread = std::thread::Builder::new()
        .name("meeting".into())
        .spawn(move || {
            // A panic mustn't leave a meeting marked as recording for good.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                record(&handle, &stop, &keep)
            }))
            .unwrap_or_else(|_| Err("the recording stopped unexpectedly".into()));
            if let Err(e) = result {
                tracing::warn!("meeting: {e}");
                tell(&handle, e, true);
            }
            *running() = None;
            show(&handle, None);
        })
        .map_err(|e| {
            *running() = None;
            e.to_string()
        })?;
    if let Some(run) = running().as_mut() {
        run.thread = Some(thread);
    }
    Ok("Starting a meeting".into())
}

/// Stops the meeting; its note is finished in the background.
pub fn stop() -> Result<(), String> {
    match running().as_ref() {
        Some(run) => {
            run.stop.store(true, Ordering::Relaxed);
            Ok(())
        }
        None => Err("No meeting is recording.".into()),
    }
}

/// "Keep recording" after "Still in a meeting?".
pub fn keep_going() {
    if let Some(run) = running().as_ref() {
        run.keep.store(true, Ordering::Relaxed);
    }
}

/// Quit while a meeting records: ask, then stop it, finish the note and
/// quit, or carry on.
pub fn quit(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let answer = app
            .dialog()
            .message("Stop it and finish the note before quitting?")
            .title("A meeting is recording")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Stop and quit".into(),
                "Keep recording".into(),
            ))
            .blocking_show_with_result();
        let stop_and_quit = answer == MessageDialogResult::Ok
            || matches!(&answer, MessageDialogResult::Custom(s) if s == "Stop and quit");
        if !stop_and_quit {
            return;
        }
        let thread = running().as_mut().and_then(|run| {
            run.stop.store(true, Ordering::Relaxed);
            run.thread.take()
        });
        if let Some(thread) = thread {
            let _ = thread.join();
        }
        splashdown_platform::restore_output();
        app.exit(0);
    });
}

fn tell(app: &AppHandle, text: String, error: bool) {
    #[derive(Clone, Serialize)]
    struct Message {
        text: String,
        error: bool,
    }
    crate::notes::open(app);
    let _ = app.emit_to(crate::notes::WINDOW, MESSAGE, Message { text, error });
}

/// Updates the menu bar, the Notes window and the meeting pill.
fn show(app: &AppHandle, status: Option<&Status>) {
    if let Some(run) = running().as_mut()
        && let Some(status) = status
    {
        run.status = status.clone();
    }
    let elapsed = status
        .filter(|s| !s.finishing)
        .map(|s| timestamp(Duration::from_secs_f64(s.elapsed)));
    crate::tray::show_meeting(app, elapsed.as_deref());
    crate::pill::update(app, status);
    let _ = app.emit_to(crate::notes::WINDOW, STATUS, status);
}

/// What a spool file is for, next to it as JSON.
#[derive(Debug, Serialize, Deserialize)]
struct Spool {
    note: String,
    /// The notes folder the note is in.
    folder: PathBuf,
    /// When the meeting started (RFC 3339).
    started: String,
    /// Seconds of the recording whose turns are in the note.
    #[serde(default)]
    written: f64,
}

fn spool_dir() -> Result<PathBuf, String> {
    Ok(crate::engine::paths()?.config_dir.join("meetings"))
}

/// Where a meeting comes from, for its title: a call app in front when it
/// started.
fn call_app() -> (Option<String>, String) {
    let Some(focus) = splashdown_platform::Focus::capture() else {
        return (None, "Meeting".into());
    };
    let app = &focus.app;
    let host = app.url.as_deref().unwrap_or_default();
    let name = app.name.to_lowercase();
    let label = if host.contains("meet.google.com") {
        Some("Google Meet")
    } else if name.contains("zoom") {
        Some("Zoom")
    } else if name.contains("teams") {
        Some("Teams")
    } else if name.contains("slack") {
        Some("Slack")
    } else if name.contains("facetime") {
        Some("FaceTime")
    } else if name.contains("webex") {
        Some("Webex")
    } else if name.contains("discord") {
        Some("Discord")
    } else {
        None
    };
    match label {
        Some(label) => (Some(app.name.clone()), format!("{label} meeting")),
        None => (None, "Meeting".into()),
    }
}

/// Records a meeting until it's stopped, then finishes its note.
fn record(app: &AppHandle, stop: &AtomicBool, keep: &AtomicBool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state.settings();
    if !settings.notes.meeting_reminded {
        remind(app);
        settings = state.settings();
        settings.notes.meeting_reminded = true;
        if let Err(e) = settings.save(&crate::engine::paths()?.config_file()) {
            tracing::warn!("couldn't save the meeting reminder: {e}");
        }
        *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings.clone();
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
    }
    let speech = state.session.file_speech()?;
    let (place, _) = crate::notes::files::place(&speech.stt.name());

    let mut you = LiveAudio::microphone(
        splashdown_audio::devices::choose(
            &settings.general.microphones(),
            &splashdown_audio::devices::inputs(),
        )
        .as_deref(),
    )
    .map_err(|e| e.to_string())?;
    let (mut them, them_problem) = match system_audio() {
        Ok(them) => (Some(them), None),
        Err(e) => (None, Some(e)),
    };

    let now = chrono::Local::now();
    let (app_name, title) = call_app();
    let (store, _) = crate::notes::store_for(settings.notes.folder.clone())?;
    let header = format!(
        "# {title} · {}\n\nRecorded on this computer · transcribed {place}.\n",
        now.format("%b %-d, %-I:%M %p")
    );
    let note = store
        .create_in(
            settings.notes.meeting_folder(),
            &header,
            now,
            &Origin {
                app: app_name.as_deref(),
                source: Some("meeting"),
                ..Origin::default()
            },
        )
        .map_err(|e| e.to_string())?;
    if let Some(problem) = &them_problem {
        let _ = store.add_paragraph(
            &note.path,
            &format!("*Only your side is recorded: {problem}*"),
        );
    }

    // The spool: both sides as they're recorded, until the note is done.
    let dir = spool_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = now.format("%Y%m%d-%H%M%S").to_string();
    let raw_path = dir.join(format!("{id}.raw"));
    let spool = Spool {
        note: note.path.clone(),
        folder: store.root().to_path_buf(),
        started: now.to_rfc3339(),
        written: 0.0,
    };
    let meta_path = dir.join(format!("{id}.json"));
    std::fs::write(
        &meta_path,
        serde_json::to_string(&spool).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut raw =
        std::io::BufWriter::new(std::fs::File::create(&raw_path).map_err(|e| e.to_string())?);

    crate::notes::open(app);
    let mut job = Job::new(&store, &note.path, &speech);
    job.spool = Some((meta_path.clone(), spool));
    let mut step = InStep::default();
    let mut status = Status {
        note: note.path.clone(),
        them_problem: them_problem.clone(),
        place: place.to_string(),
        ..Status::default()
    };
    show(app, Some(&status));
    let started = Instant::now();
    let mut asked: Option<Instant> = None;
    let mut ended = None;

    loop {
        std::thread::sleep(TICK);
        let mine = you.take();
        let theirs = match them.as_mut() {
            Some(them) => them.take(),
            // Without the other side, silence keeps it in step.
            None => vec![0.0; mine.len()],
        };
        let (mine, theirs) = step.push(&mine, &theirs);
        write_stereo(&mut raw, &mine, &theirs);
        job.push(&mine, &theirs);
        job.transcribe(false);

        status.elapsed = job.recorded().as_secs_f64();
        status.you = you.level();
        status.them = them.as_ref().map_or(0.0, LiveAudio::level);

        if keep.swap(false, Ordering::Relaxed) {
            asked = None;
            status.warning = None;
            job.heard_at = job.recorded();
        }
        let quiet = job.recorded().saturating_sub(job.heard_at);
        let too_long = started.elapsed() >= LONGEST - GRACE;
        // Someone spoke after "Still in a meeting?": it is one.
        if asked.is_some() && !too_long && quiet < QUIET {
            asked = None;
            status.warning = None;
        }
        if asked.is_none() && ((settings.notes.stop_when_quiet() && quiet >= QUIET) || too_long) {
            asked = Some(Instant::now());
            status.warning = Some(if too_long {
                "Meetings record for 4 hours at most. Stopping in 1 minute.".into()
            } else {
                "Still in a meeting? Nobody has spoken for 10 minutes. Stopping in 1 minute.".into()
            });
            crate::notes::open(app);
        }
        show(app, Some(&status));

        let at = timestamp(job.recorded());
        if stop.load(Ordering::Relaxed) {
            ended = Some(format!("*Ended at {at}.*"));
        } else if you.lost() {
            ended = Some(format!("*Stopped at {at}: the microphone went away.*"));
        } else if asked.is_some_and(|t| t.elapsed() >= GRACE) {
            ended = Some(if too_long {
                format!("*Stopped at {at}, the longest a meeting records.*")
            } else {
                format!("*Stopped at {at}: nobody had spoken for 10 minutes.*")
            });
        }
        if ended.is_some() {
            break;
        }
    }

    // The last of it.
    let mine = you.take();
    let theirs = match them.as_mut() {
        Some(them) => them.take(),
        None => vec![0.0; mine.len()],
    };
    drop((you, them));
    let _ = step.push(&mine, &theirs);
    let (mine, theirs) = step.rest();
    write_stereo(&mut raw, &mine, &theirs);
    let _ = raw.flush();
    drop(raw);
    status.finishing = true;
    status.warning = None;
    show(app, Some(&status));
    job.push(&mine, &theirs);
    let echoes = job.finish();
    if let Some(ended) = &ended {
        let _ = store.add_paragraph(&note.path, ended);
    }
    if echoes > 0 {
        tell(
            app,
            "Some of what your speakers played was picked up by the microphone and left out. Headphones avoid that.".into(),
            false,
        );
    }
    if job.lost_writes {
        // The note couldn't take the turns (renamed, moved or deleted
        // meanwhile): the recording stays, to finish at the next launch.
        tell(
            app,
            "Part of the meeting couldn't be written into its note. Its recording is kept: Splashdown offers to finish it when it next starts.".into(),
            true,
        );
        return Ok(());
    }
    finish(app, &store, &note.path, &raw_path, &settings);
    let _ = std::fs::remove_file(&raw_path);
    let _ = std::fs::remove_file(&meta_path);
    tracing::info!("meeting ended after {:.0} s", job.recorded().as_secs_f64());
    Ok(())
}

/// Keeps You and Them in step. Each tick brings a slightly different
/// amount from each side; the surplus waits for the other side rather
/// than being padded with silence (which would put gaps in the recording
/// and stretch its timeline). A side that sends nothing for a while (the
/// computer's sound while nothing plays, on Windows) is filled with
/// silence once it's a second behind.
#[derive(Default)]
struct InStep {
    you: Vec<f32>,
    them: Vec<f32>,
}

impl InStep {
    /// Takes a tick's audio; returns what both sides have, in step.
    fn push(&mut self, you: &[f32], them: &[f32]) -> (Vec<f32>, Vec<f32>) {
        self.you.extend_from_slice(you);
        self.them.extend_from_slice(them);
        let ahead = self.you.len().max(self.them.len());
        let behind = ahead.saturating_sub(RATE);
        for side in [&mut self.you, &mut self.them] {
            if side.len() < behind {
                side.resize(behind, 0.0);
            }
        }
        let n = self.you.len().min(self.them.len());
        (
            self.you.drain(..n).collect(),
            self.them.drain(..n).collect(),
        )
    }

    /// Everything left, the shorter side padded to the longer.
    fn rest(&mut self) -> (Vec<f32>, Vec<f32>) {
        let n = self.you.len().max(self.them.len());
        self.you.resize(n, 0.0);
        self.them.resize(n, 0.0);
        (
            std::mem::take(&mut self.you),
            std::mem::take(&mut self.them),
        )
    }
}

/// What the computer plays, as a live recording.
fn system_audio() -> Result<LiveAudio, String> {
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::system_audio();
    #[cfg(windows)]
    return LiveAudio::system_output().map_err(|e| e.to_string());
    #[cfg(not(any(target_os = "macos", windows)))]
    Err("recording the computer's sound isn't supported here".into())
}

/// The spool: 16-bit stereo, You left and Them right. The shorter side is
/// padded so the two stay in step.
fn write_stereo(out: &mut impl std::io::Write, left: &[f32], right: &[f32]) {
    let frames = left.len().max(right.len());
    let mut bytes = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        for side in [left, right] {
            let s = side.get(i).copied().unwrap_or(0.0).clamp(-1.0, 1.0);
            bytes.extend_from_slice(&((s * f32::from(i16::MAX)) as i16).to_le_bytes());
        }
    }
    if let Err(e) = out.write_all(&bytes) {
        tracing::warn!("couldn't write the meeting spool: {e}");
    }
}

/// The first meeting: a reminder that the people on the call should know.
fn remind(app: &AppHandle) {
    app.dialog()
        .message(
            "Splashdown records your microphone and what your computer plays, and writes a transcript into your notes. Recording laws vary: let the people on the call know you're transcribing it.",
        )
        .title("Recording a meeting")
        .buttons(MessageDialogButtons::OkCustom("Got it".into()))
        .blocking_show();
}

/// The turn-by-turn transcript, written into the note as it's finished.
struct Job<'a> {
    store: &'a Store,
    note: &'a str,
    stt: Arc<dyn splashdown_core::SpeechToText>,
    hints: splashdown_core::SttHints,
    cleanup: splashdown_audio::Cleanup,
    cutters: [TurnCutter; 2],
    meeting: Meeting,
    /// Samples taken from each side so far.
    taken: [usize; 2],
    /// When someone last spoke, in recording time.
    heard_at: Duration,
    /// Turns that ended before this (an earlier, unfinished run of the
    /// same meeting) are already in the note.
    resume_at: Duration,
    /// The spool's sidecar, kept up to date with how much is written.
    spool: Option<(PathBuf, Spool)>,
    /// A turn couldn't be written into the note.
    lost_writes: bool,
}

impl<'a> Job<'a> {
    fn new(store: &'a Store, note: &'a str, speech: &crate::session::FileSpeech) -> Self {
        Self {
            store,
            note,
            stt: speech.stt.clone(),
            hints: speech.hints.clone(),
            cleanup: speech.cleanup,
            cutters: [TurnCutter::new(), TurnCutter::new()],
            meeting: Meeting::new(),
            taken: [0, 0],
            heard_at: Duration::ZERO,
            resume_at: Duration::ZERO,
            spool: None,
            lost_writes: false,
        }
    }

    fn recorded(&self) -> Duration {
        Duration::from_secs_f64(self.taken[0].max(self.taken[1]) as f64 / RATE as f64)
    }

    fn push(&mut self, you: &[f32], them: &[f32]) {
        self.cutters[0].push(you);
        self.cutters[1].push(them);
        self.taken[0] += you.len();
        self.taken[1] += them.len();
    }

    /// Transcribes the turns that are ready (all of them when `ended`) and
    /// writes the paragraphs that are final.
    fn transcribe(&mut self, ended: bool) {
        loop {
            // The side whose next turn is ready, earliest first... a turn
            // only comes out once its end is known, so take one at a time.
            let mut progressed = false;
            for (i, side) in [Side::You, Side::Them].into_iter().enumerate() {
                let Some(piece) = self.cutters[i].next(ended) else {
                    continue;
                };
                progressed = true;
                let end = piece.start + piece.clip.duration();
                if end <= self.resume_at {
                    self.meeting.reached(side, end);
                    continue;
                }
                let text = match splashdown_audio::prepare_with(&piece.clip, self.cleanup) {
                    Some(clip) => {
                        self.heard_at = self.heard_at.max(end);
                        transcribe(self.stt.as_ref(), &clip, &self.hints)
                    }
                    None => String::new(),
                };
                self.meeting.push(Segment {
                    side,
                    start: piece.start,
                    end,
                    text,
                });
            }
            let ready = self.meeting.ready();
            self.write(ready);
            if !progressed {
                return;
            }
        }
    }

    /// Transcribes what's left and writes the rest. Returns how many
    /// microphone turns were dropped as echo.
    fn finish(&mut self) -> usize {
        self.transcribe(true);
        let meeting = std::mem::take(&mut self.meeting);
        let echoes = meeting.echoes;
        let rest = meeting.finish();
        self.write(rest);
        echoes
    }

    fn write(&mut self, paragraphs: Vec<Paragraph>) {
        if paragraphs.is_empty() {
            return;
        }
        for p in paragraphs {
            let line = format!(
                "**[{}] {}:** {}",
                timestamp(p.start),
                p.side.label(),
                p.text
            );
            if let Err(e) = self.store.add_paragraph(self.note, &line) {
                tracing::warn!("couldn't add to the meeting note: {e}");
                self.lost_writes = true;
            }
        }
        // How far the note goes, for finishing it after a crash.
        if !self.lost_writes
            && let Some((path, spool)) = &mut self.spool
        {
            spool.written = self.meeting.settled().as_secs_f64();
            if let Ok(json) = serde_json::to_string(spool) {
                let _ = std::fs::write(path, json);
            }
        }
    }
}

/// One turn, tried twice; a failure leaves a marker rather than losing
/// the meeting.
fn transcribe(
    stt: &dyn splashdown_core::SpeechToText,
    clip: &splashdown_core::AudioClip,
    hints: &splashdown_core::SttHints,
) -> String {
    let attempt = || tauri::async_runtime::block_on(stt.transcribe(clip, hints));
    match attempt().or_else(|e| {
        tracing::warn!("a meeting turn failed, trying again: {e}");
        attempt()
    }) {
        Ok(t) => t.text.trim().to_string(),
        Err(e) => {
            tracing::warn!("a meeting turn couldn't be transcribed: {e}");
            "*(couldn't be transcribed)*".into()
        }
    }
}

/// After the transcript: the summary (as the settings say, asking first
/// for a cloud AI) and the recording, if it's kept.
fn finish(app: &AppHandle, store: &Store, note: &str, raw: &Path, settings: &Settings) {
    summarize(app, store, note, settings);
    if settings.notes.keep_meeting_audio {
        match keep_recording(store, note, raw) {
            Ok(link) => {
                let _ = store
                    .add_paragraph(note, &format!("[Recording]({})", link.replace(' ', "%20")));
            }
            Err(e) => tracing::warn!("couldn't keep the meeting recording: {e}"),
        }
    }
}

/// The AI that runs transforms (it writes the summary): a name for the
/// question, and whether it's on this computer. `None` without one.
fn summary_ai(settings: &Settings) -> Option<(String, bool)> {
    let secrets = Secrets::load();
    let on_device = crate::engine::on_device(settings);
    let choice = settings
        .resolve_instruction_ai_with(&secrets, on_device)
        .or_else(|| settings.resolve_ai_with(&secrets, on_device).ok().flatten())?;
    let local = choice.destination() == Destination::Local;
    let name = match choice.provider {
        splashdown_config::AiProvider::Openai => "OpenAI",
        splashdown_config::AiProvider::Openrouter => "OpenRouter",
        splashdown_config::AiProvider::Anthropic => "Anthropic",
        _ if local => "the AI on this computer",
        _ => "your AI server",
    };
    Some((name.to_string(), local))
}

fn summarize(app: &AppHandle, store: &Store, note: &str, settings: &Settings) {
    let Some((ai, local)) = summary_ai(settings) else {
        return;
    };
    let run = match settings.notes.meeting_summary {
        Some(MeetingSummary::Always) => true,
        Some(MeetingSummary::Never) => false,
        None if local => true,
        Some(MeetingSummary::Ask) | None => ask_to_summarize(app, &ai),
    };
    if !run {
        return;
    }
    let Ok(body) = store.read(note).map(|n| n.body) else {
        return;
    };
    let Some(at) = body.find("\n**[") else {
        return;
    };
    let transcript = body[at..].trim().to_string();
    let state = app.state::<AppState>();
    match state
        .session
        .transform("meeting-summary".into(), transcript)
    {
        Ok(out) if out.unchanged.is_none() => {
            if let Err(e) = store.insert_summary(note, &out.text) {
                tracing::warn!("couldn't add the summary: {e}");
            }
        }
        Ok(out) => tracing::info!("no meeting summary: {}", out.unchanged.unwrap_or_default()),
        Err(e) => tell(app, format!("The meeting summary didn't work: {e}"), true),
    }
}

/// Asks before the meeting's text goes to a cloud AI. "Always" is kept.
fn ask_to_summarize(app: &AppHandle, ai: &str) -> bool {
    let answer = app
        .dialog()
        .message(format!(
            "The transcript goes to {ai} to write a summary of decisions, action items and open questions."
        ))
        .title(format!("Summarize this meeting with {ai}?"))
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            "Summarize".into(),
            "Always summarize".into(),
            "Not now".into(),
        ))
        .blocking_show_with_result();
    let pressed = |label: &str, plain: MessageDialogResult| {
        answer == plain || matches!(&answer, MessageDialogResult::Custom(s) if s == label)
    };
    if pressed("Always summarize", MessageDialogResult::No) {
        let state = app.state::<AppState>();
        let mut settings = state.settings();
        settings.notes.meeting_summary = Some(MeetingSummary::Always);
        if let Ok(paths) = crate::engine::paths()
            && settings.save(&paths.config_file()).is_ok()
        {
            *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
            crate::commands::settings_changed_elsewhere(app);
        }
        return true;
    }
    pressed("Summarize", MessageDialogResult::Yes)
}

/// The spool as a WAV in `Attachments/`. Returns its path for the link.
fn keep_recording(store: &Store, note: &str, raw: &Path) -> Result<String, String> {
    let stem = Path::new(note)
        .file_stem()
        .map_or_else(|| "Meeting".into(), |s| s.to_string_lossy().into_owned());
    let (path, rel) = store
        .new_attachment(&stem, "wav")
        .map_err(|e| e.to_string())?;
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(raw).map_err(|e| e.to_string())?;
    for pair in bytes.as_chunks::<2>().0 {
        writer
            .write_sample(i16::from_le_bytes(*pair))
            .map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    Ok(rel)
}

// ---------------------------------------------------------------- after a crash

/// At launch: a meeting whose spool is still there wasn't finished (the
/// app crashed or was forced to quit). Offers to finish its transcript.
pub fn recover(app: &AppHandle) {
    let Ok(dir) = spool_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let spools: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    if spools.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        for meta in spools {
            if let Err(e) = recover_one(&app, &meta) {
                tracing::warn!("couldn't finish an earlier meeting: {e}");
            }
        }
    });
}

fn recover_one(app: &AppHandle, meta: &Path) -> Result<(), String> {
    let spool: Spool =
        serde_json::from_str(&std::fs::read_to_string(meta).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let raw = meta.with_extension("raw");
    let when = chrono::DateTime::parse_from_rfc3339(&spool.started)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%b %-d, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_default();
    let answer = app
        .dialog()
        .message(format!(
            "The meeting recorded {when} wasn't finished when Splashdown closed. Its recording is still here."
        ))
        .title("Finish transcribing it?")
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            "Finish".into(),
            "Delete it".into(),
            "Later".into(),
        ))
        .blocking_show_with_result();
    let pressed = |label: &str, plain: MessageDialogResult| {
        answer == plain || matches!(&answer, MessageDialogResult::Custom(s) if s == label)
    };
    if pressed("Delete it", MessageDialogResult::No) {
        let _ = std::fs::remove_file(&raw);
        let _ = std::fs::remove_file(meta);
        return Ok(());
    }
    if !pressed("Finish", MessageDialogResult::Yes) {
        return Ok(());
    }

    let state = app.state::<AppState>();
    let settings = state.settings();
    let speech = state.session.file_speech()?;
    let (store, _) = crate::notes::store_for(Some(spool.folder.clone()))?;
    let note = &spool.note;
    // The note already has the turns up to where the spool says (older
    // spools: its last time stamp).
    let resume_at = if spool.written > 0.0 {
        Duration::from_secs_f64(spool.written)
    } else {
        store
            .read(note)
            .ok()
            .and_then(|n| last_timestamp(&n.body))
            .unwrap_or_default()
    };
    let mut job = Job::new(&store, note, &speech);
    job.resume_at = resume_at;
    // Half a second at a time, as when it was recorded: all at once, every
    // turn would scan the whole meeting.
    let mut file = std::io::BufReader::new(std::fs::File::open(&raw).map_err(|e| e.to_string())?);
    let mut bytes = vec![0u8; RATE / 2 * 4];
    loop {
        let n = read_full(&mut file, &mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let (mut you, mut them) = (Vec::new(), Vec::new());
        for frame in bytes[..n].as_chunks::<4>().0 {
            you.push(f32::from(i16::from_le_bytes([frame[0], frame[1]])) / f32::from(i16::MAX));
            them.push(f32::from(i16::from_le_bytes([frame[2], frame[3]])) / f32::from(i16::MAX));
        }
        job.push(&you, &them);
        job.transcribe(false);
    }
    job.finish();
    if job.lost_writes {
        return Err("the meeting note couldn't be written to; its recording is kept".into());
    }
    let _ = store.add_paragraph(
        note,
        &format!(
            "*Finished after Splashdown closed; the recording ends at {}.*",
            timestamp(job.recorded())
        ),
    );
    finish(app, &store, note, &raw, &settings);
    let _ = std::fs::remove_file(&raw);
    let _ = std::fs::remove_file(meta);
    Ok(())
}

/// Fills `buf` from `file` as far as it goes; returns how much was read.
fn read_full(file: &mut impl std::io::Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// The time of the last `**[m:ss] …:**` paragraph in a meeting note.
fn last_timestamp(body: &str) -> Option<Duration> {
    let line = body.lines().rev().find(|l| l.starts_with("**["))?;
    let time = line.strip_prefix("**[")?.split(']').next()?;
    let mut secs = 0u64;
    for part in time.split(':') {
        secs = secs * 60 + part.parse::<u64>().ok()?;
    }
    Some(Duration::from_secs(secs))
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub fn meeting_toggle(app: AppHandle) {
    toggle(&app);
}

#[tauri::command]
pub fn meeting_keep_going() {
    keep_going();
}

#[tauri::command]
pub fn meeting_status() -> Option<Status> {
    status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_sides_stay_in_step_without_gaps() {
        let mut step = InStep::default();
        // The microphone brings 0.6 s, the computer's sound 0.5 s: the
        // extra 0.1 s waits rather than being matched with silence.
        let (you, them) = step.push(&vec![0.1; 9600], &vec![0.2; 8000]);
        assert_eq!((you.len(), them.len()), (8000, 8000));
        let (you, them) = step.push(&vec![0.1; 7200], &vec![0.2; 8800]);
        assert_eq!((you.len(), them.len()), (8800, 8800));
        assert!(you.iter().all(|&s| s == 0.1), "no silence slipped in");
        // The computer's sound goes quiet (sends nothing): once it's a
        // second behind, it's filled with silence so turns keep coming.
        let mut out = 0;
        for _ in 0..6 {
            out += step.push(&vec![0.1; 8000], &[]).0.len();
        }
        assert_eq!(out, 6 * 8000 - RATE);
        // At the end, the rest, padded to the longer side.
        let (you, them) = step.rest();
        assert_eq!((you.len(), them.len()), (RATE, RATE));
    }

    /// Answers with the length of what it hears, so turns can be told apart.
    struct Lengths;

    #[async_trait::async_trait]
    impl splashdown_core::SpeechToText for Lengths {
        fn name(&self) -> String {
            "local/test".into()
        }
        async fn transcribe(
            &self,
            audio: &splashdown_core::AudioClip,
            _: &splashdown_core::SttHints,
        ) -> splashdown_core::ProviderResult<splashdown_core::Transcript> {
            Ok(splashdown_core::Transcript {
                text: format!("Heard {:.0} seconds.", audio.duration().as_secs_f32()),
                usage: splashdown_core::Usage::FREE,
            })
        }
    }

    fn clip(name: &str) -> Vec<f32> {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../eval/audio/{name}.wav"));
        splashdown_audio::wav::read(&path).unwrap().samples
    }

    #[test]
    fn a_call_becomes_turns_in_the_note() {
        let root = std::env::temp_dir().join(format!("splashdown-meeting-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Store::new(root.clone());
        let note = store
            .create_in(
                "Meetings",
                "# Meeting\n",
                chrono::Local::now(),
                &Origin::default(),
            )
            .unwrap();
        let speech = crate::session::FileSpeech {
            stt: Arc::new(Lengths),
            hints: splashdown_core::SttHints::default(),
            cleanup: splashdown_audio::Cleanup::default(),
        };
        let mut job = Job::new(&store, &note.path, &speech);

        // Them asks (the long clip), then You answers; each side is silent
        // while the other speaks, as on a call with headphones.
        let them_says = clip("long-email");
        let you_say = clip("question");
        let gap = vec![0.0; RATE];
        let you: Vec<f32> = [
            vec![0.0; them_says.len()],
            gap.clone(),
            you_say.clone(),
            gap.clone(),
        ]
        .concat();
        let them: Vec<f32> = [
            them_says.clone(),
            gap.clone(),
            vec![0.0; you_say.len()],
            gap,
        ]
        .concat();
        // Half a second at a time, as the recording loop takes it.
        for (y, t) in you.chunks(RATE / 2).zip(them.chunks(RATE / 2)) {
            job.push(y, t);
            job.transcribe(false);
        }
        assert_eq!(job.finish(), 0);
        let body = store.read(&note.path).unwrap().body;
        let turns: Vec<&str> = body.lines().filter(|l| l.starts_with("**[")).collect();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(turns.len(), 2, "{body}");
        assert!(turns[0].starts_with("**[0:00] Them:** Heard"), "{body}");
        let answer_at = timestamp(Duration::from_secs_f64(
            (them_says.len() + RATE) as f64 / RATE as f64,
        ));
        assert!(
            turns[1].starts_with(&format!("**[{answer_at}] You:** Heard")),
            "{body}"
        );
        assert!(job.recorded() >= Duration::from_secs(5));
    }

    #[test]
    fn finds_where_a_transcript_stopped() {
        let body =
            "# Zoom meeting\n\n**[0:04] Them:** Hi.\n\n**[1:02:30] You:** Bye.\n\n*Ended.*\n";
        assert_eq!(last_timestamp(body), Some(Duration::from_secs(3750)));
        assert_eq!(last_timestamp("# Empty\n"), None);
    }

    #[test]
    fn the_spool_keeps_the_sides_in_step() {
        let mut out = Vec::new();
        write_stereo(&mut out, &[0.5, -0.5], &[0.25]);
        let samples: Vec<i16> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        assert_eq!(samples.len(), 4);
        assert_eq!(samples[1], (0.25 * f32::from(i16::MAX)) as i16);
        assert_eq!(samples[3], 0, "the shorter side is padded");
    }
}
