//! Meeting notes: a call recorded as two sides, **You** (the microphone)
//! and **Them** (what the computer plays), transcribed a turn at a time into
//! a note in `Meetings/` while it goes, and summarized when it ends.
//!
//! An app's audio is recorded the same way with one side: what that app
//! plays (a webinar in a browser, a podcast app), with no microphone, into
//! a note of timed paragraphs. So is the microphone on its own (a lecture,
//! a conversation in the room), with no computer sound.
//!
//! The audio is spooled to the config folder as it's recorded, so a crash
//! or a forced quit loses nothing: the next launch offers to finish the
//! transcript. The spool is deleted once the note is done.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use splashdown_audio::file::{TurnCutter, timestamp};
use splashdown_audio::live::LiveAudio;
use splashdown_config::{Destination, MeetingSummary, Secrets, Settings};
use splashdown_core::meeting::{Meeting, Paragraph, Segment, Side};
use tauri::{AppHandle, Emitter, Manager};

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
/// How often to look for a change of output device.
const OUTPUT_CHECK: Duration = Duration::from_secs(2);
/// The heading of what the user writes during the meeting, in its note.
pub const MY_NOTES: &str = "My notes";
/// An app's audio note's `source`, in its front matter.
pub const APP_AUDIO: &str = "app-audio";

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
    /// Recording an app's audio rather than a meeting: the app's name (or
    /// "this computer's sound").
    pub app: Option<String>,
    /// Seconds into the recording when words were last heard, if they
    /// have been: whether what's recorded is being transcribed.
    pub heard: Option<f64>,
    /// A question the note waits on ("Summarize this meeting with
    /// OpenRouter?"). It's asked in its own panel; Notes shows it too, so
    /// it can be answered there.
    pub question: Option<crate::prompt::Prompt>,
}

/// What's recorded.
#[derive(Debug, Clone)]
pub enum Source {
    /// A call: the microphone and what the computer plays.
    Meeting,
    /// What an app plays, with no microphone: `sources` from
    /// `splashdown_platform::audio_apps`, or none for everything the
    /// computer plays.
    App { name: String, sources: Vec<u32> },
    /// The microphone alone: a lecture, a conversation in the room.
    Microphone,
}

/// The name of a one-sided recording's sound, as the page shows it.
const MICROPHONE: &str = "Microphone";
/// A microphone recording's note's `source`, in its front matter.
pub const RECORDING: &str = "recording";

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
    start_recording(app, Source::Meeting)
}

