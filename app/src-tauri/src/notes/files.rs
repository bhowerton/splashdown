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
    /// A link is still downloading (there's no note yet).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    downloading: bool,
}

#[derive(Clone, Serialize)]
struct Message {
    text: String,
    error: bool,
}

/// A file to transcribe.
struct Item {
    path: PathBuf,
    /// Downloaded from this link: the note says so, and the file is
    /// deleted afterwards.
    link: Option<Link>,
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

struct Link {
    url: String,
    /// The page's title, when the downloader reports one.
    title: Option<String>,
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<Item>,
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
    add_items(
        app,
        files
            .into_iter()
            .map(|path| Item { path, link: None })
            .collect(),
    );
}

fn add_items(app: &AppHandle, items: Vec<Item>) {
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
        let (item, stop) = {
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
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            transcribe(&app, path, item.link.as_ref(), &stop)
        }))
        .unwrap_or_else(|_| {
            tracing::error!("transcribing a file panicked");
            Err("something went wrong reading it".into())
        });
        match result {
            Ok(None) => {}
            Ok(Some(stopped)) => tell(
                &app,
                format!("{name}: {stopped}"),
                !stop.load(Ordering::Relaxed),
            ),
            Err(e) => tell(&app, format!("Couldn't transcribe {name}: {e}"), true),
        }
        // (A link's download goes when `item` does.)
    }
}

