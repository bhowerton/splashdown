//! Local speech-to-text with transcribe.cpp: Nemotron, Parakeet
//! and Whisper GGUF models on this computer. Streaming models transcribe
//! while the user speaks and run on the CPU, where each small piece is
//! quickest; the others use Metal on macOS.
//!
//! A model is only loaded once the user has chosen it, and then in the
//! background: the first load after an update compiles the GPU shaders,
//! which takes about 10 s, and dictation mustn't wait for that to start.
//! A stand-in for a failed cloud provider loads only when it's needed.
//!
//! One model does one thing at a time: while a stream runs, the library
//! turns other transcriptions away as busy. They wait their turn here (a
//! file being transcribed waits for a dictation), and a stream that's
//! dropped or too slow stops at once to let them in.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Once, OnceLock, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use splashdown_core::{
    AudioClip, ProviderError, ProviderResult, SpeechStream, SpeechToText, SttHints, Transcript,
    Usage,
};
use transcribe_cpp::{
    CommitPolicy, Feature, Model, ModelOptions, RunOptions, StreamOptions, devices,
};

/// After this long unused the model is let go (it can take half a gigabyte
/// or more); the next dictation, or the next recording's warm-up, loads it
/// again.
const UNLOAD_AFTER: Duration = Duration::from_secs(10 * 60);
/// The most a stream's final text may take once the last audio is in.
const FINISH_TIMEOUT: Duration = Duration::from_secs(10);
/// Silence put before and after a clip for the streaming models (Nemotron).
/// They need a moment to settle: from a clip trimmed close to the speech
/// they drop the first word, or, in some languages, everything.
const LEAD_IN: Duration = Duration::from_millis(1000);
const LEAD_OUT: Duration = Duration::from_millis(500);
/// How long a transcription waits for a stream on the same model to end
/// (a long hands-free dictation), checking this often.
const BUSY_WAIT: Duration = Duration::from_secs(15 * 60);
const BUSY_POLL: Duration = Duration::from_millis(50);

pub struct LocalSpeech {
    model: Arc<Loaded>,
    model_name: String,
    english_only: bool,
    streams: bool,
}

/// What's known about a model before it's loaded (from the catalog).
#[derive(Debug, Clone, Copy, Default)]
pub struct Profile {
    /// Keep it on the CPU (streaming models).
    pub cpu: bool,
    pub english_only: bool,
    /// It can transcribe while the user speaks.
    pub streams: bool,
}

/// The model file and, while it's in use, the loaded model.
struct Loaded {
    path: PathBuf,
    /// Keep it on the CPU (streaming models).
    cpu: bool,
    loaded: Mutex<Option<Model>>,
    last_used: Mutex<Instant>,
}

/// Speech models being loaded right now, for Settings.
static LOADING: AtomicUsize = AtomicUsize::new(0);

/// Whether a speech model is loading right now.
pub fn loading() -> bool {
    LOADING.load(Ordering::Relaxed) > 0
}

impl Loaded {
    fn model(&self) -> ProviderResult<Model> {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(model) = &*loaded {
            return Ok(model.clone());
        }
        LOADING.fetch_add(1, Ordering::Relaxed);
        let model = open(&self.path, self.cpu);
        LOADING.fetch_sub(1, Ordering::Relaxed);
        let model = model?;
        *loaded = Some(model.clone());
        Ok(model)
    }

    fn is_loaded(&self) -> bool {
        match self.loaded.try_lock() {
            Ok(loaded) => loaded.is_some(),
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner().is_some(),
            Err(std::sync::TryLockError::WouldBlock) => false,
        }
    }

    fn unload_if_idle(&self) {
        let idle = self
            .last_used
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed();
        if idle >= UNLOAD_AFTER {
            let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
            if loaded.take().is_some() {
                tracing::debug!(
                    "unloaded {} after {} min unused",
                    self.path.display(),
                    idle.as_secs() / 60
                );
            }
        }
    }
}

