//! Downloadable local models: speech models run by
//! transcribe.cpp, and the built-in text models run by llama.cpp. Both are
//! GGUF files fetched from Hugging Face when the user asks, never bundled.
//! Downloading doesn't need the `local-speech` feature, so the app can fetch
//! a model before loading it.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use splashdown_core::ProviderError;

/// Local speech-to-text models, run by transcribe.cpp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechModel {
    /// NVIDIA Nemotron streaming, English: transcribes while you speak, so
    /// the text is ready moments after you stop.
    NemotronEn,
    /// NVIDIA Nemotron 3.5 streaming: the same, in 32 languages. The default.
    Nemotron,
    /// NVIDIA Parakeet TDT v2, English, transcribed when you stop.
    ParakeetV2,
    /// NVIDIA Parakeet TDT v3, 25 European languages.
    ParakeetV3,
    /// OpenAI Whisper base.en: small and English-only.
    WhisperBaseEn,
    /// OpenAI Whisper small.en.
    WhisperSmallEn,
    /// OpenAI Whisper large-v3-turbo, 4-bit: about 100 languages.
    WhisperTurbo,
}

impl SpeechModel {
    /// In order of preference, so the default comes first.
    pub const ALL: [SpeechModel; 7] = [
        SpeechModel::Nemotron,
        SpeechModel::NemotronEn,
        SpeechModel::ParakeetV2,
        SpeechModel::ParakeetV3,
        SpeechModel::WhisperBaseEn,
        SpeechModel::WhisperSmallEn,
        SpeechModel::WhisperTurbo,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SpeechModel::NemotronEn => "nemotron-en",
            SpeechModel::Nemotron => "nemotron",
            SpeechModel::ParakeetV2 => "parakeet-v2",
            SpeechModel::ParakeetV3 => "parakeet-v3",
            SpeechModel::WhisperBaseEn => "base.en",
            SpeechModel::WhisperSmallEn => "small.en",
            SpeechModel::WhisperTurbo => "large-v3-turbo",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        // The whisper.cpp-era id of the turbo model.
        let id = if id == "large-v3-turbo-q5_0" {
            "large-v3-turbo"
        } else {
            id
        };
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    /// The model's Hugging Face repo and file.
    fn source(self) -> (&'static str, &'static str) {
        match self {
            SpeechModel::NemotronEn => (
                "nemotron-speech-streaming-en-0.6b",
                "nemotron-speech-streaming-en-0.6b-Q4_K_M.gguf",
            ),
            SpeechModel::Nemotron => (
                "nemotron-3.5-asr-streaming-0.6b",
                "nemotron-3.5-asr-streaming-0.6b-Q4_K_M.gguf",
            ),
            SpeechModel::ParakeetV2 => ("parakeet-tdt-0.6b-v2", "parakeet-tdt-0.6b-v2-Q4_K_M.gguf"),
            SpeechModel::ParakeetV3 => ("parakeet-tdt-0.6b-v3", "parakeet-tdt-0.6b-v3-Q4_K_M.gguf"),
            SpeechModel::WhisperBaseEn => ("whisper-base.en", "whisper-base.en-Q8_0.gguf"),
            SpeechModel::WhisperSmallEn => ("whisper-small.en", "whisper-small.en-Q8_0.gguf"),
            SpeechModel::WhisperTurbo => (
                "whisper-large-v3-turbo",
                "whisper-large-v3-turbo-Q5_K_M.gguf",
            ),
        }
    }

    pub fn file_name(self) -> &'static str {
        self.source().1
    }

    pub fn url(self) -> String {
        let (repo, file) = self.source();
        format!("https://huggingface.co/handy-computer/{repo}-gguf/resolve/main/{file}")
    }

