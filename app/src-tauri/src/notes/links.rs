//! Transcribing a link: downloading the audio of a file's address, or of a
//! page such as a video through the link helper (yt-dlp, see
//! `link_helper.rs`), into the transcription queue, with a Stop that ends
//! the download.

use splashdown_core::sync::Locked;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::AppHandle;

use super::files::*;

/// The longest download a link may be.
pub(super) const MAX_DOWNLOAD: u64 = 2 << 30;

/// Transcribe → A link…: a link to an audio or video file is downloaded;
/// a page (YouTube, a podcast's page) goes through yt-dlp if it's
/// installed. The download is transcribed like a dropped file, then
/// deleted.
#[tauri::command]
pub async fn notes_transcribe_url(url: String, app: AppHandle) -> Result<(), String> {
    let url = url.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("That isn't a web link (it should start with https://).".into());
    }
    let models_dir = crate::engine::paths()?.models_dir;
    let direct = url_extension(&url).is_some_and(|ext| EXTENSIONS.contains(&ext.as_str()));
    let helper = (!direct)
        .then(|| super::link_helper::find(&models_dir))
        .flatten();
    // A page needs the link helper; the Notes window asks before it's
    // downloaded (`notes_install_link_helper`), then tries again.
    if !direct && helper.is_none() && super::link_helper::can_download() {
        return Err(NEEDS_HELPER.into());
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
    let item = match helper {
        Some(helper) => {
            let (url, dir) = (url.clone(), dir.clone());
            tauri::async_runtime::spawn_blocking(move || {
                if super::link_helper::update_due(&helper) {
                    super::link_helper::update(&helper);
                }
                match download_page(&helper, &url, &dir) {
                    // Refused: sites change, and yt-dlp may need its
                    // latest version. Once more after updating.
                    Err(e) if e == REFUSED && helper.managed => {
                        super::link_helper::update(&helper);
                        download_page(&helper, &url, &dir)
                    }
                    other => other,
                }
            })
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

/// What `notes_transcribe_url` says when a page needs the link helper.
pub(super) const NEEDS_HELPER: &str = "needs-link-helper";

/// A site refused yt-dlp's download.
pub(super) const REFUSED: &str = "The site refused the download. Try again later.";

/// Downloads the link helper (see `link_helper`), after the Notes window
/// asked. The Notes bar shows it, with Stop.
#[tauri::command]
pub async fn notes_install_link_helper(app: AppHandle) -> Result<(), String> {
    let models_dir = crate::engine::paths()?.models_dir;
    CANCEL.store(false, Ordering::Relaxed);
    let shown = !queue().running;
    if shown {
        show(
            &app,
            Some(Status {
                file: format!(
                    "the link helper (about {} MB)",
                    super::link_helper::APPROX_MB
                ),
                note: String::new(),
                done: 0.0,
                total: None,
                waiting: 0,
                place: String::new(),
                downloading: true,
            }),
        );
    }
    let result = super::link_helper::install(&models_dir).await;
    if shown && !queue().running {
        show(&app, None);
    }
    result
}

/// Whether Stop was pressed during a download.
pub(super) fn download_cancelled() -> bool {
    CANCEL.load(Ordering::Relaxed)
}

/// Set by Stop: a download in progress ends.
pub(super) static CANCEL: AtomicBool = AtomicBool::new(false);

/// yt-dlp while it runs, for Stop to end it.
pub(super) static DOWNLOADER: Mutex<Option<std::process::Child>> = Mutex::new(None);

pub(super) const STOPPED: &str = "Download stopped.";

/// A link, short, for the Notes bar: its host and the end of its path.
pub(super) fn url_label(url: &str) -> String {
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
pub(super) const DOWNLOAD_IDLE: Duration = Duration::from_secs(60);

/// A file extension for a download of this content type, from those the
/// decoder reads ("audio/mpeg" is "mp3").
pub(super) fn extension_for(kind: &str) -> &'static str {
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
pub(super) fn url_extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let last = path.rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    Some(ext.to_lowercase())
}

/// Downloads a link that is itself an audio or video file.
pub(super) async fn download_file(url: &str, dir: &Path) -> Result<Item, String> {
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
        again: None,
        watched: None,
        reply: None,
    })
}

/// Downloads a page's audio with yt-dlp.
pub(super) fn download_page(
    helper: &super::link_helper::Helper,
    url: &str,
    dir: &Path,
) -> Result<Item, String> {
    let mut command = std::process::Command::new(&helper.yt_dlp);
    // The JavaScript engine YouTube needs.
    if let Some(qjs) = &helper.qjs {
        command
            .arg("--js-runtimes")
            .arg(format!("quickjs:{}", qjs.display()));
    }
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
            tracing::warn!("yt-dlp failed: {}", err.trim());
            return Err(yt_dlp_error(&err));
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
        again: None,
        watched: None,
        reply: None,
    })
}

