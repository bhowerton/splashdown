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
//!
//! A finished meeting can be carried on: more is recorded into its note,
//! timed from where it stopped (`carry_on`).
//!
//! This file starts and stops recordings and has the commands; the rest is
//! in `meeting/`: `record.rs` (the audio), `transcript.rs` (turns into the
//! note, speakers, each turn's language), `voices.rs` (recognizing people
//! by voice), `summary.rs`, `again.rs` (transcribing a recording again)
//! and `recover.rs` (after a crash).

mod again;
mod record;
mod recover;
mod summary;
mod transcript;
pub mod voices;

use splashdown_core::sync::Locked;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use splashdown_audio::file::{TurnCutter, timestamp};
use splashdown_audio::live::LiveAudio;
use splashdown_config::{Destination, MeetingSummary, Secrets, Settings};
use splashdown_core::meeting::{Meeting, Paragraph, Segment, Side, Turn};
use tauri::{AppHandle, Emitter, Manager};

use crate::notes::store::{Origin, Store};
use crate::state::AppState;

pub(crate) use self::again::*;
use self::record::*;
pub use self::recover::*;
pub(crate) use self::summary::summary_ai;
use self::summary::*;
use self::transcript::*;

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
/// ...and for a change of microphone.
const MICROPHONE_CHECK: Duration = Duration::from_secs(2);
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
    /// Recording a call (`Source::Meeting`), not an app's sound or the
    /// microphone alone.
    call: bool,
}

fn running() -> MutexGuard<'static, Option<Running>> {
    static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
    RUNNING.locked()
}

/// Whether anything is recording: a meeting, an app's sound or the
/// microphone.
pub fn is_running() -> bool {
    running().is_some()
}

/// What's recording, if anything: `Some(true)` for a call, `Some(false)`
/// for an app's sound or the microphone alone.
pub fn recording_call() -> Option<bool> {
    running().as_ref().map(|r| r.call)
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

/// Starts a meeting that's known to be `event` on the calendar (the offer
/// to join it was answered with Record): its note is named from it and
/// says who's invited, whether or not meetings are named from the calendar.
pub(crate) fn start_for(app: &AppHandle, event: Event) -> Result<String, String> {
    start_with(app, Source::Meeting, Some(event), None)
}

/// Starts recording `source`, as `start`.
pub fn start_recording(app: &AppHandle, source: Source) -> Result<String, String> {
    start_with(app, source, None, None)
}

/// Carries on the finished meeting (or microphone recording) in note
/// `rel`: records more into it, timed from where it stopped, and
/// summarizes all of it again at the end.
pub fn carry_on(app: &AppHandle, rel: &str) -> Result<String, String> {
    let folder = app.state::<AppState>().settings().notes.folder;
    let (store, _) = crate::notes::store_for(folder)?;
    let note = store.read(rel).map_err(|e| e.to_string())?;
    let source = match note.source.as_str() {
        "meeting" => Source::Meeting,
        RECORDING => Source::Microphone,
        _ => return Err("Only a meeting or a microphone recording can be carried on.".into()),
    };
    start_with(app, source, None, Some(rel.to_string()))
}

/// Starts recording `source`: into a new note (named from `event`, when
/// the meeting is known to be that one on the calendar), or into the note
/// to carry on.
fn start_with(
    app: &AppHandle,
    source: Source,
    event: Option<Event>,
    carry_on: Option<String>,
) -> Result<String, String> {
    // Held until the thread's handle is stored, so a thread that ends at
    // once (and clears the slot) can't leave its handle on a later meeting.
    let mut slot = running();
    if slot.is_some() {
        return Err("A meeting is already recording.".into());
    }
    let run = Running {
        call: matches!(source, Source::Meeting),
        ..Running::default()
    };
    let (stop, keep) = (run.stop.clone(), run.keep.clone());
    let handle = app.clone();
    let thread = std::thread::Builder::new()
        .name("meeting".into())
        .spawn(move || {
            // A panic mustn't leave a meeting marked as recording for good.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                record(&handle, &source, event, &stop, &keep, carry_on.as_deref())
            }))
            .unwrap_or_else(|_| Err("the recording stopped unexpectedly".into()));
            if let Err(e) = result {
                tracing::warn!("meeting: {e}");
                tell(&handle, e, true);
            }
            *running() = None;
            show(&handle, None);
        })
        .map_err(|e| e.to_string())?;
    *slot = Some(Running {
        thread: Some(thread),
        ..run
    });
    drop(slot);
    Ok("Starting a meeting".into())
}

/// The calendar event on now, or the one about to start: its title and
/// who else is invited.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Event {
    pub(crate) title: String,
    pub(crate) people: Vec<String>,
}

