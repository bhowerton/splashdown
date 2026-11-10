//! File transcription: an audio or video file dropped on Notes,
//! or picked from its menu, becomes a transcript note, written a paragraph
//! at a time as each stretch of about 30 s is transcribed. One file runs at
//! a time; the rest wait their turn. A meeting's kept recording can be
//! transcribed again the same way, with another speech model.

use splashdown_core::sync::Locked;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use splashdown_audio::file::{AudioFile, timestamp};
use splashdown_config::{Secrets, SpeechChoice, SpeechProvider};
use splashdown_core::{AudioClip, SpeechToText, SttHints, Usage};
use splashdown_providers::SpeechModel;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use super::store::Origin;
use super::{WINDOW, store};
use crate::engine;
use crate::state::AppState;

/// App event with the running job (`Status`), or null when there's none.
const PROGRESS: &str = "notes-transcribing";
/// App event with something to tell the user (`Message`): a file that
/// can't be read, a job that stopped.
const MESSAGE: &str = "notes-transcribe-message";
/// App event while files are dragged over the window: true on the way in,
/// false when they leave or are dropped.
pub const DRAGGING: &str = "notes-dragging";

/// What the file picker offers and drops accept. Symphonia decides in the
/// end; these are the kinds it reads.
pub(super) const EXTENSIONS: &[&str] = &[
    "wav", "wave", "aif", "aiff", "aifc", "caf", "mp3", "m4a", "m4b", "aac", "mp4", "m4v", "mov",
    "flac", "ogg", "oga", "webm", "mkv", "mka",
];

/// The job running now, for the Notes window.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// The file's name.
    pub(super) file: String,
    /// The transcript note.
    pub(super) note: String,
    /// Seconds of the file transcribed.
    pub(super) done: f64,
    /// The file's length in seconds, when its header says.
    pub(super) total: Option<f64>,
    /// Files waiting after this one.
    pub(super) waiting: usize,
    /// "on this computer", "with OpenAI".
    pub(super) place: String,
    /// A link is still downloading (there's no note yet).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(super) downloading: bool,
}

#[derive(Clone, Serialize)]
struct Message {
    text: String,
    error: bool,
}

/// A file to transcribe.
pub(super) struct Item {
    pub(super) path: PathBuf,
    /// Downloaded from this link: the note says so, and the file is
    /// deleted afterwards.
    pub(super) link: Option<Link>,
    /// A meeting's recording, transcribed again into its note.
    pub(super) again: Option<Again>,
    /// Found in this watched folder: the note goes where it says, with
    /// subtitles beside the file if it asks.
    pub(super) watched: Option<splashdown_config::WatchedFolder>,
    /// Told the note's path when it's done, and why it stopped early if it
    /// did (a Shortcuts action waiting for it).
    pub(super) reply: Option<std::sync::mpsc::Sender<Result<Done, String>>>,
}

/// A transcription that ran: its note, and why it stopped early if it did
/// (what was done is kept).
#[derive(Debug, Clone)]
pub(crate) struct Done {
    pub(crate) note: String,
    pub(crate) stopped: Option<String>,
}

pub(super) struct Again {
    pub(super) note: String,
    /// A downloaded speech model's id, or `None` for the one in Settings.
    pub(super) model: Option<String>,
}