fn open(path: &Path, cpu: bool) -> ProviderResult<Model> {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Its own and ggml's logging would go to stderr.
        transcribe_cpp::disable_logging();
        if let Err(e) = transcribe_cpp::init_backends_default() {
            tracing::warn!("speech backends: {e}");
        }
    });
    let device = cpu
        .then(|| devices().into_iter().find(|d| d.kind == "cpu"))
        .flatten();
    Model::load_with(
        path,
        &ModelOptions {
            device,
            ..Default::default()
        },
    )
    .map_err(|e| ProviderError::Unavailable(format!("couldn't load speech model: {e}")))
}

/// Lets the model go once it's been idle a while; stops when the provider
/// is dropped.
fn watch_idle(model: Weak<Loaded>) {
    let _ = std::thread::Builder::new()
        .name("speech-idle".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(60));
                match model.upgrade() {
                    Some(model) => model.unload_if_idle(),
                    None => return,
                }
            }
        });
}

impl LocalSpeech {
    /// A GGUF model file, loaded in the background from now on (and again
    /// after it's been let go): the model the user chose.
    pub fn load(path: &Path, profile: Profile) -> ProviderResult<Self> {
        let speech = Self::on_demand(path, profile)?;
        preload(speech.model.clone());
        Ok(speech)
    }

    /// A GGUF model file, loaded only when it's first used: a stand-in for
    /// a cloud provider that failed.
    pub fn on_demand(path: &Path, profile: Profile) -> ProviderResult<Self> {
        if !path.is_file() {
            return Err(ProviderError::Unavailable(format!(
                "model not found at {} — download it first",
                path.display()
            )));
        }
        let model_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("local")
            .to_string();
        Ok(Self {
            english_only: profile.english_only,
            streams: profile.streams,
            model_name,
            model: shared(path, profile.cpu),
        })
    }
}

