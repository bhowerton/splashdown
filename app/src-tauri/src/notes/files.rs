//! File transcription: an audio or video file dropped on Notes,
//! or picked from its menu, becomes a transcript note, written a paragraph
//! at a time as each stretch of about 30 s is transcribed. One file runs at
//! a time; the rest wait their turn.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use splashdown_audio::file::{AudioFile, timestamp};
use splashdown_core::{AudioClip, SpeechToText, SttHints, Usage};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use super::store::Origin;
use super::{WINDOW, store};
use crate::commands::AppState;
use crate::engine;

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
const EXTENSIONS: &[&str] = &[
    "wav", "wave", "aif", "aiff", "aifc", "caf", "mp3", "m4a", "m4b", "aac", "mp4", "m4v", "mov",
    "flac", "ogg", "oga", "webm", "mkv", "mka",
];

/// The job running now, for the Notes window.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// The file's name.
    file: String,
    /// The transcript note.
    note: String,
    /// Seconds of the file transcribed.
    done: f64,
    /// The file's length in seconds, when its header says.
    total: Option<f64>,
    /// Files waiting after this one.
    waiting: usize,
    /// "on this computer", "with OpenAI".
    place: String,
}

#[derive(Clone, Serialize)]
struct Message {
    text: String,
    error: bool,
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<PathBuf>,
    running: bool,
    status: Option<Status>,
    /// Set by Stop; cleared when the next job starts.
    stop: Arc<AtomicBool>,
}

fn queue() -> MutexGuard<'static, Queue> {
    static QUEUE: OnceLock<Mutex<Queue>> = OnceLock::new();
    QUEUE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn is_media(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn tell(app: &AppHandle, text: String, error: bool) {
    let _ = app.emit_to(WINDOW, MESSAGE, Message { text, error });
}

fn show(app: &AppHandle, status: Option<Status>) {
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
    let mut waiting = queue();
    waiting.waiting.extend(files);
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
        let (path, stop) = {
            let mut queue = queue();
            let Some(path) = queue.waiting.pop_front() else {
                queue.running = false;
                queue.status = None;
                let _ = app.emit_to(WINDOW, PROGRESS, None::<Status>);
                return;
            };
            queue.stop.store(false, Ordering::Relaxed);
            (path, queue.stop.clone())
        };
        let name = file_name(&path);
        match transcribe(&app, &path, &stop) {
            Ok(None) => {}
            Ok(Some(stopped)) => tell(
                &app,
                format!("{name}: {stopped}"),
                !stop.load(Ordering::Relaxed),
            ),
            Err(e) => tell(&app, format!("Couldn't transcribe {name}: {e}"), true),
        }
    }
}

/// Transcribes one file into a new note. Returns why it stopped early, if
/// it did (what's done is kept).
fn transcribe(app: &AppHandle, path: &Path, stop: &AtomicBool) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let mut file = AudioFile::open(path).map_err(|e| e.to_string())?;
    let speech = state.session.file_speech()?;
    let stt_name = speech.stt.name();
    let (place, local) = place(&stt_name);
    let (store, _) = store(&state)?;

    let name = file_name(path);
    let stem = path
        .file_stem()
        .map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
    let length = file
        .duration()
        .map(|d| format!(" ({})", length(d)))
        .unwrap_or_default();
    let header = format!(
        "# {stem}\n\nTranscribed from **{}**{length} {place}.\n",
        name.replace('*', "\\*")
    );
    let note = store
        .create(
            &header,
            chrono::Local::now(),
            &Origin {
                source: Some("file"),
                file: Some(&name),
                ..Origin::default()
            },
        )
        .map_err(|e| e.to_string())?;
    let mut status = Status {
        file: name,
        note: note.path.clone(),
        done: 0.0,
        total: file.duration().map(|d| d.as_secs_f64()),
        waiting: 0,
        place: place.to_string(),
    };
    show(app, Some(status.clone()));

    let started = Instant::now();
    let mut reached = Duration::ZERO;
    let mut heard = false;
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
        if let Some(clip) = splashdown_audio::prepare_with(&piece.clip, speech.cleanup) {
            match transcribe_piece(speech.stt.as_ref(), &clip, &speech.hints) {
                Ok((text, usage)) => {
                    crate::spend::shared()
                        .record(&[(stt_name.clone(), usage)], chrono::Local::now());
                    if !text.is_empty() {
                        store
                            .add_paragraph(
                                &note.path,
                                &format!("**[{}]** {text}", timestamp(piece.start)),
                            )
                            .map_err(|e| e.to_string())?;
                        heard = true;
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
    if let Some(last) = last {
        store
            .add_paragraph(&note.path, &last)
            .map_err(|e| e.to_string())?;
    }
    Ok(stopped)
}

/// One piece, tried twice before giving up.
fn transcribe_piece(
    stt: &dyn SpeechToText,
    clip: &AudioClip,
    hints: &SttHints,
) -> Result<(String, Usage), String> {
    let attempt = || tauri::async_runtime::block_on(stt.transcribe(clip, hints));
    let transcript = attempt()
        .or_else(|e| {
            tracing::warn!("a piece of the file failed, trying again: {e}");
            attempt()
        })
        .map_err(|e| e.to_string())?;
    Ok((transcript.text.trim().to_string(), transcript.usage))
}

/// Where the audio goes, from the speech-to-text's name, and whether that's
/// this computer.
fn place(stt_name: &str) -> (&'static str, bool) {
    match stt_name.split('/').next().unwrap_or_default() {
        "local" | "apple" => ("on this computer", true),
        "openai" => ("with OpenAI", false),
        "openrouter" => ("with OpenRouter", false),
        _ => ("with your speech-to-text server", false),
    }
}

/// "35 s", "42 min", "1 h 5 min".
fn length(d: Duration) -> String {
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

/// "Transcribe a file…": a file picker; the files chosen are transcribed.
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

/// Stop: ends the running job (keeping what's done) and drops the waiting
/// ones.
#[tauri::command]
pub fn notes_transcribe_stop() {
    let mut queue = queue();
    queue.waiting.clear();
    queue.stop.store(true, Ordering::Relaxed);
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