/// A downloaded link is deleted, with the folder it was downloaded into,
/// once it's transcribed, or dropped from the queue by Stop.
impl Drop for Item {
    fn drop(&mut self) {
        if self.link.is_some()
            && let Some(dir) = self.path.parent()
        {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

pub(super) struct Link {
    pub(super) url: String,
    /// The page's title, when the downloader reports one.
    pub(super) title: Option<String>,
}

#[derive(Default)]
pub(super) struct Queue {
    pub(super) waiting: VecDeque<Item>,
    pub(super) running: bool,
    pub(super) status: Option<Status>,
    /// Set by Stop; cleared when the next job starts.
    pub(super) stop: Arc<AtomicBool>,
}

pub(super) fn queue() -> MutexGuard<'static, Queue> {
    static QUEUE: OnceLock<Mutex<Queue>> = OnceLock::new();
    QUEUE.get_or_init(Mutex::default).locked()
}

pub(super) fn is_media(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

pub(super) fn tell(app: &AppHandle, text: String, error: bool) {
    let _ = app.emit_to(WINDOW, MESSAGE, Message { text, error });
}

pub(super) fn show(app: &AppHandle, status: Option<Status>) {
    let mut queue = queue();
    queue.status = status.map(|s| Status {
        waiting: queue.waiting.len(),
        ..s
    });
    let _ = app.emit_to(WINDOW, PROGRESS, queue.status.clone());
}

/// Files dropped on the Notes window: the audio and video ones are
/// transcribed.
pub fn dropped(app: &AppHandle, paths: Vec<PathBuf>) {
    let (media, other): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| is_media(p));
    if let Some(first) = other.first() {
        let more = match other.len() {
            1 => String::new(),
            n => format!(" and {} more", n - 1),
        };
        tell(
            app,
            format!(
                "Only audio and video files can be transcribed, so {}{more} was left out.",
                file_name(first)
            ),
            true,
        );
    }
    add(app, media);
}

/// Queues files and starts the worker if it isn't running.
fn add(app: &AppHandle, files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    add_items(
        app,
        files
            .into_iter()
            .map(|path| Item {
                path,
                link: None,
                again: None,
                watched: None,
                reply: None,
            })
            .collect(),
    );
}

pub(super) fn add_items(app: &AppHandle, items: Vec<Item>) {
    let mut waiting = queue();
    waiting.waiting.extend(items);
    if waiting.running {
        let status = waiting.status.clone();
        drop(waiting);
        show(app, status);
        return;
    }
    waiting.running = true;
    drop(waiting);
    let app = app.clone();
    let started = std::thread::Builder::new()
        .name("transcribe-file".into())
        .spawn(move || work(app));
    if let Err(e) = started {
        tracing::error!("couldn't start transcribing: {e}");
        queue().running = false;
    }
}

/// Transcribes waiting files until there are none.
fn work(app: AppHandle) {
    loop {
        let (mut item, stop) = {
            let mut queue = queue();
            let Some(item) = queue.waiting.pop_front() else {
                queue.running = false;
                queue.status = None;
                let _ = app.emit_to(WINDOW, PROGRESS, None::<Status>);
                return;
            };
            queue.stop.store(false, Ordering::Relaxed);
            (item, queue.stop.clone())
        };
        let path = &item.path;
        let name = item
            .link
            .as_ref()
            .and_then(|l| l.title.clone())
            .unwrap_or_else(|| file_name(path));
        // A panic (a decoder on a damaged file) mustn't leave the queue
        // marked running with the note locked until a restart.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &item.again {
            Some(again) => transcribe_again(&app, path, again, &stop).map(|stopped| Done {
                note: again.note.clone(),
                stopped,
            }),
            None => transcribe(&app, path, item.link.as_ref(), item.watched.as_ref(), &stop),
        }))
        .unwrap_or_else(|_| {
            tracing::error!("transcribing a file panicked");
            Err("something went wrong reading it".into())
        });
        match &result {
            Ok(Done {
                stopped: None,
                note,
            }) => {
                // A new transcript, not one done again.
                if item.again.is_none() {
                    crate::hooks::note_finished(
                        &app,
                        splashdown_config::HookEvent::Transcript,
                        note,
                    );
                }
            }
            Ok(Done {
                stopped: Some(stopped),
                ..
            }) => tell(
                &app,
                format!("{name}: {stopped}"),
                !stop.load(Ordering::Relaxed),
            ),
            Err(e) => tell(&app, format!("Couldn't transcribe {name}: {e}"), true),
        }
        if item.watched.is_some() {
            match &result {
                // A note came of it, whole or as far as it got.
                Ok(_) => super::watch::settled(std::slice::from_ref(&item.path)),
                // Nothing did: it's tried again when the app next starts.
                // Logged, since nobody may have the Notes window open.
                Err(e) => tracing::warn!("a watched file couldn't be transcribed: {e}"),
            }
        }
        if let Some(reply) = item.reply.take() {
            let _ = reply.send(result);
        }
        // (A link's download goes when `item` does.)
    }
}