/// The model for `path`, shared by every provider that uses it. Settings
/// rebuild the providers on every change; a new copy each time would load
/// the model again (seconds, and another copy in memory meanwhile).
fn shared(path: &Path, cpu: bool) -> Arc<Loaded> {
    type Models = Mutex<Vec<(PathBuf, bool, Weak<Loaded>)>>;
    static MODELS: OnceLock<Models> = OnceLock::new();
    let mut models = MODELS
        .get_or_init(Models::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    models.retain(|(_, _, m)| m.strong_count() > 0);
    if let Some(loaded) = models
        .iter()
        .find(|(p, c, _)| p == path && *c == cpu)
        .and_then(|(_, _, m)| m.upgrade())
    {
        return loaded;
    }
    let loaded = Arc::new(Loaded {
        path: path.to_path_buf(),
        cpu,
        loaded: Mutex::new(None),
        last_used: Mutex::new(Instant::now()),
    });
    watch_idle(Arc::downgrade(&loaded));
    models.push((path.to_path_buf(), cpu, Arc::downgrade(&loaded)));
    loaded
}

/// Loads the model on another thread; whoever needs it first waits there.
fn preload(loaded: Arc<Loaded>) {
    let _ = std::thread::Builder::new()
        .name("speech-load".into())
        .spawn(move || {
            let started = Instant::now();
            if loaded.is_loaded() {
                return;
            }
            match loaded.model() {
                Ok(_) => tracing::info!(
                    "loaded {} in {} ms",
                    loaded.path.display(),
                    started.elapsed().as_millis()
                ),
                Err(e) => tracing::warn!("{e}"),
            }
        });
}

/// The model's own name for `code` (ISO 639-1): itself, or the first of
/// its locales for that language (Nemotron 3.5 lists `de-DE`, not `de`).
/// `None` when it doesn't list the language: it would fail every run, so
/// it detects the language instead.
fn model_language(languages: &[String], code: &str) -> Option<String> {
    if languages.is_empty() {
        return Some(code.to_string());
    }
    let prefix = format!("{}-", code.to_ascii_lowercase());
    languages
        .iter()
        .find(|l| l.eq_ignore_ascii_case(code))
        .or_else(|| {
            languages
                .iter()
                .find(|l| l.to_ascii_lowercase().starts_with(&prefix))
        })
        .cloned()
}

fn run_options(model: &Model, english_only: bool, hints: &SttHints) -> RunOptions {
    let languages = model
        .capabilities()
        .map(|c| c.languages)
        .unwrap_or_default();
    RunOptions {
        language: hints
            .language
            .as_deref()
            .filter(|_| !english_only)
            .and_then(|code| model_language(&languages, code)),
        vocabulary: if model.supports(Feature::Vocabulary) {
            hints.vocabulary.clone()
        } else {
            Vec::new()
        },
        prompt: hints
            .prompt()
            .filter(|_| model.supports(Feature::ContextPrompt)),
        ..Default::default()
    }
}

#[async_trait]
impl SpeechToText for LocalSpeech {
    fn name(&self) -> String {
        format!("local/{}", self.model_name)
    }

    fn ready(&self) -> bool {
        // Without waiting: the lock is held for the whole load, and waiting
        // here would freeze the start of a dictation.
        self.model.is_loaded()
    }

    fn warm_up(&self) {
        let model = self.model.clone();
        std::thread::spawn(move || {
            if let Err(e) = model.model() {
                tracing::warn!("{e}");
            }
        });
    }

    async fn transcribe(&self, clip: &AudioClip, hints: &SttHints) -> ProviderResult<Transcript> {
        let mut samples = to_16k(clip);
        if self.streams {
            let silence = |d: Duration| std::iter::repeat_n(0.0, d.as_millis() as usize * 16);
            samples.splice(0..0, silence(LEAD_IN));
            samples.extend(silence(LEAD_OUT));
        }
        let loaded = self.model.clone();
        let (english_only, hints) = (self.english_only, hints.clone());
        let (text, prompt) = tokio::task::spawn_blocking(move || {
            // Waits here if it's still loading.
            let model = loaded.model()?;
            let options = run_options(&model, english_only, &hints);
            let mut session = model
                .session()
                .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
            let waiting = Instant::now();
            let result = loop {
                match session.run(&samples, &options) {
                    // A stream has the model: wait for it to end.
                    Err(transcribe_cpp::Error::Busy(_)) if waiting.elapsed() < BUSY_WAIT => {
                        std::thread::sleep(BUSY_POLL);
                    }
                    result => {
                        break result.map_err(|e| {
                            ProviderError::InvalidResponse(format!("transcription failed: {e}"))
                        })?;
                    }
                }
            };
            Ok::<_, ProviderError>((result.text, options.prompt))
        })
        .await
        .map_err(|e| ProviderError::Unavailable(format!("transcription task failed: {e}")))??;

        let mut text = clean_whisper_text(&text);
        if prompt
            .as_deref()
            .is_some_and(|hints| echoes_hints(&text, hints))
        {
            tracing::info!("the model only repeated its hints; treating it as silence");
            text.clear();
        }
        Ok(Transcript {
            text,
            usage: Usage::FREE,
        })
    }

    fn stream(&self, hints: &SttHints) -> Option<Box<dyn SpeechStream>> {
        self.streams.then(|| {
            Box::new(LocalStream::start(
                self.model.clone(),
                self.english_only,
                hints.clone(),
            )) as Box<dyn SpeechStream>
        })
    }
}

fn to_16k(clip: &AudioClip) -> Vec<f32> {
    if clip.sample_rate == splashdown_audio::TARGET_SAMPLE_RATE {
        clip.samples.clone()
    } else {
        splashdown_audio::resample::resample(
            &clip.samples,
            clip.sample_rate,
            splashdown_audio::TARGET_SAMPLE_RATE,
        )
    }
}

// ---------------------------------------------------------------- streaming

enum Feed {
    Audio(Vec<f32>),
    Finish(Sender<ProviderResult<String>>),
}

/// A stream on its own thread (the library's stream borrows its session,
/// so both live there): `feed` only queues audio, and `finish` waits for
/// the last of it.
struct LocalStream {
    tx: Sender<Feed>,
    partial: Arc<Mutex<Option<String>>>,
    /// Set when nobody wants the result any more (dropped, or `finish`
    /// gave up), so the thread stops and frees the model instead of
    /// working through audio still queued.
    abandoned: Arc<AtomicBool>,
}

impl LocalStream {
    /// Starts at once: audio fed before the model is loaded waits in the
    /// channel.
    fn start(loaded: Arc<Loaded>, english_only: bool, hints: SttHints) -> Self {
        let (tx, rx) = mpsc::channel();
        let partial = Arc::new(Mutex::new(None));
        let abandoned = Arc::new(AtomicBool::new(false));
        let (shared, stop) = (partial.clone(), abandoned.clone());
        let _ = std::thread::Builder::new()
            .name("speech-stream".into())
            .spawn(move || match loaded.model() {
                Ok(model) => {
                    let options = run_options(&model, english_only, &hints);
                    run_stream(&model, &options, &rx, &shared, &stop);
                }
                Err(e) => fail(&rx, e),
            });
        Self {
            tx,
            partial,
            abandoned,
        }
    }
}

fn run_stream(
    model: &Model,
    options: &RunOptions,
    rx: &Receiver<Feed>,
    partial: &Mutex<Option<String>>,
    abandoned: &AtomicBool,
) {
    let error =
        |e: transcribe_cpp::Error| ProviderError::InvalidResponse(format!("stream failed: {e}"));
    let mut session = match model.session() {
        Ok(s) => s,
        Err(e) => return fail(rx, error(e)),
    };
    let stream_options = StreamOptions {
        commit_policy: CommitPolicy::Auto,
        ..Default::default()
    };
    let mut stream = match session.stream(options, &stream_options) {
        Ok(s) => s,
        Err(e) => return fail(rx, error(e)),
    };
    let mut failed = None;
    while let Ok(msg) = rx.recv() {
        if abandoned.load(Ordering::Relaxed) {
            // Dropping the stream frees the model for others.
            return;
        }
        match msg {
            Feed::Audio(samples) if failed.is_none() => match stream.feed(&samples) {
                Ok(update) if update.result_changed => {
                    *partial.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(stream.text().full.trim().to_string());
                }
                Ok(_) => {}
                Err(e) => failed = Some(error(e)),
            },
            Feed::Audio(_) => {}
            Feed::Finish(reply) => {
                let result = match failed.take() {
                    Some(e) => Err(e),
                    None => stream
                        .finalize()
                        .map(|_| clean_whisper_text(&stream.text().full))
                        .map_err(error),
                };
                let _ = reply.send(result);
                return;
            }
        }
    }
}

/// The stream never started: every `finish` gets the reason.
fn fail(rx: &Receiver<Feed>, e: ProviderError) {
    tracing::warn!("{e}");
    while let Ok(msg) = rx.recv() {
        if let Feed::Finish(reply) = msg {
            let _ = reply.send(Err(e));
            return;
        }
    }
}

impl Drop for LocalStream {
    fn drop(&mut self) {
        self.abandoned.store(true, Ordering::Relaxed);
    }
}

impl SpeechStream for LocalStream {
    fn feed(&mut self, samples: &[f32]) {
        let _ = self.tx.send(Feed::Audio(samples.to_vec()));
    }

    fn partial(&mut self) -> Option<String> {
        self.partial
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    fn finish(self: Box<Self>) -> ProviderResult<String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Feed::Finish(reply))
            .map_err(|_| ProviderError::Unavailable("the stream stopped".into()))?;
        match rx.recv_timeout(FINISH_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(ProviderError::Unavailable(
                "the stream didn't finish in time".into(),
            )),
            Err(RecvTimeoutError::Disconnected) => {
                Err(ProviderError::Unavailable("the stream stopped".into()))
            }
        }
    }
}