/// Starts recording `source`, as `start`.
pub fn start_recording(app: &AppHandle, source: Source) -> Result<String, String> {
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
                record(&handle, &source, &stop, &keep)
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

/// The calendar event on now: its title and who else is invited.
#[derive(Debug, Clone)]
struct Event {
    title: String,
    people: Vec<String>,
}

#[cfg(target_os = "macos")]
fn calendar_event() -> Option<Event> {
    let event = tauri::async_runtime::block_on(splashdown_apple_shim::calendar_now())?;
    tracing::info!(
        "meeting named from the calendar ({} other people)",
        event.people.len()
    );
    Some(Event {
        title: event.title,
        people: event.people,
    })
}

#[cfg(not(target_os = "macos"))]
fn calendar_event() -> Option<Event> {
    None
}

/// Asks for the calendar (Settings → Notes → Meetings): whether it's allowed.
#[tauri::command]
pub async fn calendar_access() -> bool {
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::calendar_access().await;
    #[cfg(not(target_os = "macos"))]
    false
}

/// The sound of the app with `sources` (none: everything the computer
/// plays but Splashdown).
fn app_audio(sources: &[u32]) -> Result<LiveAudio, String> {
    if sources.is_empty() {
        return system_audio();
    }
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::app_audio(sources);
    #[cfg(windows)]
    return windows_app_audio(sources[0]);
    #[cfg(not(any(target_os = "macos", windows)))]
    Err("recording an app's sound isn't supported here".into())
}

/// What process `pid`'s tree plays, through Windows' process loopback.
#[cfg(windows)]
fn windows_app_audio(pid: u32) -> Result<LiveAudio, String> {
    let (live, pusher) = LiveAudio::pushed(Box::new(()));
    let capture = splashdown_platform::app_loopback(
        pid,
        Box::new(move |samples: &[f32], rate: u32| pusher.push(samples, rate)),
    )
    .map_err(|e| format!("Couldn't record that app: {e}"))?;
    Ok(live.keeping(Box::new(capture)))
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
        let answer = crate::prompt::ask(
            &app,
            "A meeting is recording",
            "Stop it and finish the note before quitting?",
            &["Stop and quit", "Keep recording"],
        );
        if answer != Some(0) {
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
    /// The app whose audio this is, when it isn't a meeting.
    #[serde(default)]
    app: Option<String>,
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

/// Records a meeting (or an app's audio) until it's stopped, then finishes
/// its note.
fn record(
    app: &AppHandle,
    source: &Source,
    stop: &AtomicBool,
    keep: &AtomicBool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state.settings();
    let listening = match source {
        Source::App { name, .. } => Some(name.clone()),
        Source::Microphone => Some(MICROPHONE.to_string()),
        Source::Meeting => None,
    };
    let microphone_only = matches!(source, Source::Microphone);
    // Telling people they're recorded is about other people: a call, or
    // the room.
    if !matches!(source, Source::App { .. }) && !settings.notes.meeting_reminded {
        remind(app);
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

    // An app's audio takes the microphone's place: it's the one side, and
    // keeps the time.
    let microphone = || {
        LiveAudio::microphone(
            splashdown_audio::devices::choose(
                &settings.general.microphones(),
                &splashdown_audio::devices::inputs(),
            )
            .as_deref(),
        )
        .map_err(|e| e.to_string())
    };
    let (mut you, mut them, them_problem) = match source {
        Source::Meeting => {
            let you = microphone()?;
            match system_audio() {
                Ok(them) => (you, Some(them), None),
                Err(e) => (you, None, Some(e)),
            }
        }
        Source::App { sources, .. } => (app_audio(sources)?, None, None),
        Source::Microphone => (microphone()?, None, None),
    };

    let now = chrono::Local::now();
    // A meeting on the calendar names the note and says who's in it.
    let event = match source {
        Source::Meeting if settings.notes.meeting_calendar => calendar_event(),
        _ => None,
    };
    let (app_name, title) = match &listening {
        _ if microphone_only => (None, "Recording".to_string()),
        Some(name) => (Some(name.clone()), name.clone()),
        None => {
            let (app_name, title) = call_app();
            (app_name, event.as_ref().map_or(title, |e| e.title.clone()))
        }
    };
    let (store, _) = crate::notes::store_for(settings.notes.folder.clone())?;
    let recorded_from = match &listening {
        _ if microphone_only => "Recorded from the microphone".to_string(),
        Some(name) => format!("Recorded from {name}"),
        None => "Recorded on this computer".into(),
    };
    let with = event
        .as_ref()
        .filter(|e| !e.people.is_empty())
        .map(|e| format!("With {}.\n\n", e.people.join(", ")))
        .unwrap_or_default();
    let header = format!(
        "# {title} · {}\n\n{with}{recorded_from} · transcribed {place}.\n",
        now.format("%b %-d, %-I:%M %p")
    );
    let note = store
        .create_in(
            if listening.is_some() {
                ""
            } else {
                settings.notes.meeting_folder()
            },
            &header,
            now,
            &Origin {
                app: app_name.as_deref(),
                source: Some(match source {
                    Source::Meeting => "meeting",
                    Source::App { .. } => APP_AUDIO,
                    Source::Microphone => RECORDING,
                }),
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
        app: listening.clone(),
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
    let mut step = InStep::default();
    let mut status = Status {
        note: note.path.clone(),
        them_problem: them_problem.clone(),
        place: place.to_string(),
        app: listening.clone(),
        ..Status::default()
    };
    show(app, Some(&status));
    let started = Instant::now();
    let mut asked: Option<Instant> = None;
    let mut ended = None;
    // Samples recorded on each side (in step), and when someone was last
    // heard, in milliseconds of recording, from the transcriber.
    let mut recorded = 0usize;
    let heard = AtomicU64::new(0);
    let mut kept_at = Duration::ZERO;
    let mut output = splashdown_audio::devices::default_output();
    let mut output_checked = Instant::now();

    // Transcribing runs beside the recording: a turn waiting for the
    // speech model (busy with a dictation, say) never holds up capture,
    // the spool or Stop.
    let one_sided = listening.is_some();
    let (echoes, lost_writes) = std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel::<(Vec<f32>, Vec<f32>)>();
        let (store, note_path, speech, heard) = (&store, &note.path, &speech, &heard);
        let spool = (meta_path.clone(), spool);
        let transcriber = scope.spawn(move || {
            let mut job = Job::new(store, note_path, speech);
            job.one_sided = one_sided;
            job.spool = Some(spool);
            while let Ok((you, them)) = rx.recv() {
                job.push(&you, &them);
                // Whatever else arrived meanwhile, then the turns.
                while let Ok((you, them)) = rx.try_recv() {
                    job.push(&you, &them);
                }
                job.transcribe(false);
                heard.store(job.heard_at.as_millis() as u64, Ordering::Relaxed);
            }
            (job.finish(), job.lost_writes)
        });

        loop {
            std::thread::sleep(TICK);
            // The computer's sound follows the output device: headphones
            // plugged in or AirPods connected mid-call.
            if output_checked.elapsed() >= OUTPUT_CHECK {
                output_checked = Instant::now();
                let now_output = splashdown_audio::devices::default_output();
                if now_output != output && them_problem.is_none() && listening.is_none() {
                    tracing::info!(
                        "the output device changed: recording the computer's sound again"
                    );
                    output = now_output;
                    them = match system_audio() {
                        Ok(again) => Some(again),
                        Err(e) => {
                            status.them_problem = Some(e);
                            None
                        }
                    };
                }
            }
            let mine = you.take();
            let theirs = match them.as_mut() {
                Some(them) => them.take(),
                // Without the other side, silence keeps it in step.
                None => vec![0.0; mine.len()],
            };
            let (mine, theirs) = step.push(&mine, &theirs);
            write_stereo(&mut raw, &mine, &theirs);
            recorded += mine.len();
            let _ = tx.send((mine, theirs));
            let recorded_for = Duration::from_secs_f64(recorded as f64 / RATE as f64);

            status.elapsed = recorded_for.as_secs_f64();
            status.you = you.level();
            status.them = them.as_ref().map_or(0.0, LiveAudio::level);

            if keep.swap(false, Ordering::Relaxed) {
                asked = None;
                status.warning = None;
                kept_at = recorded_for;
            }
            let heard_ms = heard.load(Ordering::Relaxed);
            status.heard = (heard_ms > 0).then(|| heard_ms as f64 / 1000.0);
            let heard_at = Duration::from_millis(heard_ms).max(kept_at);
            let quiet = recorded_for.saturating_sub(heard_at);
            let too_long = started.elapsed() >= LONGEST - GRACE;
            // Someone spoke after "Still in a meeting?": it is one.
            if asked.is_some() && !too_long && quiet < QUIET {
                asked = None;
                status.warning = None;
            }
            if asked.is_none() && ((settings.notes.stop_when_quiet() && quiet >= QUIET) || too_long)
            {
                asked = Some(Instant::now());
                status.warning = Some(if too_long {
                    "Recordings last 4 hours at most. Stopping in 1 minute.".into()
                } else if listening.is_some() {
                    "Still listening? Nothing has been said for 10 minutes. Stopping in 1 minute."
                        .into()
                } else {
                    "Still in a meeting? Nobody has spoken for 10 minutes. Stopping in 1 minute."
                        .into()
                });
                crate::notes::open(app);
            }
            show(app, Some(&status));

            let at = timestamp(recorded_for);
            if stop.load(Ordering::Relaxed) {
                ended = Some(format!("*Ended at {at}.*"));
            } else if you.lost() {
                ended = Some(match &listening {
                    Some(name) if !microphone_only => {
                        format!("*Stopped at {at}: {name}'s sound went away.*")
                    }
                    _ => format!("*Stopped at {at}: the microphone went away.*"),
                });
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
        let _ = step.push(&mine, &theirs);
        let (mine, theirs) = step.rest();
        write_stereo(&mut raw, &mine, &theirs);
        let _ = raw.flush();
        recorded += mine.len();
        let _ = tx.send((mine, theirs));
        drop(tx);
        status.finishing = true;
        status.warning = None;
        show(app, Some(&status));
        transcriber.join().unwrap_or_else(|_| {
            tracing::error!("transcribing the meeting panicked");
            (0, true)
        })
    });
    drop((you, them, raw));
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
    if lost_writes {
        // The note couldn't take the turns (renamed, moved or deleted
        // meanwhile): the recording stays, to finish at the next launch.
        tell(
            app,
            "Part of the meeting couldn't be written into its note. Its recording is kept: Splashdown offers to finish it when it next starts.".into(),
            true,
        );
        return Ok(());
    }
    finish(
        app,
        &store,
        &note.path,
        &raw_path,
        &settings,
        listening.is_some(),
    );
    let _ = std::fs::remove_file(&raw_path);
    let _ = std::fs::remove_file(&meta_path);
    tracing::info!("meeting ended after {:.0} s", recorded as f64 / RATE as f64);
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
/// The recording doesn't wait for it.
fn remind(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        crate::prompt::ask(
            &app,
            "Recording a meeting",
            "Splashdown records your microphone and what your computer plays, and writes a transcript into your notes. Recording laws vary: let the people on the call know you're transcribing it.",
            &["Got it"],
        );
    });
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
    /// One side only (an app's audio): turns are written without You or
    /// Them.
    one_sided: bool,
    /// Turns are kept here instead of written into the note (transcribing
    /// a recording again).
    collected: Option<Vec<String>>,
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
            one_sided: false,
            collected: None,
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
            let line = if self.one_sided {
                format!("**[{}]** {}", timestamp(p.start), p.text)
            } else {
                format!(
                    "**[{}] {}:** {}",
                    timestamp(p.start),
                    p.side.label(),
                    p.text
                )
            };
            if let Some(collected) = &mut self.collected {
                collected.push(line);
                continue;
            }
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
fn finish(
    app: &AppHandle,
    store: &Store,
    note: &str,
    raw: &Path,
    settings: &Settings,
    one_sided: bool,
) {
    // Speakers first, so the summary can say who said what.
    if settings.notes.meeting_speakers {
        name_speakers(store, note, raw, one_sided);
    }
    summarize(app, store, note, settings, one_sided);
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

/// Tells the other people apart (a meeting's Them side, or everyone in a
/// one-sided recording) with the downloaded diarization model, and labels
/// their turns Speaker 1, Speaker 2… Nothing changes with only one voice,
/// or without the model.
fn name_speakers(store: &Store, note: &str, raw: &Path, one_sided: bool) {
    let Ok(models) = crate::engine::paths().map(|p| p.models_dir) else {
        return;
    };
    let model = splashdown_providers::models::Diarizer::path_in(&models);
    if !model.is_file() {
        return;
    }
    let Ok(bytes) = std::fs::read(raw) else {
        return;
    };
    // The spool is You left, Them right; a one-sided recording is all left.
    let channel = if one_sided { 0 } else { 2 };
    let pcm: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|f| f32::from(i16::from_le_bytes([f[channel], f[channel + 1]])) / f32::from(i16::MAX))
        .collect();
    let started = Instant::now();
    let turns = match splashdown_providers::diarize(&model, &pcm) {
        Ok(turns) => turns,
        Err(e) => {
            tracing::warn!("couldn't tell the speakers apart: {e}");
            return;
        }
    };
    tracing::info!(
        "speakers told apart in {:.1} s ({} turns)",
        started.elapsed().as_secs_f32(),
        turns.len()
    );
    let spans: Vec<(u32, f64, f64)> = turns
        .iter()
        .map(|t| (t.speaker, t.start.as_secs_f64(), t.end.as_secs_f64()))
        .collect();
    let side = if one_sided { None } else { Some("Them") };
    if let Err(e) = store.rewrite(note, |body| label_speakers(body, &spans, side)) {
        tracing::warn!("couldn't label the speakers: {e}");
    }
}

/// `body` with the turns from one side (`side`, or every unlabelled turn of
/// a one-sided recording with `None`) labelled by who spoke: each turn gets
/// the speaker of `spans` (speaker, start s, end s) it overlaps most,
/// numbered in the order they first speak. Unchanged unless there are two
/// or more.
fn label_speakers(body: &str, spans: &[(u32, f64, f64)], side: Option<&str>) -> String {
    // Each turn of that side: its line, and when it starts and ends (the
    // next turn's start).
    let lines: Vec<&str> = body.split('\n').collect();
    let mut turns: Vec<(usize, f64)> = Vec::new();
    let mut starts: Vec<f64> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((stamp, who, _)) = turn_head(line) else {
            continue;
        };
        let at = seconds(stamp);
        starts.push(at);
        if who == side.unwrap_or("") {
            turns.push((i, at));
        }
    }
    let end_of = |at: f64| {
        starts
            .iter()
            .copied()
            .filter(|&s| s > at)
            .fold(f64::INFINITY, f64::min)
    };
    let speaker_of = |from: f64, to: f64| {
        let mut best: Option<(u32, f64)> = None;
        for &(speaker, s, e) in spans {
            let overlap = e.min(to) - s.max(from);
            if overlap > 0.0 && best.is_none_or(|(_, o)| overlap > o) {
                best = Some((speaker, overlap));
            }
        }
        best.map(|(speaker, _)| speaker)
    };
    let found: Vec<Option<u32>> = turns
        .iter()
        .map(|&(_, at)| speaker_of(at, end_of(at).min(at + 120.0)))
        .collect();
    // Numbered as they first speak.
    let mut order: Vec<u32> = Vec::new();
    for s in found.iter().flatten() {
        if !order.contains(s) {
            order.push(*s);
        }
    }
    if order.len() < 2 {
        return body.to_string();
    }
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for (&(i, _), speaker) in turns.iter().zip(&found) {
        let (Some(speaker), Some((stamp, _, said))) = (speaker, turn_head(lines[i])) else {
            continue;
        };
        let n = order.iter().position(|s| s == speaker).unwrap_or(0) + 1;
        out[i] = format!("**[{stamp}] Speaker {n}:**{said}");
    }
    out.join("\n")
}

/// A transcript turn's time stamp, who said it ("" when it doesn't say)
/// and what was said: `**[1:02] Them:** …` or `**[1:02]** …`.
fn turn_head(line: &str) -> Option<(&str, &str, &str)> {
    let rest = line.strip_prefix("**[")?;
    let (stamp, rest) = rest.split_once(']')?;
    if !stamp.chars().all(|c| c.is_ascii_digit() || c == ':') || stamp.is_empty() {
        return None;
    }
    if let Some(said) = rest.strip_prefix("**") {
        return Some((stamp, "", said));
    }
    let (who, said) = rest.strip_prefix(' ')?.split_once(":**")?;
    Some((stamp, who, said))
}

/// "1:02:30" or "12:05" in seconds.
fn seconds(stamp: &str) -> f64 {
    stamp.split(':').fold(0.0, |total, n| {
        total * 60.0 + n.parse::<f64>().unwrap_or(0.0)
    })
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

/// Summarizes a meeting (with the Meeting summary transform) or an app's
/// audio (`one_sided`, with Summary).
fn summarize(app: &AppHandle, store: &Store, note: &str, settings: &Settings, one_sided: bool) {
    let Some((ai, local)) = summary_ai(settings) else {
        return;
    };
    // Nobody said anything: nothing to summarize, or to ask about.
    let Ok(body) = store.read(note).map(|n| n.body) else {
        return;
    };
    if crate::notes::store::transcript_start(&body).is_none() {
        return;
    }
    let run = match settings.notes.meeting_summary {
        Some(MeetingSummary::Always) => true,
        Some(MeetingSummary::Never) => false,
        None if local => true,
        Some(MeetingSummary::Ask) | None => ask_to_summarize(app, &ai, one_sided),
    };
    if !run {
        return;
    }
    // Re-read: My notes may have changed while the question was up.
    let Ok(body) = store.read(note).map(|n| n.body) else {
        return;
    };
    let Some(at) = crate::notes::store::transcript_start(&body) else {
        return;
    };
    let transcript = body[at..].trim();
    // What the user wrote down shows what mattered to them.
    let input = match crate::notes::store::section(&body[..at], MY_NOTES) {
        Some(mine) if !mine.is_empty() => {
            format!("My notes, written during the meeting:\n{mine}\n\nTranscript:\n{transcript}")
        }
        _ => transcript.to_string(),
    };
    let state = app.state::<AppState>();
    let transform = if one_sided {
        "summary"
    } else {
        "meeting-summary"
    };
    match state.session.transform(transform.into(), input) {
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
fn ask_to_summarize(app: &AppHandle, ai: &str, one_sided: bool) -> bool {
    let what = if one_sided { "recording" } else { "meeting" };
    let title = format!("Summarize this {what} with {ai}?");
    let text = if one_sided {
        format!("The transcript goes to {ai} to write a summary of its main points.")
    } else {
        format!(
            "The transcript goes to {ai} to write a summary of decisions, action items and open questions."
        )
    };
    let buttons = ["Summarize", "Always summarize", "Not now"];
    asking(
        app,
        Some(crate::prompt::Prompt {
            title: title.clone(),
            text: text.clone(),
            buttons: buttons.map(String::from).to_vec(),
        }),
    );
    let answer = crate::prompt::ask(app, &title, &text, &buttons);
    asking(app, None);
    if answer == Some(1) {
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
    answer == Some(0)
}

/// Shows the meeting waiting on `question` (or no longer), in Notes.
fn asking(app: &AppHandle, question: Option<crate::prompt::Prompt>) {
    let status = {
        let mut slot = running();
        let Some(run) = slot.as_mut() else {
            return;
        };
        run.status.question = question;
        run.status.clone()
    };
    show(app, Some(&status));
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

// ---------------------------------------------------------------- again

/// The recording kept with a meeting note (its `[Recording](…)` link), if
/// it's still there.
pub(crate) fn recording_of(store: &Store, note: &str) -> Option<PathBuf> {
    let body = store.read(note).ok()?.body;
    let link = body
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("[Recording](")?.strip_suffix(')'))?;
    let rel = link.replace("%20", " ");
    // Only the notes folder's own attachments.
    if rel.contains("..") || Path::new(&rel).is_absolute() {
        return None;
    }
    let path = store.root().join(rel);
    path.is_file().then_some(path)
}

/// Transcribes a meeting's kept recording again with `speech`, and puts the
/// new transcript in place of the old one; `model` names it in the note.
/// Speakers keep the names they were given (by the time each turn starts);
/// the rest of the note stays as it is. `progress` hears the seconds done,
/// and answers false to stop. Returns why it stopped early, if it did:
/// then nothing has changed.
pub(crate) fn transcribe_again(
    settings: &Settings,
    store: &Store,
    note: &str,
    recording: &Path,
    speech: &crate::session::FileSpeech,
    model: &str,
    progress: &mut dyn FnMut(f64) -> bool,
) -> Result<Option<String>, String> {
    let one_sided = store
        .list()
        .into_iter()
        .find(|n| n.path == note)
        .is_some_and(|n| n.source != "meeting");
    let mut reader = hound::WavReader::open(recording).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.channels != 2 || spec.sample_rate != RATE as u32 || spec.bits_per_sample != 16 {
        return Err("the recording isn't one Splashdown made".into());
    }
    // The samples as the meeting's spool had them, for telling speakers
    // apart afterwards.
    let raw = spool_dir()?.join(format!("again-{}.raw", std::process::id()));
    let mut spool =
        std::io::BufWriter::new(std::fs::File::create(&raw).map_err(|e| e.to_string())?);
    let result = (|| {
        let mut job = Job::new(store, note, speech);
        job.one_sided = one_sided;
        job.collected = Some(Vec::new());
        let mut samples = reader.samples::<i16>();
        loop {
            if !progress(job.recorded().as_secs_f64()) {
                return Ok(Some("Stopped. The transcript is as it was.".to_string()));
            }
            let (mut you, mut them) = (Vec::new(), Vec::new());
            for _ in 0..RATE / 2 {
                let (Some(l), Some(r)) = (samples.next(), samples.next()) else {
                    break;
                };
                let (l, r) = (l.map_err(|e| e.to_string())?, r.map_err(|e| e.to_string())?);
                let _ = spool.write_all(&l.to_le_bytes());
                let _ = spool.write_all(&r.to_le_bytes());
                you.push(f32::from(l) / f32::from(i16::MAX));
                them.push(f32::from(r) / f32::from(i16::MAX));
            }
            if you.is_empty() {
                break;
            }
            job.push(&you, &them);
            job.transcribe(false);
        }
        job.finish();
        let turns = job.collected.take().unwrap_or_default();
        if turns.is_empty() {
            return Ok(Some(
                "No speech was found. The transcript is as it was.".to_string(),
            ));
        }
        let mut named = false;
        let day = chrono::Local::now().format("%b %-d");
        store
            .rewrite(note, |body| {
                let (body, kept) = with_transcript(
                    body,
                    &turns,
                    &format!("*Transcribed again with {model} on {day}.*"),
                );
                named = kept;
                body
            })
            .map_err(|e| e.to_string())?;
        drop(spool.flush());
        if settings.notes.meeting_speakers && !named {
            name_speakers(store, note, &raw, one_sided);
        }
        Ok(None)
    })();
    let _ = std::fs::remove_file(&raw);
    result
}

/// `body` with `turns` in place of its transcript, and `said` (what made
/// them) after them. A speaker's name or number in the old transcript is
/// kept for the new turn starting at the same time; returns whether there
/// were any. What follows the transcript (when it ended, the recording's
/// link) stays.
fn with_transcript(body: &str, turns: &[String], said: &str) -> (String, bool) {
    let lines: Vec<&str> = body.lines().collect();
    let aside = |l: &str| {
        let l = l.trim();
        (l.len() > 1 && l.starts_with('*') && l.ends_with('*') && !l.starts_with("**"))
            || l.starts_with("[Recording](")
    };
    // Where the transcript starts: its first turn, or else the notes at the
    // end of the note about how it ended.
    let start = lines
        .iter()
        .position(|l| turn_head(l).is_some())
        .unwrap_or_else(|| {
            let mut at = lines.len();
            while at > 0 && (aside(lines[at - 1]) || lines[at - 1].trim().is_empty()) {
                at -= 1;
            }
            at
        });
    let names: std::collections::HashMap<&str, &str> = lines[start..]
        .iter()
        .filter_map(|l| turn_head(l))
        .filter(|(_, who, _)| {
            !who.is_empty() && *who != Side::You.label() && *who != Side::Them.label()
        })
        .map(|(stamp, who, _)| (stamp, who))
        .collect();
    let turns: Vec<String> = turns
        .iter()
        .map(|t| match turn_head(t) {
            Some((stamp, _, said)) if names.contains_key(stamp) => {
                format!("**[{stamp}] {}:**{said}", names[stamp])
            }
            _ => t.clone(),
        })
        .collect();
    let (links, notes): (Vec<&str>, Vec<&str>) = lines[start..]
        .iter()
        .copied()
        .filter(|l| aside(l) && !l.trim().starts_with("*Transcribed again"))
        .partition(|l| l.trim().starts_with("[Recording]("));
    let mut out = lines[..start].join("\n").trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(&turns.join("\n\n"));
    for line in notes.into_iter().chain([said]).chain(links) {
        out.push_str("\n\n");
        out.push_str(line.trim());
    }
    out.push('\n');
    (out, !names.is_empty())
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
    let answer = crate::prompt::ask(
        app,
        "Finish transcribing it?",
        &format!(
            "The {} recorded {when} wasn't finished when Splashdown closed. Its recording is still here.",
            if spool.app.is_some() {
                "audio"
            } else {
                "meeting"
            }
        ),
        &["Finish", "Delete it", "Later"],
    );
    if answer == Some(1) {
        let _ = std::fs::remove_file(&raw);
        let _ = std::fs::remove_file(meta);
        return Ok(());
    }
    if answer != Some(0) {
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
    job.one_sided = spool.app.is_some();
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
    finish(app, &store, note, &raw, &settings, spool.app.is_some());
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

/// An app that plays sound, for the picker.
#[derive(Debug, Clone, Serialize)]
pub struct AudioAppItem {
    id: String,
    name: String,
    playing: bool,
    sources: Vec<u32>,
}

/// The apps that play sound, to pick one to transcribe.
#[tauri::command]
pub async fn audio_apps() -> Vec<AudioAppItem> {
    tauri::async_runtime::spawn_blocking(|| {
        splashdown_platform::audio_apps()
            .into_iter()
            .map(|a| AudioAppItem {
                id: a.id,
                name: a.name,
                playing: a.playing,
                sources: a.sources,
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Starts transcribing what an app plays (`sources` from `audio_apps`; none
/// for everything the computer plays).
#[tauri::command]
pub fn meeting_start_app(name: String, sources: Vec<u32>, app: AppHandle) -> Result<(), String> {
    start_recording(&app, Source::App { name, sources }).map(|_| ())
}

/// Starts transcribing the microphone alone.
#[tauri::command]
pub fn meeting_start_microphone(app: AppHandle) -> Result<(), String> {
    start_recording(&app, Source::Microphone).map(|_| ())
}

/// What the user writes beside a meeting while it records: kept under
/// `## My notes` in its note, above the transcript (which only this side
/// writes to until the meeting ends).
#[tauri::command]
pub fn meeting_set_notes(
    note: String,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let (store, _) = crate::notes::store_for(state.settings().notes.folder)?;
    store
        .set_section(&note, MY_NOTES, &text)
        .map_err(|e| e.to_string())
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
    #[ignore = "needs eval/audio (splashdown-cli eval make-audio, on a Mac)"]
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

    /// A meeting's recording transcribed again with a downloaded model.
    /// `SPLASHDOWN_AGAIN_WAV` is a stereo 16 kHz recording (You left, Them
    /// right) and `SPLASHDOWN_AGAIN_MODEL` a downloaded speech model's id.
    #[test]
    #[ignore = "needs a recording and a downloaded speech model"]
    fn transcribes_a_recording_again() {
        let wav = PathBuf::from(std::env::var("SPLASHDOWN_AGAIN_WAV").unwrap());
        let id = std::env::var("SPLASHDOWN_AGAIN_MODEL").unwrap_or("parakeet-v3".into());
        let root = std::env::temp_dir().join(format!("splashdown-again-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Store::new(root.clone());
        store.ensure().unwrap();
        std::fs::create_dir_all(root.join("Attachments")).unwrap();
        std::fs::copy(&wav, root.join("Attachments/Call.wav")).unwrap();
        std::fs::write(
            root.join("Call.md"),
            "---\nsource: meeting\n---\n# Call\n\n**[0:00] Them:** old words\n\n*Ended at 0:23.*\n\n[Recording](Attachments/Call.wav)\n",
        )
        .unwrap();
        let recording = recording_of(&store, "Call.md").expect("the link is found");
        let models = crate::engine::paths().unwrap().models_dir;
        let choice = splashdown_config::SpeechChoice {
            provider: splashdown_config::SpeechProvider::WhisperLocal,
            model: id,
        };
        let speech = crate::session::FileSpeech {
            stt: splashdown_providers::factory::speech(&choice, &Secrets::default(), &models)
                .unwrap(),
            hints: Default::default(),
            cleanup: Default::default(),
        };
        let mut seen = 0.0;
        let stopped = transcribe_again(
            &Settings::default(),
            &store,
            "Call.md",
            &recording,
            &speech,
            "Parakeet v3",
            &mut |done| {
                seen = done;
                true
            },
        )
        .unwrap();
        assert_eq!(stopped, None);
        assert!(seen > 20.0, "{seen}");
        let body = store.read("Call.md").unwrap().body;
        println!("{body}");
        assert!(!body.contains("old words"));
        assert!(body.contains("Them:**"));
        assert!(body.contains("*Transcribed again with Parakeet v3 on "));
        assert!(
            body.trim_end()
                .ends_with("[Recording](Attachments/Call.wav)")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_new_transcript_keeps_names_and_the_rest() {
        let body = "# Weekly sync\n\nWith Dana Lee.\n\n## My notes\n\nBudget!\n\n## Summary\n\nShort.\n\n**[0:01] You:** helo\n\n**[0:05] Dana Lee:** hi ther\n\n*Ended at 0:09.*\n\n*Transcribed again with Whisper on Oct 1.*\n\n[Recording](Attachments/Weekly%20sync.wav)\n";
        let turns = [
            "**[0:01] You:** hello".to_string(),
            "**[0:05] Them:** hi there".to_string(),
            "**[0:08] Them:** bye".to_string(),
        ];
        let (out, named) = with_transcript(
            body,
            &turns,
            "*Transcribed again with Parakeet v3 on Nov 1.*",
        );
        assert!(named);
        assert_eq!(
            out,
            "# Weekly sync\n\nWith Dana Lee.\n\n## My notes\n\nBudget!\n\n## Summary\n\nShort.\n\n**[0:01] You:** hello\n\n**[0:05] Dana Lee:** hi there\n\n**[0:08] Them:** bye\n\n*Ended at 0:09.*\n\n*Transcribed again with Parakeet v3 on Nov 1.*\n\n[Recording](Attachments/Weekly%20sync.wav)\n"
        );
        // Nobody spoke the first time.
        let (out, named) = with_transcript(
            "# Call\n\n*Ended at 0:30.*\n\n[Recording](Attachments/Call.wav)\n",
            &turns[..1],
            "*Again.*",
        );
        assert!(!named);
        assert_eq!(
            out,
            "# Call\n\n**[0:01] You:** hello\n\n*Ended at 0:30.*\n\n*Again.*\n\n[Recording](Attachments/Call.wav)\n"
        );
    }

    #[test]
    fn the_other_side_is_told_apart() {
        let body = "# Call\n\n**[0:00] Them:** Hi.\n\n**[0:05] You:** Hello.\n\n**[0:09] Them:** I'm Sam.\n\n**[0:20] Them:** And I'm Dana.\n";
        // Sam until 0:15, Dana after.
        let spans = [(3, 0.0, 4.5), (3, 9.0, 15.0), (1, 20.0, 26.0)];
        assert_eq!(
            label_speakers(body, &spans, Some("Them")),
            "# Call\n\n**[0:00] Speaker 1:** Hi.\n\n**[0:05] You:** Hello.\n\n**[0:09] Speaker 1:** I'm Sam.\n\n**[0:20] Speaker 2:** And I'm Dana.\n"
        );
        // One voice: left as it was.
        assert_eq!(label_speakers(body, &[(1, 0.0, 30.0)], Some("Them")), body);
        // A recording of the room: unlabelled turns get labels.
        let room = "**[0:00]** Today: cells.\n\n**[0:30]** A question?\n";
        assert_eq!(
            label_speakers(room, &[(1, 0.0, 25.0), (2, 30.0, 34.0)], None),
            "**[0:00] Speaker 1:** Today: cells.\n\n**[0:30] Speaker 2:** A question?\n"
        );
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