    /// Approximate download size in MB, for UI.
    pub fn approx_mb(self) -> u32 {
        match self {
            SpeechModel::NemotronEn | SpeechModel::ParakeetV2 => 475,
            SpeechModel::Nemotron => 495,
            SpeechModel::ParakeetV3 => 485,
            SpeechModel::WhisperBaseEn => 84,
            SpeechModel::WhisperSmallEn => 269,
            SpeechModel::WhisperTurbo => 619,
        }
    }

    /// Only understands English.
    pub fn english_only(self) -> bool {
        matches!(
            self,
            SpeechModel::NemotronEn
                | SpeechModel::ParakeetV2
                | SpeechModel::WhisperBaseEn
                | SpeechModel::WhisperSmallEn
        )
    }

    /// Transcribes while the user speaks.
    pub fn streams(self) -> bool {
        matches!(self, SpeechModel::NemotronEn | SpeechModel::Nemotron)
    }

    /// What loading needs to know before the file is read: streaming
    /// models stay on the CPU, where each small piece is quickest.
    #[cfg(feature = "local-speech")]
    pub fn profile(self) -> crate::SpeechProfile {
        crate::SpeechProfile {
            cpu: self.streams(),
            english_only: self.english_only(),
            streams: self.streams(),
        }
    }

    /// A short description, for UI.
    pub fn label(self) -> &'static str {
        match self {
            SpeechModel::NemotronEn => "Nemotron · English · text as you speak",
            SpeechModel::Nemotron => "Nemotron 3.5 · 32 languages · text as you speak",
            SpeechModel::ParakeetV2 => "Parakeet v2 · English",
            SpeechModel::ParakeetV3 => "Parakeet v3 · 25 European languages",
            SpeechModel::WhisperBaseEn => "Whisper base · English · smallest",
            SpeechModel::WhisperSmallEn => "Whisper small · English",
            SpeechModel::WhisperTurbo => "Whisper large-v3-turbo · about 100 languages",
        }
    }

    /// The model's license, which the app shows next to it.
    pub fn license(self) -> &'static str {
        match self {
            SpeechModel::NemotronEn => "NVIDIA Open Model License",
            SpeechModel::Nemotron => "OpenMDW-1.1",
            SpeechModel::ParakeetV2 | SpeechModel::ParakeetV3 => "CC-BY-4.0 (NVIDIA)",
            SpeechModel::WhisperBaseEn
            | SpeechModel::WhisperSmallEn
            | SpeechModel::WhisperTurbo => "MIT (OpenAI)",
        }
    }

    pub fn path_in(self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.file_name())
    }

    /// Whether the model is already downloaded to `models_dir`.
    pub fn is_downloaded(self, models_dir: &Path) -> bool {
        self.path_in(models_dir).is_file()
    }

    /// Downloads the model into `models_dir`. `progress` receives
    /// `(downloaded_bytes, total_bytes)`.
    pub async fn download(
        self,
        models_dir: &Path,
        progress: impl FnMut(u64, Option<u64>),
    ) -> Result<PathBuf, ProviderError> {
        download_file(&self.url(), &self.path_in(models_dir), progress).await
    }
}

/// Built-in local text models: GGUF files from Hugging Face, run by
/// llama.cpp (feature `llm-local`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalModel {
    /// Qwen3 4B Instruct (2507), 4-bit. Apache-2.0. ~2.5 GB.
    Qwen3_4B,
    /// Qwen3 1.7B, 4-bit, thinking off (by `splashdown-llm`). Apache-2.0.
    /// ~1.1 GB. For machines with about 8 GB of memory.
    Qwen3_1_7B,
}

impl LocalModel {
    pub const ALL: [LocalModel; 2] = [LocalModel::Qwen3_4B, LocalModel::Qwen3_1_7B];

    pub fn id(self) -> &'static str {
        match self {
            LocalModel::Qwen3_4B => "qwen3-4b",
            LocalModel::Qwen3_1_7B => "qwen3-1.7b",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn file_name(self) -> &'static str {
        match self {
            LocalModel::Qwen3_4B => "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            LocalModel::Qwen3_1_7B => "Qwen3-1.7B-Q4_K_M.gguf",
        }
    }