/// On near-silent audio Whisper sometimes "hears" its own prompt
/// ("Vocabulary: Tauri, OpenRouter."). Output of three or more words that's
/// only a stretch of the hints is that, not speech; a word or two from the
/// vocabulary could really have been said.
fn echoes_hints(text: &str, hints: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let said = words(text);
    let hints = words(hints);
    said.len() >= 3 && hints.windows(said.len()).any(|w| w == said.as_slice())
}

/// Whisper marks silence and noise with bracketed tags like `[BLANK_AUDIO]`
/// or `(music)`; those aren't speech.
fn clean_whisper_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transcription while a stream has the same model waits its turn,
    /// and the two models share one load. Needs the Nemotron (English)
    /// model downloaded and the eval audio:
    /// `cargo test -p splashdown-providers --release -- --ignored stream_and_run`.
    /// Streaming vs one run on the multilingual set, for a model given by
    /// `MODEL` (an id): `MODEL=nemotron cargo test -p splashdown-providers
    /// --release -- --ignored --nocapture stream_vs_run`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a downloaded model and eval/audio-languages"]
    async fn stream_vs_run() {
        let id = std::env::var("MODEL").unwrap_or_else(|_| "nemotron".into());
        let model = crate::SpeechModel::from_id(&id).unwrap();
        let dir = splashdown_config::Paths::resolve().unwrap().models_dir;
        let speech = LocalSpeech::on_demand(&dir.join(model.file_name()), model.profile()).unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/audio-languages");
        for case in ["fr-1", "fr-2", "it-1", "it-2", "de-1", "ja-1"] {
            let clip = splashdown_audio::wav::read(&root.join(format!("{case}.wav"))).unwrap();
            let mut clip = splashdown_audio::prepare(&clip).unwrap();
            if let Ok(pad) = std::env::var("LEAD_MS") {
                let n = pad.parse::<usize>().unwrap() * 16;
                clip.samples.splice(0..0, std::iter::repeat_n(0.0, n));
                clip.samples.extend(std::iter::repeat_n(0.0, n));
            }
            let hints = SttHints {
                language: Some(case[..2].to_string()),
                ..SttHints::default()
            };
            let run = speech.transcribe(&clip, &hints).await.unwrap().text;
            let mut stream = speech.stream(&hints).unwrap();
            for piece in clip.samples.chunks(1600) {
                stream.feed(piece);
            }
            let streamed = tokio::task::spawn_blocking(move || stream.finish())
                .await
                .unwrap()
                .unwrap();
            println!("{case}  run: {run}\n      stream: {streamed}");
        }
    }

    /// What each downloaded model says about itself, against the catalog.
    /// `cargo test -p splashdown-providers --release -- --ignored --nocapture catalog`.
    #[test]
    #[ignore = "needs downloaded models"]
    fn catalog_matches_the_models() {
        let dir = splashdown_config::Paths::resolve().unwrap().models_dir;
        let mut wrong = Vec::new();
        for model in crate::SpeechModel::ALL {
            let path = dir.join(model.file_name());
            if !path.is_file() {
                println!("{:<16} not downloaded", model.id());
                continue;
            }
            let loaded = open(&path, true).unwrap();
            let caps = loaded.capabilities().unwrap();
            let english_only = caps.languages.len() == 1 && caps.languages[0] == "en";
            println!(
                "{:<16} languages {:>3} {:<40} streaming {:<5} max audio {:>6} ms  vocabulary {:<5} prompt {:<5} detect {}",
                model.id(),
                caps.languages.len(),
                if std::env::var_os("ALL_LANGUAGES").is_some() {
                    caps.languages.join(",")
                } else {
                    caps.languages
                        .iter()
                        .take(8)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",")
                },
                caps.supports_streaming,
                caps.max_audio_ms,
                loaded.supports(Feature::Vocabulary),
                loaded.supports(Feature::ContextPrompt),
                caps.supports_language_detect,
            );
            if model.streams() != caps.supports_streaming {
                wrong.push(format!("{}: streams", model.id()));
            }
            if model.english_only() != english_only && !caps.languages.is_empty() {
                wrong.push(format!("{}: english_only", model.id()));
            }
        }
        assert!(
            wrong.is_empty(),
            "catalog differs from the models: {wrong:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a downloaded model and eval audio"]
    async fn stream_and_run_share_one_model() {
        let model = crate::SpeechModel::NemotronEn;
        let dir = splashdown_config::Paths::resolve().unwrap().models_dir;
        let path = dir.join(model.file_name());
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/audio");
        let clip = splashdown_audio::wav::read(&root.join("technical.wav")).unwrap();
        let samples = to_16k(&clip);

        let a = LocalSpeech::load(&path, model.profile()).unwrap();
        // A second provider for the same file (Settings rebuilt the engine)
        // gets the same model, not a second load.
        let b = LocalSpeech::on_demand(&path, model.profile()).unwrap();
        assert!(Arc::ptr_eq(&a.model, &b.model));

        let mut stream = a.stream(&SttHints::default()).unwrap();
        stream.feed(&samples);
        let busy = tokio::spawn(async move {
            let clip = AudioClip::new(samples, splashdown_audio::TARGET_SAMPLE_RATE);
            b.transcribe(&clip, &SttHints::default()).await
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let streamed = tokio::task::spawn_blocking(move || stream.finish())
            .await
            .unwrap()
            .unwrap();
        let run = busy.await.unwrap().unwrap();
        assert!(streamed.to_lowercase().contains("kubernetes"), "{streamed}");
        assert!(
            run.text.to_lowercase().contains("kubernetes"),
            "{}",
            run.text
        );
    }

    #[test]
    fn languages_are_named_as_the_model_names_them() {
        let locales: Vec<String> = ["en-US", "en-GB", "de-DE", "pt-BR", "pt-PT"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(model_language(&locales, "de").as_deref(), Some("de-DE"));
        assert_eq!(model_language(&locales, "pt").as_deref(), Some("pt-BR"));
        assert_eq!(model_language(&locales, "ja"), None);
        let codes: Vec<String> = ["en", "de"].into_iter().map(String::from).collect();
        assert_eq!(model_language(&codes, "de").as_deref(), Some("de"));
        assert_eq!(model_language(&[], "ja").as_deref(), Some("ja"));
    }

    #[test]
    fn repeated_hints_are_not_speech() {
        let hints = "Vocabulary: Tauri, OpenRouter, Kubernetes.";
        assert!(echoes_hints("Vocabulary: Tauri, OpenRouter.", hints));
        assert!(echoes_hints("tauri openrouter kubernetes", hints));
        // A word or two could really have been said.
        assert!(!echoes_hints("Kubernetes.", hints));
        assert!(!echoes_hints("Tauri, OpenRouter", hints));
        assert!(!echoes_hints("Deploy Tauri to Kubernetes.", hints));
    }

    #[test]
    fn strips_non_speech_tags() {
        assert_eq!(clean_whisper_text(" [BLANK_AUDIO]"), "");
        assert_eq!(
            clean_whisper_text(" Hello there. (music) How are you?"),
            "Hello there. How are you?"
        );
    }

    #[test]
    fn missing_model_is_unavailable() {
        let err = LocalSpeech::load(Path::new("/nonexistent/model.gguf"), Profile::default())
            .err()
            .unwrap();
        assert!(matches!(err, ProviderError::Unavailable(_)));
    }
}