/// Transcribes one file into a new note.
fn transcribe(
    app: &AppHandle,
    path: &Path,
    link: Option<&Link>,
    watched: Option<&splashdown_config::WatchedFolder>,
    stop: &AtomicBool,
) -> Result<Done, String> {
    let state = app.state::<AppState>();
    let mut file = AudioFile::open(path).map_err(|e| e.to_string())?;
    let speech = state.session.file_speech()?;
    let stt_name = speech.stt.name();
    let (place, local) = place(&stt_name);
    let (store, _) = store(&state)?;

    let name = file_name(path);
    let stem = link
        .and_then(|l| l.title.clone())
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| name.clone());
    let length = file
        .duration()
        .map(|d| format!(" ({})", length(d)))
        .unwrap_or_default();
    let source = match link {
        Some(link) => format!("<{}>", link.url),
        None => format!("**{}**", name.replace('*', "\\*")),
    };
    let header = format!("# {stem}\n\nTranscribed from {source}{length} {place}.\n");
    // The transcript isn't the note the user was working on, so captures
    // set to "the last note" don't go into it.
    let last = store.last().map(|(path, _)| path);
    let note = store
        .create_in(
            watched.map_or("", |w| w.into.trim()),
            &header,
            chrono::Local::now(),
            &Origin {
                source: Some("file"),
                file: Some(&name),
                ..Origin::default()
            },
        )
        .map_err(|e| e.to_string())?;
    store.set_last(last.as_deref());
    let mut status = Status {
        file: link.and_then(|l| l.title.clone()).unwrap_or(name),
        note: note.path.clone(),
        done: 0.0,
        total: file.duration().map(|d| d.as_secs_f64()),
        waiting: 0,
        place: place.to_string(),
        downloading: false,
    };
    show(app, Some(status.clone()));

    let started = Instant::now();
    let mut reached = Duration::ZERO;
    let mut heard = false;
    // For subtitles: every word's time while the model gives them, and
    // where each line's speech is.
    let mut timings = super::subtitles::Timings::default();
    let mut all_timed = true;
    let stopped = loop {
        if stop.load(Ordering::Relaxed) {
            break Some(format!("Stopped at {}.", timestamp(reached)));
        }
        if !local && engine::budget_reached(&state.settings()) {
            break Some(format!(
                "Stopped at {}: the spending limit was reached.",
                timestamp(reached)
            ));
        }
        let piece = match file.next_piece() {
            Ok(Some(piece)) => piece,
            Ok(None) => break None,
            Err(e) => {
                break Some(format!(
                    "Stopped at {}: {}.",
                    timestamp(reached),
                    sentence(&e.to_string())
                ));
            }
        };
        if let Some((clip, offset)) = splashdown_audio::prepare_at(&piece.clip, speech.cleanup) {
            match transcribe_piece(speech.stt.as_ref(), &clip, &speech.hints) {
                Ok((text, words, usage)) => {
                    crate::spend::shared()
                        .record(&[(stt_name.clone(), usage)], chrono::Local::now());
                    if !text.is_empty() {
                        store
                            .add_paragraph(
                                &note.path,
                                &splashdown_core::meeting::Turn {
                                    stamp: &timestamp(piece.start),
                                    who: "",
                                    said: &text,
                                }
                                .line(),
                            )
                            .map_err(|e| e.to_string())?;
                        heard = true;
                        let spoke = (piece.start + offset).as_millis() as u64;
                        timings
                            .spans
                            .push((spoke, spoke + clip.duration().as_millis() as u64));
                        all_timed &= !words.is_empty();
                        timings.words.extend(words.into_iter().map(|w| {
                            splashdown_core::subtitles::TimedWord {
                                start_ms: w.start_ms + spoke,
                                end_ms: w.end_ms + spoke,
                                ..w
                            }
                        }));
                    }
                }
                Err(e) => {
                    break Some(format!(
                        "Stopped at {}: {}.",
                        timestamp(piece.start),
                        sentence(&e)
                    ));
                }
            }
        }
        reached = piece.start + piece.clip.duration();
        status.done = reached.as_secs_f64();
        show(app, Some(status.clone()));
    };

    tracing::info!(
        "transcribed {:.0} s of a file in {:.1} s{}",
        reached.as_secs_f64(),
        started.elapsed().as_secs_f64(),
        if stopped.is_some() { " (stopped)" } else { "" }
    );
    let last = match &stopped {
        Some(reason) => Some(format!("*{reason}*")),
        None if !heard => Some("*No speech was found in it.*".to_string()),
        None => None,
    };
    if heard {
        if !all_timed {
            timings.words.clear();
        }
        let saved = store
            .id(&note.path)
            .ok_or_else(|| "the note has no id".to_string())
            .and_then(|id| super::subtitles::save(&id, &timings));
        if let Err(e) = saved {
            tracing::warn!("couldn't keep the times for subtitles: {e}");
        }
    }
    if let Some(last) = last {
        store
            .add_paragraph(&note.path, &last)
            .map_err(|e| e.to_string())?;
    }
    if heard
        && stopped.is_none()
        && watched.is_some_and(|w| w.subtitles)
        && let Err(e) = super::subtitles::save_beside(&store, &note.path, path)
    {
        tell(
            app,
            format!("Couldn't save subtitles for {}: {e}", file_name(path)),
            true,
        );
    }
    Ok(Done {
        note: note.path,
        stopped,
    })
}