/// Transcribes one file into a new note. Returns why it stopped early, if
/// it did (what's done is kept).
fn transcribe(
    app: &AppHandle,
    path: &Path,
    link: Option<&Link>,
    stop: &AtomicBool,
) -> Result<Option<String>, String> {
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
pub(crate) fn place(stt_name: &str) -> (&'static str, bool) {
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

// ---------------------------------------------------------------- links

/// The longest download a link may be.
const MAX_DOWNLOAD: u64 = 2 << 30;

/// "Transcribe a link…": a link to an audio or video file is downloaded;
/// a page (YouTube, a podcast's page) goes through yt-dlp if it's
/// installed. The download is transcribed like a dropped file, then
/// deleted.
#[tauri::command]
pub async fn notes_transcribe_url(url: String, app: AppHandle) -> Result<(), String> {
    let url = url.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("That isn't a web link (it should start with https://).".into());
    }
    // A folder of its own, so two downloads never share a file.
    static DOWNLOADS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = DOWNLOADS.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join("splashdown-links")
        .join(format!("{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    tell(&app, "Downloading…".into(), false);
    // The Notes bar shows it, with Stop, unless a file is already being
    // transcribed (then Stop ends both).
    CANCEL.store(false, Ordering::Relaxed);
    let shown = !queue().running;
    if shown {
        show(
            &app,
            Some(Status {
                file: url_label(&url),
                note: String::new(),
                done: 0.0,
                total: None,
                waiting: 0,
                place: String::new(),
                downloading: true,
            }),
        );
    }
    let direct = url_extension(&url).is_some_and(|ext| EXTENSIONS.contains(&ext.as_str()));
    let item = match yt_dlp().filter(|_| !direct) {
        Some(program) => {
            let (url, dir) = (url.clone(), dir.clone());
            tauri::async_runtime::spawn_blocking(move || download_page(&program, &url, &dir))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r)
        }
        None => download_file(&url, &dir).await,
    };
    if shown && !queue().running {
        show(&app, None);
    }
    match item {
        Ok(item) => {
            add_items(&app, vec![item]);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

/// Set by Stop: a download in progress ends.
static CANCEL: AtomicBool = AtomicBool::new(false);
/// yt-dlp while it runs, for Stop to end it.
static DOWNLOADER: Mutex<Option<std::process::Child>> = Mutex::new(None);

const STOPPED: &str = "Download stopped.";

/// A link, short, for the Notes bar: its host and the end of its path.
fn url_label(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    if rest.chars().count() <= 48 {
        return rest.to_string();
    }
    let host = rest.split('/').next().unwrap_or(rest);
    let last = rest.rsplit('/').next().unwrap_or("");
    format!("{host}/…/{last}")
}

/// How long a download may wait for the next piece.
const DOWNLOAD_IDLE: Duration = Duration::from_secs(60);

/// A file extension for a download of this content type, from those the
/// decoder reads ("audio/mpeg" is "mp3").
fn extension_for(kind: &str) -> &'static str {
    let subtype = kind.split(['/', ';']).nth(1).unwrap_or("").trim();
    match subtype {
        "mpeg" | "mp3" => "mp3",
        "mp4" | "x-m4a" | "m4a" | "aac" => "m4a",
        "wav" | "x-wav" | "wave" => "wav",
        "flac" | "x-flac" => "flac",
        "ogg" => "ogg",
        "webm" => "webm",
        "quicktime" => "mov",
        _ => "bin",
    }
}

/// The extension of the link's file name, lowercased.
fn url_extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let last = path.rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    Some(ext.to_lowercase())
}

/// Downloads a link that is itself an audio or video file.
async fn download_file(url: &str, dir: &Path) -> Result<Item, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("Couldn't download it: {e}"))?;
    let kind = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    let ext = url_extension(url).filter(|e| EXTENSIONS.contains(&e.as_str()));
    if ext.is_none() && !(kind.starts_with("audio/") || kind.starts_with("video/")) {
        return Err(format!(
            "That link is a web page, not an audio or video file. For YouTube and other pages, install yt-dlp ({}) and try again.",
            if cfg!(windows) {
                "winget install yt-dlp"
            } else {
                "brew install yt-dlp"
            }
        ));
    }
    if resp.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
        return Err("That file is over 2 GB.".into());
    }
    // The link's own name is only the note's title; the file gets a fixed
    // one, so nothing in the link (or the server's reply) picks the path.
    let title = url
        .split(['?', '#'])
        .next()
        .and_then(|p| p.rsplit('/').next())
        .map(|n| n.rsplit_once('.').map_or(n, |(stem, _)| stem))
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    let path = dir.join(format!(
        "download.{}",
        ext.as_deref()
            .and_then(|e| EXTENSIONS.iter().find(|x| **x == e))
            .copied()
            .unwrap_or_else(|| extension_for(&kind))
    ));
    let mut out = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut resp = resp;
    let mut written = 0u64;
    while let Some(chunk) = tokio::time::timeout(DOWNLOAD_IDLE, resp.chunk())
        .await
        .map_err(|_| "The download stopped coming.".to_string())?
        .map_err(|e| format!("The download stopped: {e}"))?
    {
        if CANCEL.load(Ordering::Relaxed) {
            return Err(STOPPED.into());
        }
        written += chunk.len() as u64;
        if written > MAX_DOWNLOAD {
            return Err("That file is over 2 GB.".into());
        }
        std::io::Write::write_all(&mut out, &chunk).map_err(|e| e.to_string())?;
    }
    Ok(Item {
        path,
        link: Some(Link {
            url: url.to_string(),
            title,
        }),
    })
}

