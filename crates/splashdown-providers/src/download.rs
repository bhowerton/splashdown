//! Downloading a model file: fetched in parts that resume, retried when a
//! connection drops or stalls, written beside the target and moved into
//! place only when whole (even while another program holds the old file
//! open, on Windows).

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use splashdown_core::ProviderError;

/// How long a download may go without receiving anything before it
/// counts as stalled (and is tried again).
pub(super) const STALL: Duration = Duration::from_secs(45);

/// Tries per download. A connection that drops or stalls carries on from
/// where it stopped, so a big model isn't started over.
pub(super) const ATTEMPTS: u32 = 5;

/// Downloads `url` to `dest` (via a `.part` file, so an interrupted
/// download never looks complete), unless it's already there. A `.part`
/// left by an earlier try (the app closed, the network dropped) is
/// carried on from, not started over.
pub(super) async fn download_file(
    url: &str,
    dest: &Path,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, ProviderError> {
    download_with(url, dest, STALL, Duration::from_secs(2), progress).await
}

pub(super) async fn download_with(
    url: &str,
    dest: &Path,
    stall: Duration,
    pause: Duration,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, ProviderError> {
    if dest.is_file() {
        return Ok(dest.to_path_buf());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(io_error)?;
    }
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    let mut attempt = 1;
    loop {
        match fetch(&client, url, &part, stall, &mut progress).await {
            Ok(()) => break,
            Err(Failed::Again(e)) if attempt < ATTEMPTS => {
                tracing::info!("model download, try {attempt} of {ATTEMPTS}: {e}; trying again");
                tokio::time::sleep(pause * attempt).await;
                attempt += 1;
            }
            Err(Failed::Again(e) | Failed::Stop(e)) => return Err(e),
        }
    }
    rename(&part, dest).await?;
    let _ = std::fs::remove_file(tag_file(&part));
    Ok(dest.to_path_buf())
}

/// Beside a `.part`: what the server said of the file it is part of.
fn tag_file(part: &Path) -> PathBuf {
    let mut name = part.as_os_str().to_owned();
    name.push(".etag");
    PathBuf::from(name)
}

/// What the server said of a file when its download started: its tag for
/// that version of the file (a strong `ETag`, if it gave one) and its
/// length. With these the rest is only ever added to the same version,
/// and a part that already has all of it is known to.
#[derive(Debug, Default, PartialEq, Eq)]
struct Started {
    tag: Option<String>,
    total: Option<u64>,
}

impl Started {
    /// A line each: the tag (or nothing), then the length (or nothing).
    fn read(file: &Path) -> Self {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let mut lines = text.lines().map(str::trim);
        Started {
            tag: lines.next().filter(|t| !t.is_empty()).map(str::to_string),
            total: lines.next().and_then(|n| n.parse().ok()),
        }
    }

    fn write(&self, file: &Path) {
        let total = self.total.map(|n| n.to_string()).unwrap_or_default();
        let text = format!("{}\n{total}\n", self.tag.as_deref().unwrap_or_default());
        let _ = std::fs::write(file, text);
    }
}

/// The server's strong tag for what it's sending, if it gave one (a weak
/// one, `W/"…"`, doesn't promise the same bytes).
fn strong_tag(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|t| !t.is_empty() && !t.starts_with("W/"))
        .map(str::to_string)
}

/// Why one try at a download failed.
pub(super) enum Failed {
    /// The network: worth trying again, from where it stopped.
    Again(ProviderError),
    /// Not worth trying again (no such file, the disk is full).
    Stop(ProviderError),
}

/// One try: fetches what `part` doesn't have yet and adds it.
pub(super) async fn fetch(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    stall: Duration,
    progress: &mut impl FnMut(u64, Option<u64>),
) -> Result<(), Failed> {
    let again = |e: String| Failed::Again(ProviderError::Network(e));
    let have = std::fs::metadata(part).map_or(0, |m| m.len());
    let tag_file = tag_file(part);
    let started = Started::read(&tag_file);
    // The part has all of it: an earlier run finished downloading and
    // couldn't put the file in place. Nothing to fetch.
    if have > 0 && started.total == Some(have) {
        progress(have, Some(have));
        return Ok(());
    }
    let mut request = client.get(url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
        // Only the rest of the file this part is of: a server that goes
        // by this sends the new file whole if it has been replaced since.
        // (Not all do, so the tag is compared below as well.)
        if let Some(tag) = &started.tag {
            request = request.header(reqwest::header::IF_RANGE, tag);
        }
    }
    let mut resp = tokio::time::timeout(stall, request.send())
        .await
        .map_err(|_| again("the server didn't answer".into()))?
        .map_err(|e| again(e.to_string()))?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        // Nothing comes after what the part has. If that's because it has
        // all of it (an earlier run finished downloading and couldn't put
        // the file in place), it's done; it isn't fetched again.
        let whole = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().strip_prefix("bytes */"))
            .and_then(|n| n.trim().parse::<u64>().ok());
        if have > 0 && whole == Some(have) {
            progress(have, Some(have));
            return Ok(());
        }
        // Otherwise the partial file is no use (the file changed): start over.
        let _ = std::fs::remove_file(part);
        let _ = std::fs::remove_file(&tag_file);
        return Err(again("the partial download didn't match".into()));
    }
    if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(again(format!("the server answered {status}")));
    }
    if !status.is_success() {
        return Err(Failed::Stop(ProviderError::Network(format!(
            "the server answered {status}"
        ))));
    }
    // Only carry on from `have` if the server sends the rest from there.
    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT
        && resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with(&format!("bytes {have}-")));
    let tag = strong_tag(&resp);
    if resumed
        && let (Some(was), Some(now)) = (&started.tag, &tag)
        && was != now
    {
        // The file was replaced since the part was begun: its end
        // doesn't belong to this beginning. From the start, next try.
        let _ = std::fs::remove_file(part);
        let _ = std::fs::remove_file(&tag_file);
        return Err(again("the file changed since the download began".into()));
    }
    let start = if resumed { have } else { 0 };
    let total = resp.content_length().map(|n| n + start);
    if !resumed {
        // A fresh start: note which version of the file this is.
        Started { tag, total }.write(&tag_file);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(part)
        .map_err(|e| Failed::Stop(io_error(e)))?;
    let mut file = BufWriter::with_capacity(1 << 20, file);
    let mut done = start;
    progress(done, total);
    loop {
        let chunk = tokio::time::timeout(stall, resp.chunk())
            .await
            .map_err(|_| again("the download stalled".into()))?
            .map_err(|e| again(e.to_string()))?;
        let Some(chunk) = chunk else { break };
        file.write_all(&chunk)
            .map_err(|e| Failed::Stop(io_error(e)))?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().map_err(|e| Failed::Stop(io_error(e)))?;
    if total.is_some_and(|t| t != done) {
        return Err(again("the download was cut off".into()));
    }
    Ok(())
}

/// Moves the finished download into place. Windows can refuse for a
/// moment while a virus scanner reads the new file, so this tries a few
/// times.
pub(super) async fn rename(part: &Path, dest: &Path) -> Result<(), ProviderError> {
    let mut tries = 0;
    loop {
        match std::fs::rename(part, dest) {
            Ok(()) => return Ok(()),
            Err(e) if tries < 20 && in_use(&e) => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(e) => return Err(io_error(e)),
        }
    }
}

/// Whether a file operation failed because another program (a virus
/// scanner, the search indexer, a sync app) has the file open for a
/// moment: access denied, or Windows' sharing and lock violations
/// (errors 32 and 33), which Rust doesn't count as denied.
pub fn in_use(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(32 | 33))
}