/// Transcribes a meeting's recording again into its note, with the model
/// asked for. Returns why it stopped early, if it did.
fn transcribe_again(
    app: &AppHandle,
    recording: &Path,
    again: &Again,
    stop: &AtomicBool,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let settings = state.settings();
    let mut speech = state.session.file_speech()?;
    let model = match &again.model {
        Some(id) => {
            let chosen = SpeechModel::from_id(id).ok_or("That speech model isn't known.")?;
            let models = engine::paths()?.models_dir;
            let choice = SpeechChoice {
                provider: SpeechProvider::WhisperLocal,
                model: id.clone(),
            };
            speech.stt =
                splashdown_providers::factory::speech(&choice, &Secrets::default(), &models)
                    .map_err(|e| e.to_string())?;
            short_label(chosen)
        }
        None => {
            let name = speech.stt.name();
            name.split_once('/')
                .map_or(name.clone(), |(_, m)| m.to_string())
        }
    };
    let (place, local) = place(&speech.stt.name());
    if !local && engine::budget_reached(&settings) {
        return Err("the spending limit was reached".into());
    }
    let (store, _) = store(&state)?;
    let title = store
        .read(&again.note)
        .map(|n| super::markdown::title_of(&n.body))
        .unwrap_or_default();
    let total = hound::WavReader::open(recording)
        .ok()
        .map(|r| f64::from(r.duration()) / f64::from(r.spec().sample_rate));
    let mut status = Status {
        file: format!("{title} again"),
        note: again.note.clone(),
        done: 0.0,
        total,
        waiting: 0,
        place: place.to_string(),
        downloading: false,
    };
    show(app, Some(status.clone()));
    let mut shown = Instant::now();
    let mut progress = |done: f64| {
        // Twice a second is plenty for a progress bar.
        if shown.elapsed() >= Duration::from_millis(500) {
            shown = Instant::now();
            status.done = done;
            show(app, Some(status.clone()));
        }
        !stop.load(Ordering::Relaxed)
    };
    let stopped = crate::meeting::transcribe_again(
        &settings,
        &store,
        &again.note,
        recording,
        &speech,
        &model,
        &mut progress,
    )?;
    if stopped.is_none() {
        tell(
            app,
            format!("{title} was transcribed again with {model}."),
            false,
        );
    }
    Ok(stopped)
}

/// "Parakeet v3", from "Parakeet v3 · 25 European languages".
fn short_label(model: SpeechModel) -> String {
    model
        .label()
        .split(" · ")
        .next()
        .unwrap_or_default()
        .to_string()
}

/// One piece transcribed, tried twice before giving up: its text, its
/// words with their times (when the model gives them) and what it cost.
fn transcribe_piece(
    stt: &dyn SpeechToText,
    clip: &AudioClip,
    hints: &SttHints,
) -> Result<(String, Vec<splashdown_core::subtitles::TimedWord>, Usage), String> {
    let attempt = || tauri::async_runtime::block_on(stt.transcribe(clip, hints));
    let transcript = attempt()
        .or_else(|e| {
            tracing::warn!("a piece of the file failed, trying again: {e}");
            attempt()
        })
        .map_err(|e| e.to_string())?;
    Ok((
        transcript.text.trim().to_string(),
        transcript.words,
        transcript.usage,
    ))
}

/// Where the audio goes, from the speech-to-text's name, and whether that's
/// this computer.
pub(crate) fn place(stt_name: &str) -> (&'static str, bool) {
    match stt_name.split('/').next().unwrap_or_default() {
        "local" | "apple" => ("on this computer", true),
        "openai" => ("with OpenAI", false),
        "openrouter" => ("with OpenRouter", false),
        _ => ("with your speech-to-text server", false),
    }
}

/// "35 s", "42 min", "1 h 5 min".
pub(crate) fn length(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", (secs + 30) / 60),
        _ => {
            let mins = (secs + 30) / 60;
            match mins % 60 {
                0 => format!("{} h", mins / 60),
                m => format!("{} h {m} min", mins / 60),
            }
        }
    }
}