#[cfg(target_os = "macos")]
pub(crate) fn calendar_event() -> Option<Event> {
    let event = tauri::async_runtime::block_on(splashdown_apple_shim::calendar_now())?;
    tracing::info!(
        "a calendar event is on ({} other people)",
        event.people.len()
    );
    Some(Event {
        title: event.title,
        people: event.people,
    })
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn calendar_event() -> Option<Event> {
    None
}

/// Asks for the calendar (Settings → Meetings): whether it's allowed.
#[tauri::command]
pub async fn calendar_access() -> bool {
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::calendar_access().await;
    #[cfg(not(target_os = "macos"))]
    false
}

/// Asks for Reminders, when a hook that adds reminders is saved: whether
/// it's allowed.
#[tauri::command]
pub async fn reminders_access() -> bool {
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::reminders_access().await;
    #[cfg(not(target_os = "macos"))]
    false
}

/// Whether Reminders has a list called `name`, when a hook that adds to
/// it is saved. True where it can't be told (no access is said by
/// `reminders_access`), so that only a list known to be missing stops the
/// save.
#[tauri::command]
pub async fn reminders_has_list(name: String) -> bool {
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::reminders_list_exists(name.trim())
        .await
        .unwrap_or(true);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = name;
        true
    }
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

// ---------------------------------------------------------------- commands

#[tauri::command]
// Async: a synchronous command runs on the main thread, and building a
// window from there deadlocks on Windows (the meeting pill, Notes).
pub async fn meeting_toggle(app: AppHandle) {
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
pub async fn meeting_start_app(
    name: String,
    sources: Vec<u32>,
    app: AppHandle,
) -> Result<(), String> {
    start_recording(&app, Source::App { name, sources }).map(|_| ())
}

/// Starts transcribing the microphone alone.
#[tauri::command]
pub async fn meeting_start_microphone(app: AppHandle) -> Result<(), String> {
    start_recording(&app, Source::Microphone).map(|_| ())
}

/// Records more into the finished meeting in note `path`.
#[tauri::command]
pub async fn meeting_carry_on(path: String, app: AppHandle) -> Result<(), String> {
    carry_on(&app, &path).map(|_| ())
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
                words: Vec::new(),
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

    #[test]
    #[ignore = "needs eval/audio (splashdown-cli eval make-audio, on a Mac)"]
    fn a_note_carried_on_is_timed_from_where_it_stopped() {
        let root = std::env::temp_dir().join(format!("splashdown-carry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Store::new(root.clone());
        let note = store
            .create_in(
                "Meetings",
                "# Meeting\n\n**[0:04] Them:** Hi.\n\n*Ended at 12:40.*\n",
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
        job.offset = ended_at(&store.read(&note.path).unwrap().body);
        let you = [clip("question"), vec![0.0; RATE]].concat();
        let them = vec![0.0; you.len()];
        for (y, t) in you.chunks(RATE / 2).zip(them.chunks(RATE / 2)) {
            job.push(y, t);
            job.transcribe(false);
        }
        assert_eq!(job.finish(), 0);
        let body = store.read(&note.path).unwrap().body;
        let _ = std::fs::remove_dir_all(&root);
        let turns: Vec<&str> = body.lines().filter(|l| l.starts_with("**[")).collect();
        assert_eq!(turns.len(), 2, "{body}");
        assert!(turns[1].starts_with("**[12:40] You:** Heard"), "{body}");
        // The recording's own length is its own: the spool goes by it.
        assert!(job.recorded() < Duration::from_secs(60));
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
    fn finds_how_far_a_finished_note_goes() {
        // Past its last turn, to where the recording was stopped.
        let body = "# Zoom meeting\n\n## Summary\n\n*Met at 9:30.*\n\n## Transcript\n\n**[0:04] Them:** Hi.\n\n**[5:10] You:** Bye.\n\n*Ended at 5:23.*\n\n[Recording](Attachments/Zoom%20meeting.wav)\n";
        assert_eq!(ended_at(body), Duration::from_secs(323));
        // Carried on once already: the later part counts.
        let twice = format!(
            "{body}\n*Carried on at 5:23, Nov 3, 2:00 PM.*\n\n**[5:30] You:** Back.\n\n*Stopped at 1:02:07: nobody had spoken for a while.*\n"
        );
        assert_eq!(ended_at(&twice), Duration::from_secs(3727));
        // No line about its end (an older note): its last turn.
        assert_eq!(
            ended_at("# Call\n\n**[0:04] Them:** Hi.\n"),
            Duration::from_secs(4)
        );
        assert_eq!(ended_at("# Empty\n\n*Nothing at all.*\n"), Duration::ZERO);
    }

    #[test]
    fn a_later_part_joins_the_recording_where_the_note_had_got_to() {
        let dir = std::env::temp_dir().join(format!("splashdown-join-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (wav, raw) = (dir.join("Meeting.wav"), dir.join("part.raw"));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        // One second kept, of a note that ends at 1.5 s.
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        for _ in 0..RATE * 2 {
            writer.write_sample(7i16).unwrap();
        }
        writer.finalize().unwrap();
        // Half a second more.
        let more: Vec<u8> = std::iter::repeat_n(9i16.to_le_bytes(), RATE)
            .flatten()
            .collect();
        std::fs::write(&raw, more).unwrap();
        extend_recording(&wav, &raw, Duration::from_millis(1500)).unwrap();
        let samples: Vec<i16> = hound::WavReader::open(&wav)
            .unwrap()
            .samples::<i16>()
            .map(Result::unwrap)
            .collect();
        let left_over = dir.join("Meeting.wav.part").exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(samples.len(), RATE * 4, "two seconds, two sides");
        assert!(samples[..RATE * 2].iter().all(|&s| s == 7));
        assert!(samples[RATE * 2..RATE * 3].iter().all(|&s| s == 0));
        assert!(samples[RATE * 3..].iter().all(|&s| s == 9));
        assert!(!left_over);
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