/// `std::fs::rename`, tried again for up to `wait` while the file is in
/// use (`in_use`).
pub fn rename_patiently(from: &Path, to: &Path, wait: Duration) -> std::io::Result<()> {
    let started = std::time::Instant::now();
    loop {
        match std::fs::rename(from, to) {
            Err(e) if in_use(&e) && started.elapsed() < wait => {
                std::thread::sleep(Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

pub(super) fn io_error(e: std::io::Error) -> ProviderError {
    ProviderError::Unavailable(format!("couldn't write model file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Diarizer;
    use std::time::Duration;

    /// What the test server does with each request, in turn (the last one
    /// repeats).
    #[derive(Clone, Copy)]
    enum Serve {
        /// The file, or the rest of it from a `Range`.
        Rest,
        /// The rest from a `Range`, whichever version of the file the
        /// request says its part is of (as some hosts do).
        RestOfAnything,
        /// The whole file whatever was asked for.
        Whole,
        /// Promises the file but sends this many bytes and hangs up.
        CutAfter(usize),
        /// Sends this many bytes, then nothing for a second.
        StallAfter(usize),
        NotFound,
    }

    /// The tag the server gives its file.
    const TAG: &str = "\"v1\"";

    /// A one-file HTTP server on localhost; returns the file's address.
    fn server(body: Vec<u8>, script: Vec<Serve>) -> String {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model.gguf", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for (i, conn) in listener.incoming().enumerate() {
                let Ok(mut conn) = conn else { return };
                let mut from = 0;
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    let header = line.to_ascii_lowercase();
                    if let Some(v) = header.strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse().unwrap();
                    }
                    // The rest of another version of the file: all of
                    // this one instead.
                    if let Some(v) = header.strip_prefix("if-range:")
                        && v.trim() != TAG
                        && !matches!(script[i.min(script.len() - 1)], Serve::RestOfAnything)
                    {
                        from = 0;
                    }
                    line.clear();
                }
                let len = body.len();
                if from >= len && matches!(script[i.min(script.len() - 1)], Serve::Rest) {
                    let _ = conn.write_all(
                        format!(
                            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    );
                    continue;
                }
                let head = |status: &str, from: usize| {
                    let range = if from > 0 {
                        format!("Content-Range: bytes {from}-{}/{len}\r\n", len - 1)
                    } else {
                        String::new()
                    };
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nETag: {TAG}\r\n{range}Connection: close\r\n\r\n",
                        len - from
                    )
                };
                let ok = |from: usize| {
                    if from > 0 {
                        "206 Partial Content"
                    } else {
                        "200 OK"
                    }
                };
                let _ = match script[i.min(script.len() - 1)] {
                    Serve::Rest | Serve::RestOfAnything => conn
                        .write_all(head(ok(from), from).as_bytes())
                        .and_then(|()| conn.write_all(&body[from..])),
                    Serve::Whole => conn
                        .write_all(head("200 OK", 0).as_bytes())
                        .and_then(|()| conn.write_all(&body)),
                    Serve::CutAfter(n) => conn
                        .write_all(head(ok(from), from).as_bytes())
                        .and_then(|()| conn.write_all(&body[from..from + n])),
                    Serve::StallAfter(n) => conn
                        .write_all(head(ok(from), from).as_bytes())
                        .and_then(|()| conn.write_all(&body[from..from + n]))
                        .map(|()| std::thread::sleep(Duration::from_secs(1))),
                    Serve::NotFound => conn.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    ),
                };
            }
        });
        url
    }

    /// An empty folder of its own for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("splashdown-download-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file() -> Vec<u8> {
        (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect()
    }

    async fn get(url: &str, dest: &Path) -> Result<PathBuf, ProviderError> {
        let short = Duration::from_millis(300);
        download_with(url, dest, short, Duration::from_millis(10), |_, _| {}).await
    }

    #[tokio::test]
    async fn a_dropped_download_carries_on_where_it_stopped() {
        let dir = scratch("dropped");
        let dest = dir.join("model.gguf");
        let url = server(file(), vec![Serve::CutAfter(70_000), Serve::Rest]);
        get(&url, &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), file());
        assert!(!dir.join("model.gguf.part").exists());
        assert!(!dir.join("model.gguf.part.etag").exists());
    }

    #[tokio::test]
    async fn a_stalled_download_is_tried_again() {
        let dir = scratch("stalled");
        let dest = dir.join("model.gguf");
        let url = server(file(), vec![Serve::StallAfter(50_000), Serve::Rest]);
        get(&url, &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), file());
    }

    #[tokio::test]
    async fn a_part_from_an_earlier_run_is_carried_on_from() {
        let dir = scratch("part");
        let dest = dir.join("model.gguf");
        std::fs::write(dir.join("model.gguf.part"), &file()[..120_000]).unwrap();
        let url = server(file(), vec![Serve::Rest]);
        let mut first = None;
        download_with(&url, &dest, STALL, Duration::ZERO, |done, _| {
            first.get_or_insert(done);
        })
        .await
        .unwrap();
        assert_eq!(first, Some(120_000), "carried on from the part");
        assert_eq!(std::fs::read(&dest).unwrap(), file());
    }

    #[tokio::test]
    async fn a_finished_part_is_put_in_place_not_fetched_again() {
        // An earlier run downloaded all of it and couldn't move it.
        let dir = scratch("finished");
        let dest = dir.join("model.gguf");
        std::fs::write(dir.join("model.gguf.part"), file()).unwrap();
        let url = server(file(), vec![Serve::Rest]);
        let mut seen = Vec::new();
        download_with(&url, &dest, STALL, Duration::ZERO, |done, total| {
            seen.push((done, total));
        })
        .await
        .unwrap();
        assert_eq!(seen, [(200_000, Some(200_000))], "nothing was fetched");
        assert_eq!(std::fs::read(&dest).unwrap(), file());
        assert!(!dir.join("model.gguf.part").exists());
    }

    #[tokio::test]
    async fn a_part_of_a_file_since_replaced_isn_t_added_to() {
        let dir = scratch("replaced");
        let dest = dir.join("model.gguf");
        // 120,000 bytes of the file as it was last week.
        // A server that sends the new file whole when told which version
        // the part is of, and one that sends the rest regardless.
        for (name, script) in [
            ("goes-by-it", vec![Serve::Rest]),
            ("doesn-t", vec![Serve::RestOfAnything, Serve::Rest]),
        ] {
            let dir = scratch(name);
            let dest = dir.join("model.gguf");
            std::fs::write(dir.join("model.gguf.part"), vec![9u8; 120_000]).unwrap();
            std::fs::write(dir.join("model.gguf.part.etag"), "\"last-week\"\n150000\n").unwrap();
            let url = server(file(), script);
            get(&url, &dest).await.unwrap();
            assert_eq!(
                std::fs::read(&dest).unwrap(),
                file(),
                "{name}: not old and new joined"
            );
            assert!(!dir.join("model.gguf.part.etag").exists());
        }
        let _ = (dir, dest);
    }

    #[tokio::test]
    async fn a_part_known_to_be_whole_needs_no_server() {
        // What the server said when the download began is kept beside the
        // part: with all of it there, nothing is asked for again.
        let dir = scratch("whole");
        let dest = dir.join("model.gguf");
        std::fs::write(dir.join("model.gguf.part"), file()).unwrap();
        std::fs::write(dir.join("model.gguf.part.etag"), "\"v1\"\n200000\n").unwrap();
        let url = server(file(), vec![Serve::NotFound]);
        get(&url, &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), file());
        assert_eq!(
            Started::read(Path::new("/nowhere/at/all")),
            Started::default()
        );
    }

    #[tokio::test]
    async fn a_server_that_cant_resume_starts_over() {
        let dir = scratch("restart");
        let dest = dir.join("model.gguf");
        std::fs::write(dir.join("model.gguf.part"), b"something else").unwrap();
        let url = server(file(), vec![Serve::Whole]);
        get(&url, &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), file());
    }

    #[tokio::test]
    async fn a_missing_file_fails_at_once_and_leaves_nothing() {
        let dir = scratch("missing");
        let dest = dir.join("model.gguf");
        let url = server(file(), vec![Serve::NotFound, Serve::Rest]);
        let e = get(&url, &dest).await.unwrap_err();
        assert!(e.to_string().contains("404"), "{e}");
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn a_download_that_keeps_failing_gives_up() {
        let dir = scratch("gives-up");
        let dest = dir.join("model.gguf");
        let url = server(file(), vec![Serve::CutAfter(10)]);
        assert!(get(&url, &dest).await.is_err());
        assert!(!dest.exists(), "never looks complete");
    }

    /// The real thing: a model from Hugging Face, cut off part way, then
    /// carried on. `cargo test -p splashdown-providers -- --ignored
    /// --nocapture resumes_a_real_download`.
    #[tokio::test]
    #[ignore = "downloads about 139 MB"]
    async fn resumes_a_real_download() {
        let dir = scratch("real");
        let dest = Diarizer::path_in(&dir);
        let cut = tokio::time::timeout(
            Duration::from_secs(3),
            download_file(&Diarizer::url(), &dest, |_, _| {}),
        )
        .await;
        assert!(cut.is_err(), "finished in 3 s: make the cut sooner");
        let had = std::fs::metadata(dir.join(format!("{}.part", Diarizer::FILE)))
            .unwrap()
            .len();
        let mut first = None;
        download_file(&Diarizer::url(), &dest, |done, total| {
            first.get_or_insert((done, total));
        })
        .await
        .unwrap();
        let (from, total) = first.unwrap();
        println!("cut at {had} bytes; carried on from {from} of {total:?}");
        assert_eq!(from, had);
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), total.unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