    pub fn url(self) -> String {
        let repo = match self {
            LocalModel::Qwen3_4B => "unsloth/Qwen3-4B-Instruct-2507-GGUF",
            LocalModel::Qwen3_1_7B => "unsloth/Qwen3-1.7B-GGUF",
        };
        format!(
            "https://huggingface.co/{repo}/resolve/main/{}",
            self.file_name()
        )
    }

    pub fn path_in(self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.file_name())
    }

    /// Approximate download size in MB, for UI.
    pub fn approx_mb(self) -> u32 {
        match self {
            LocalModel::Qwen3_4B => 2500,
            LocalModel::Qwen3_1_7B => 1100,
        }
    }

    /// Approximate memory while loaded, in MB, for UI (measured on a Mac;
    /// CPU-only runs use more).
    pub fn approx_ram_mb(self) -> u32 {
        match self {
            LocalModel::Qwen3_4B => 3000,
            LocalModel::Qwen3_1_7B => 1700,
        }
    }

    /// A short description, for UI.
    pub fn label(self) -> &'static str {
        match self {
            LocalModel::Qwen3_4B => "Qwen3 4B · best quality",
            LocalModel::Qwen3_1_7B => "Qwen3 1.7B · smaller, faster",
        }
    }

    pub fn is_downloaded(self, models_dir: &Path) -> bool {
        self.path_in(models_dir).is_file()
    }

    pub async fn download(
        self,
        models_dir: &Path,
        progress: impl FnMut(u64, Option<u64>),
    ) -> Result<PathBuf, ProviderError> {
        download_file(&self.url(), &self.path_in(models_dir), progress).await
    }
}

/// How long a download may go without receiving anything before it
/// counts as stalled (and is tried again).
const STALL: Duration = Duration::from_secs(45);
/// Tries per download. A connection that drops or stalls carries on from
/// where it stopped, so a big model isn't started over.
const ATTEMPTS: u32 = 5;