/// What to tell the user when yt-dlp fails, from what it wrote.
pub(super) fn yt_dlp_error(stderr: &str) -> String {
    let reason = stderr
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or("it gave no file")
        .trim()
        .trim_start_matches("ERROR: ");
    // Sites change their pages often, and an older yt-dlp then gets
    // refused; the caller updates it and tries again.
    if reason.contains("403")
        || reason.contains("Forbidden")
        || reason.contains("Sign in to confirm")
    {
        return REFUSED.into();
    }
    format!("yt-dlp couldn't get it: {reason}")
}

/// Runs `command` to the end, unless Stop ends it first.
pub(super) fn run_stoppable(
    mut command: std::process::Command,
) -> Result<std::process::Output, String> {
    // yt-dlp is a console program: no console window on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
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
    *DOWNLOADER.locked() = Some(child);
    let status = loop {
        std::thread::sleep(Duration::from_millis(200));
        let mut slot = DOWNLOADER.locked();
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

/// Stop: ends the running job (keeping what's done) and drops the waiting
/// ones.
#[tauri::command]
pub fn notes_transcribe_stop() {
    let mut queue = queue();
    // Watched files dropped here were turned down, not left unfinished:
    // they aren't taken again.
    let dropped: Vec<PathBuf> = queue
        .waiting
        .iter()
        .filter(|item| item.watched.is_some())
        .map(|item| item.path.clone())
        .collect();
    queue.waiting.clear();
    queue.stop.store(true, Ordering::Relaxed);
    drop(queue);
    super::watch::settled(&dropped);
    // And a link still downloading.
    CANCEL.store(true, Ordering::Relaxed);
    if let Some(mut child) = DOWNLOADER.locked().take() {
        end_tree(&mut child);
        let _ = child.wait();
    }
}

/// `CREATE_NO_WINDOW`.
#[cfg(windows)]
pub(super) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Ends `child` and what it started: yt-dlp on Windows is a launcher that
/// runs the real program as a child of its own, which would otherwise
/// keep downloading (and keep its folder locked) after Stop.
pub(super) fn end_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    let _ = child.kill();
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
        let started = std::time::Instant::now();
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
        assert_eq!(
            url_label("https://example.org/ep/42.mp3?x=1"),
            "example.org/ep/42.mp3"
        );
        assert_eq!(
            url_label(&format!("https://example.org/{}/talk.mp3", "a/".repeat(30))),
            "example.org/…/talk.mp3"
        );
        assert_eq!(
            yt_dlp_error("ERROR: unable to download video data: HTTP Error 403: Forbidden\n"),
            REFUSED
        );
        assert_eq!(
            yt_dlp_error("ERROR: Video unavailable\n"),
            "yt-dlp couldn't get it: Video unavailable"
        );
        // The server's content type never becomes part of a path.
        assert_eq!(extension_for("audio/mpeg"), "mp3");
        assert_eq!(extension_for("audio/x-m4a; charset=binary"), "m4a");
        assert_eq!(extension_for("audio/x\\..\\..\\thesis.docx"), "bin");
    }
}