/// Downloads a page's audio with yt-dlp.
fn download_page(program: &Path, url: &str, dir: &Path) -> Result<Item, String> {
    let mut command = std::process::Command::new(program);
    command
        // `--print` would otherwise skip the download.
        .args([
            "--no-playlist",
            "--playlist-items",
            "1",
            "--match-filter",
            "!is_live",
            "--no-progress",
            "--no-warnings",
            "--no-simulate",
            "--encoding",
            "utf-8",
        ])
        // AAC first: the decoder can't read Opus, YouTube's other audio.
        .args([
            "-f",
            "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/best[ext=mp4]/bestaudio/best",
        ])
        .arg("-o")
        // A plain name: the title comes separately (`--print title`).
        .arg(dir.join("%(id)s.%(ext)s"))
        .args(["--print", "after_move:filepath", "--print", "title"])
        .args(["--max-filesize", "2G", "--"])
        .arg(url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = run_stoppable(command)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines().filter(|l| !l.trim().is_empty());
    // The title is printed before the download, the file's path after.
    let (title, path) = match (lines.next(), lines.next()) {
        (Some(title), Some(path)) => (title.trim(), PathBuf::from(path.trim())),
        _ => {
            let err = String::from_utf8_lossy(&output.stderr);
            let reason = err
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("it gave no file");
            return Err(format!("yt-dlp couldn't get it: {}", reason.trim()));
        }
    };
    if !output.status.success() || !path.exists() {
        return Err("yt-dlp couldn't get it.".into());
    }
    Ok(Item {
        path,
        link: Some(Link {
            url: url.to_string(),
            title: Some(title.to_string()).filter(|t| !t.is_empty()),
        }),
    })
}

/// Runs `command` to the end, unless Stop ends it first.
fn run_stoppable(mut command: std::process::Command) -> Result<std::process::Output, String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("Couldn't run yt-dlp: {e}"))?;
    // Read its output as it comes, or it could fill the pipe and wait.
    let read = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = read(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = read(child.stderr.take().map(|p| Box::new(p) as _));
    *DOWNLOADER.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    let status = loop {
        std::thread::sleep(Duration::from_millis(200));
        let mut slot = DOWNLOADER.lock().unwrap_or_else(|e| e.into_inner());
        let Some(child) = slot.as_mut() else {
            break None;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                slot.take();
                break Some(status);
            }
            Ok(None) => {}
            Err(e) => {
                slot.take();
                return Err(e.to_string());
            }
        }
    };
    let output = std::process::Output {
        status: status.ok_or(STOPPED)?,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    };
    if CANCEL.load(Ordering::Relaxed) {
        return Err(STOPPED.into());
    }
    Ok(output)
}

/// yt-dlp, if it's installed: on the `PATH`, or where Homebrew and winget
/// put it (an app started from the Dock or Start doesn't get the shell's
/// `PATH`).
fn yt_dlp() -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(
            PathBuf::from(local)
                .join("Microsoft")
                .join("WinGet")
                .join("Links"),
        );
    }
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

/// Stop: ends the running job (keeping what's done) and drops the waiting
/// ones.
#[tauri::command]
pub fn notes_transcribe_stop() {
    let mut queue = queue();
    queue.waiting.clear();
    queue.stop.store(true, Ordering::Relaxed);
    drop(queue);
    // And a link still downloading.
    CANCEL.store(true, Ordering::Relaxed);
    if let Some(mut child) = DOWNLOADER.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = child.kill();
        let _ = child.wait();
    }
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
    #[cfg(unix)]
    fn stop_ends_a_download_part_way() {
        CANCEL.store(false, Ordering::Relaxed);
        let mut command = std::process::Command::new("sleep");
        command
            .arg("30")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let started = Instant::now();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(400));
            notes_transcribe_stop();
        });
        assert_eq!(run_stoppable(command).unwrap_err(), STOPPED);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(DOWNLOADER.lock().unwrap().is_none());
    }

    #[test]
    fn links_to_files_are_told_from_pages() {
        let ext = |u: &str| url_extension(u);
        assert_eq!(ext("https://x.org/ep/42.MP3?dl=1").as_deref(), Some("mp3"));
        assert_eq!(
            ext("https://x.org/a.b/talk.m4a#t=30").as_deref(),
            Some("m4a")
        );
        assert_eq!(ext("https://www.youtube.com/watch?v=abc"), None);
        assert_eq!(ext("https://x.org/"), None);
        assert!(yt_dlp().is_none_or(|p| p.is_file()));
        assert_eq!(
            url_label("https://example.org/ep/42.mp3?x=1"),
            "example.org/ep/42.mp3"
        );
        assert_eq!(
            url_label(&format!("https://example.org/{}/talk.mp3", "a/".repeat(30))),
            "example.org/…/talk.mp3"
        );
        // The server's content type never becomes part of a path.
        assert_eq!(extension_for("audio/mpeg"), "mp3");
        assert_eq!(extension_for("audio/x-m4a; charset=binary"), "m4a");
        assert_eq!(extension_for("audio/x\\..\\..\\thesis.docx"), "bin");
    }

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