/// Downloads `url` to `dest` (via a `.part` file, so an interrupted
/// download never looks complete), unless it's already there. A `.part`
/// left by an earlier try (the app closed, the network dropped) is
/// carried on from, not started over.
async fn download_file(
    url: &str,
    dest: &Path,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, ProviderError> {
    download_with(url, dest, STALL, Duration::from_secs(2), progress).await
}

async fn download_with(
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
    Ok(dest.to_path_buf())
}

/// Why one try at a download failed.
enum Failed {
    /// The network: worth trying again, from where it stopped.
    Again(ProviderError),
    /// Not worth trying again (no such file, the disk is full).
    Stop(ProviderError),
}

/// One try: fetches what `part` doesn't have yet and adds it.
async fn fetch(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    stall: Duration,
    progress: &mut impl FnMut(u64, Option<u64>),
) -> Result<(), Failed> {
    let again = |e: String| Failed::Again(ProviderError::Network(e));
    let have = std::fs::metadata(part).map_or(0, |m| m.len());
    let mut request = client.get(url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut resp = tokio::time::timeout(stall, request.send())
        .await
        .map_err(|_| again("the server didn't answer".into()))?
        .map_err(|e| again(e.to_string()))?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        // The partial file is no use (the file changed?): start over.
        let _ = std::fs::remove_file(part);
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
    let start = if resumed { have } else { 0 };
    let total = resp.content_length().map(|n| n + start);
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
async fn rename(part: &Path, dest: &Path) -> Result<(), ProviderError> {
    let mut tries = 0;
    loop {
        match std::fs::rename(part, dest) {
            Ok(()) => return Ok(()),
            Err(e) if tries < 20 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(e) => return Err(io_error(e)),
        }
    }
}

/// Downloads a catalog model (speech or local text) by id.
/// NVIDIA's streaming Sortformer: who spoke when, for up to four speakers.
/// Tells the other people on a meeting apart. NVIDIA Open Model License,
/// 8-bit, about 139 MB.
pub struct Diarizer;

impl Diarizer {
    pub const ID: &'static str = "sortformer";
    pub const FILE: &'static str = "diar_streaming_sortformer_4spk-v2.1-Q8_0.gguf";
    /// Approximate download size in MB, for UI.
    pub const APPROX_MB: u32 = 139;

    pub fn url() -> String {
        format!(
            "https://huggingface.co/handy-computer/diar_streaming_sortformer_4spk-v2.1-gguf/resolve/main/{}",
            Self::FILE
        )
    }

    pub fn path_in(models_dir: &Path) -> PathBuf {
        models_dir.join(Self::FILE)
    }

    pub fn is_downloaded(models_dir: &Path) -> bool {
        Self::path_in(models_dir).is_file()
    }
}

/// Finds notes by what they mean rather than the words they use, for Ask.
/// Qwen3 Embedding 0.6B, 8-bit, run by `splashdown-llm`; over 100
/// languages. Apache-2.0, about 639 MB.
pub struct Embedder;

impl Embedder {
    pub const ID: &'static str = "qwen3-embedding";
    pub const FILE: &'static str = "Qwen3-Embedding-0.6B-Q8_0.gguf";
    /// Approximate download size in MB, for UI.
    pub const APPROX_MB: u32 = 639;

    pub fn url() -> String {
        format!(
            "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/main/{}",
            Self::FILE
        )
    }

    pub fn path_in(models_dir: &Path) -> PathBuf {
        models_dir.join(Self::FILE)
    }

    pub fn is_downloaded(models_dir: &Path) -> bool {
        Self::path_in(models_dir).is_file()
    }
}

pub async fn download_by_id(
    id: &str,
    models_dir: &Path,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, ProviderError> {
    if id == Diarizer::ID {
        download_file(&Diarizer::url(), &Diarizer::path_in(models_dir), progress).await
    } else if id == Embedder::ID {
        download_file(&Embedder::url(), &Embedder::path_in(models_dir), progress).await
    } else if let Some(m) = SpeechModel::from_id(id) {
        m.download(models_dir, progress).await
    } else if let Some(m) = LocalModel::from_id(id) {
        m.download(models_dir, progress).await
    } else {
        Err(ProviderError::Unavailable(format!(
            "unknown model \"{id}\""
        )))
    }
}

fn io_error(e: std::io::Error) -> ProviderError {
    ProviderError::Unavailable(format!("couldn't write model file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for m in SpeechModel::ALL {
            assert_eq!(SpeechModel::from_id(m.id()), Some(m));
        }
        for m in LocalModel::ALL {
            assert_eq!(LocalModel::from_id(m.id()), Some(m));
        }
        assert_eq!(
            SpeechModel::from_id("large-v3-turbo-q5_0"),
            Some(SpeechModel::WhisperTurbo)
        );
        assert_eq!(
            SpeechModel::NemotronEn.url(),
            "https://huggingface.co/handy-computer/nemotron-speech-streaming-en-0.6b-gguf/resolve/main/nemotron-speech-streaming-en-0.6b-Q4_K_M.gguf"
        );
    }

    /// What the test server does with each request, in turn (the last one
    /// repeats).
    #[derive(Clone, Copy)]
    enum Serve {
        /// The file, or the rest of it from a `Range`.
        Rest,
        /// The whole file whatever was asked for.
        Whole,
        /// Promises the file but sends this many bytes and hangs up.
        CutAfter(usize),
        /// Sends this many bytes, then nothing for a second.
        StallAfter(usize),
        NotFound,
    }

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
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse().unwrap();
                    }
                    line.clear();
                }
                let len = body.len();
                let head = |status: &str, from: usize| {
                    let range = if from > 0 {
                        format!("Content-Range: bytes {from}-{}/{len}\r\n", len - 1)
                    } else {
                        String::new()
                    };
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{range}Connection: close\r\n\r\n",
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
                    Serve::Rest => conn
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