/// An error message as the end of a sentence: no final period of its own.
fn sentence(e: &str) -> &str {
    e.trim().trim_end_matches('.')
}

/// A Shortcuts action's file: transcribed into a new note like a dropped
/// one (in turn, with progress in the Notes window), waiting until it's
/// done.
#[cfg(target_os = "macos")]
pub(crate) fn transcribe_and_wait(app: &AppHandle, path: PathBuf) -> Result<Done, String> {
    if !is_media(&path) {
        return Err(format!(
            "{} isn't an audio or video file Splashdown can read.",
            file_name(&path)
        ));
    }
    let (reply, done) = std::sync::mpsc::channel();
    add_items(
        app,
        vec![Item {
            path,
            link: None,
            again: None,
            watched: None,
            reply: Some(reply),
        }],
    );
    done.recv()
        .unwrap_or_else(|_| Err("The transcription was stopped.".into()))
}

/// Transcribe → A file…: a file picker; the files chosen are transcribed.
#[tauri::command]
pub async fn notes_transcribe_pick(app: AppHandle) -> Result<(), String> {
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_title("Transcribe audio or video")
            .add_filter("Audio and video", EXTENSIONS)
            .blocking_pick_files()
    })
    .await
    .map_err(|e| e.to_string())?;
    let files = picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .collect();
    add(&app, files);
    Ok(())
}

/// A speech model a meeting can be transcribed again with.
#[derive(Serialize)]
pub struct AgainModel {
    /// `None` for the one chosen in Settings.
    id: Option<String>,
    label: String,
}

/// The speech models a meeting can be transcribed again with: the one in
/// Settings, then each one downloaded. Empty when the note has no
/// recording kept.
#[tauri::command]
pub fn notes_transcribe_again_models(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgainModel>, String> {
    let (store, _) = store(&state)?;
    if crate::meeting::recording_of(&store, &path).is_none() {
        return Ok(Vec::new());
    }
    let models = engine::paths()?.models_dir;
    let mut list = vec![AgainModel {
        id: None,
        label: "The speech model in Settings".into(),
    }];
    list.extend(
        SpeechModel::ALL
            .into_iter()
            .filter(|m| m.is_downloaded(&models))
            .map(|m| AgainModel {
                id: Some(m.id().to_string()),
                label: m.label().to_string(),
            }),
    );
    Ok(list)
}

/// Transcribes a meeting's kept recording again, with `model` (a
/// downloaded speech model's id, or none for the one in Settings), in
/// place of its transcript. Waits behind other transcriptions.
#[tauri::command]
pub fn notes_transcribe_again(
    path: String,
    model: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (store, _) = store(&state)?;
    super::not_recording(&path)?;
    let recording = crate::meeting::recording_of(&store, &path)
        .ok_or("This note's recording isn't kept, so it can't be transcribed again.")?;
    if queue().status.as_ref().is_some_and(|s| s.note == path)
        || queue()
            .waiting
            .iter()
            .any(|i| i.again.as_ref().is_some_and(|a| a.note == path))
    {
        return Err("This note is already being transcribed.".into());
    }
    add_items(
        &app,
        vec![Item {
            path: recording,
            link: None,
            again: Some(Again { note: path, model }),
            watched: None,
            reply: None,
        }],
    );
    Ok(())
}

/// The running job, when the Notes window loads.
#[tauri::command]
pub fn notes_transcribing() -> Option<Status> {
    queue().status.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_read_naturally() {
        assert_eq!(length(Duration::from_secs(35)), "35 s");
        assert_eq!(length(Duration::from_secs(42 * 60 + 10)), "42 min");
        assert_eq!(length(Duration::from_secs(3600)), "1 h");
        assert_eq!(length(Duration::from_secs(3600 + 5 * 60)), "1 h 5 min");
    }

    #[test]
    fn where_the_audio_goes() {
        assert_eq!(place("local/nemotron-en"), ("on this computer", true));
        assert_eq!(place("apple/speech"), ("on this computer", true));
        assert!(!place("openai/gpt-4o-mini-transcribe").1);
        assert!(!place("custom/whisper-1").1);
    }

    #[test]
    fn only_media_files_are_taken() {
        assert!(is_media(Path::new("/a/Interview.M4A")));
        assert!(is_media(Path::new("clip.mov")));
        assert!(!is_media(Path::new("notes.txt")));
        assert!(!is_media(Path::new("no-extension")));
    }
}
