//! Local speech-to-text with transcribe.cpp (spec §5.2): Nemotron, Parakeet
//! and Whisper GGUF models on this computer. Streaming models transcribe
//! while the user speaks and run on the CPU, where each small piece is
//! quickest; the others use Metal on macOS.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Once, Weak};
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

pub struct LocalSpeech {
    model: Arc<Loaded>,
    model_name: String,
    english_only: bool,
    streams: bool,
}

/// The model file and, while it's in use, the loaded model.
struct Loaded {
    path: PathBuf,
    /// Keep it on the CPU (streaming models).
    cpu: bool,
    loaded: Mutex<Option<Model>>,
    last_used: Mutex<Instant>,
}

impl Loaded {
    fn model(&self) -> ProviderResult<Model> {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(model) = &*loaded {
            return Ok(model.clone());
        }
        let model = open(&self.path, self.cpu)?;
        *loaded = Some(model.clone());
        Ok(model)
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
    /// Loads a GGUF model file. `cpu` keeps it off the GPU, which suits
    /// streaming models. Takes a moment; load once and reuse.
    pub fn load(path: &Path, cpu: bool) -> ProviderResult<Self> {
        if !path.is_file() {
            return Err(ProviderError::Unavailable(format!(
                "model not found at {} — download it first",
                path.display()
            )));
        }
        let model = open(path, cpu)?;
        let caps = model
            .capabilities()
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        let model_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("local")
            .to_string();
        let loaded = Arc::new(Loaded {
            path: path.to_path_buf(),
            cpu,
            loaded: Mutex::new(Some(model)),
            last_used: Mutex::new(Instant::now()),
        });
        watch_idle(Arc::downgrade(&loaded));
        Ok(Self {
            english_only: caps.languages.iter().all(|l| l == "en"),
            streams: caps.supports_streaming,
            model_name,
            model: loaded,
        })
    }

    fn options(&self, model: &Model, hints: &SttHints) -> RunOptions {
        RunOptions {
            language: if self.english_only {
                None
            } else {
                hints.language.clone()
            },
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
}

#[async_trait]
impl SpeechToText for LocalSpeech {
    fn name(&self) -> String {
        format!("local/{}", self.model_name)
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
        let samples = to_16k(clip);
        let loaded = self.model.clone();
        let model = loaded.model()?;
        let options = self.options(&model, hints);
        let prompt = options.prompt.clone();
        let text = tokio::task::spawn_blocking(move || -> ProviderResult<String> {
            let mut session = model
                .session()
                .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
            let result = session.run(&samples, &options).map_err(|e| {
                ProviderError::InvalidResponse(format!("transcription failed: {e}"))
            })?;
            Ok(result.text)
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
        if !self.streams {
            return None;
        }
        let model = match self.model.model() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("no stream: {e}");
                return None;
            }
        };
        let options = self.options(&model, hints);
        Some(Box::new(LocalStream::start(model, options)))
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
}

impl LocalStream {
    fn start(model: Model, options: RunOptions) -> Self {
        let (tx, rx) = mpsc::channel();
        let partial = Arc::new(Mutex::new(None));
        let shared = partial.clone();
        let _ = std::thread::Builder::new()
            .name("speech-stream".into())
            .spawn(move || run_stream(&model, &options, &rx, &shared));
        Self { tx, partial }
    }
}

fn run_stream(
    model: &Model,
    options: &RunOptions,
    rx: &Receiver<Feed>,
    partial: &Mutex<Option<String>>,
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
        let err = LocalSpeech::load(Path::new("/nonexistent/model.gguf"), false)
            .err()
            .unwrap();
        assert!(matches!(err, ProviderError::Unavailable(_)));
    }
}
