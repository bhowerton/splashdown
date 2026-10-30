//! Local speech-to-text with whisper.cpp (Metal on macOS, CPU elsewhere).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use splashdown_core::{
    AudioClip, ProviderError, ProviderResult, SpeechToText, SttHints, Transcript, Usage,
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// After this long unused the model is let go (it can take a gigabyte or
/// more); the next dictation, or the next recording's warm-up, loads it
/// again.
const UNLOAD_AFTER: Duration = Duration::from_secs(10 * 60);

pub struct WhisperLocal {
    model: Arc<Model>,
    model_name: String,
    english_only: bool,
}

/// The model file and, while it's in use, the loaded model.
struct Model {
    path: PathBuf,
    loaded: Mutex<Option<Arc<WhisperContext>>>,
    last_used: Mutex<Instant>,
}

impl Model {
    fn context(&self) -> ProviderResult<Arc<WhisperContext>> {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ctx) = &*loaded {
            return Ok(ctx.clone());
        }
        let ctx = Arc::new(open(&self.path)?);
        *loaded = Some(ctx.clone());
        Ok(ctx)
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

fn open(path: &Path) -> ProviderResult<WhisperContext> {
    WhisperContext::new_with_params(path, WhisperContextParameters::default())
        .map_err(|e| ProviderError::Unavailable(format!("couldn't load whisper model: {e}")))
}

/// Lets the model go once it's been idle a while; stops when the provider
/// is dropped.
fn watch_idle(model: Weak<Model>) {
    let _ = std::thread::Builder::new()
        .name("whisper-idle".into())
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

impl WhisperLocal {
    /// Loads a ggml model file. Takes a second or two; load once and reuse.
    pub fn load(path: &Path) -> ProviderResult<Self> {
        static QUIET: Once = Once::new();
        // Route whisper.cpp's chatty stderr logging into nowhere.
        QUIET.call_once(whisper_rs::install_logging_hooks);

        if !path.is_file() {
            return Err(ProviderError::Unavailable(format!(
                "model not found at {} — download it first",
                path.display()
            )));
        }
        let ctx = open(path)?;

        let model_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("whisper")
            .trim_start_matches("ggml-")
            .to_string();
        let model = Arc::new(Model {
            path: path.to_path_buf(),
            loaded: Mutex::new(Some(Arc::new(ctx))),
            last_used: Mutex::new(Instant::now()),
        });
        watch_idle(Arc::downgrade(&model));
        Ok(Self {
            english_only: model_name.ends_with(".en"),
            model_name,
            model,
        })
    }
}

#[async_trait]
impl SpeechToText for WhisperLocal {
    fn name(&self) -> String {
        format!("whisper-local/{}", self.model_name)
    }

    fn warm_up(&self) {
        let model = self.model.clone();
        std::thread::spawn(move || {
            if let Err(e) = model.context() {
                tracing::warn!("{e}");
            }
        });
    }

    async fn transcribe(&self, clip: &AudioClip, hints: &SttHints) -> ProviderResult<Transcript> {
        let samples = if clip.sample_rate == splashdown_audio::TARGET_SAMPLE_RATE {
            clip.samples.clone()
        } else {
            splashdown_audio::resample::resample(
                &clip.samples,
                clip.sample_rate,
                splashdown_audio::TARGET_SAMPLE_RATE,
            )
        };
        let model = self.model.clone();
        let language = match (&hints.language, self.english_only) {
            (_, true) => Some("en".to_string()),
            (Some(lang), false) => Some(lang.clone()),
            (None, false) => None,
        };
        let prompt = hints.prompt();

        let text = tokio::task::spawn_blocking(move || -> ProviderResult<String> {
            let ctx = model.context()?;
            let mut state = ctx
                .create_state()
                .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(Some(language.as_deref().unwrap_or("auto")));
            if let Some(prompt) = &prompt {
                params.set_initial_prompt(prompt);
            }
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
            params.set_n_threads(threads as i32);
            params.set_no_context(true);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_special(false);
            params.set_print_timestamps(false);

            state
                .full(params, &samples)
                .map_err(|e| ProviderError::InvalidResponse(format!("whisper failed: {e}")))?;

            let mut text = String::new();
            for segment in state.as_iter() {
                let s = segment
                    .to_str_lossy()
                    .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
                text.push_str(&s);
            }
            Ok(text)
        })
        .await
        .map_err(|e| ProviderError::Unavailable(format!("whisper task failed: {e}")))??;

        Ok(Transcript {
            text: clean_whisper_text(&text),
            usage: Usage::FREE,
        })
    }
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
    fn strips_non_speech_tags() {
        assert_eq!(clean_whisper_text(" [BLANK_AUDIO]"), "");
        assert_eq!(
            clean_whisper_text(" Hello there. (music) How are you?"),
            "Hello there. How are you?"
        );
    }

    #[test]
    fn missing_model_is_unavailable() {
        let err = WhisperLocal::load(Path::new("/nonexistent/ggml-base.en.bin"))
            .err()
            .unwrap();
        assert!(matches!(err, ProviderError::Unavailable(_)));
    }
}
