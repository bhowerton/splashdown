//! `SpeechToText` and `TextModel` over Apple's on-device models.

use std::ffi::CString;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use splashdown_core::{
    AudioClip, Completion, ProviderError, ProviderResult, SpeechToText, SttHints, TextModel,
    TextRequest, Transcript, Usage,
};

use crate::{Status, ffi};

/// Numbers temp audio files so concurrent transcriptions never collide.
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// Apple's on-device model has a 4,096-token context.
const CONTEXT_WINDOW: usize = 4096;

pub struct AppleLanguageModel;

impl AppleLanguageModel {
    /// Fails with the reason if the model can't be used right now.
    pub fn new() -> ProviderResult<Self> {
        match crate::language_model_status() {
            Status::Available => Ok(Self),
            other => Err(ProviderError::Unavailable(format!(
                "Apple's on-device model: {}",
                other.describe()
            ))),
        }
    }
}

#[async_trait]
impl TextModel for AppleLanguageModel {
    fn name(&self) -> String {
        "apple/on-device".into()
    }

    async fn complete(&self, req: &TextRequest) -> ProviderResult<Completion> {
        let system = CString::new(req.system.as_str()).map_err(invalid)?;
        let user = CString::new(req.user.as_str()).map_err(invalid)?;
        // SAFETY: `system` and `user` are NUL-terminated strings that outlive
        // the call (the shim copies them before returning); `ctx` and `cb`
        // come straight from `call`, and the shim calls `cb` exactly once.
        let (code, text) = ffi::call(|ctx, cb| unsafe {
            ffi::splashdown_apple_llm_complete(
                system.as_ptr(),
                user.as_ptr(),
                f64::from(req.temperature),
                req.max_tokens.min(i32::MAX as u32) as i32,
                ctx,
                cb,
            )
        })
        .await;
        match code {
            ffi::OK => Ok(Completion {
                text,
                usage: Usage::FREE,
            }),
            ffi::TOO_LONG => Err(ProviderError::TooLong),
            ffi::GUARDRAIL | ffi::REFUSED => Err(ProviderError::Api(format!("refused: {text}"))),
            _ => Err(ProviderError::Unavailable(text)),
        }
    }

    fn supports_json_schema(&self) -> bool {
        false
    }

    fn context_window(&self) -> usize {
        CONTEXT_WINDOW
    }
}

pub struct AppleSpeech;

#[async_trait]
impl SpeechToText for AppleSpeech {
    fn name(&self) -> String {
        "apple/speech".into()
    }

    async fn transcribe(&self, clip: &AudioClip, hints: &SttHints) -> ProviderResult<Transcript> {
        // The shim reads a file; a 16 kHz WAV in the temp folder.
        let path = std::env::temp_dir().join(format!(
            "splashdown-apple-{}-{}.wav",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let clip16 = if clip.sample_rate == splashdown_audio::TARGET_SAMPLE_RATE {
            clip.clone()
        } else {
            AudioClip::new(
                splashdown_audio::resample::resample(
                    &clip.samples,
                    clip.sample_rate,
                    splashdown_audio::TARGET_SAMPLE_RATE,
                ),
                splashdown_audio::TARGET_SAMPLE_RATE,
            )
        };
        std::fs::write(&path, splashdown_audio::wav::encode(&clip16))
            .map_err(|e| ProviderError::Unavailable(format!("couldn't write audio: {e}")))?;

        let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(invalid)?;
        let locale = hints
            .language
            .as_deref()
            .map(CString::new)
            .transpose()
            .map_err(invalid)?;
        // SAFETY: `c_path` is a live C string and `locale` null or one; the
        // shim copies both before returning. `ctx` and `cb` come straight
        // from `call`, and the shim calls `cb` exactly once.
        let (code, text) = ffi::call(|ctx, cb| unsafe {
            ffi::splashdown_apple_transcribe(
                c_path.as_ptr(),
                locale.as_ref().map_or(std::ptr::null(), |l| l.as_ptr()),
                ctx,
                cb,
            )
        })
        .await;
        let _ = std::fs::remove_file(&path);

        match code {
            ffi::OK => Ok(Transcript {
                text,
                usage: Usage::FREE,
            }),
            _ => Err(ProviderError::Unavailable(format!(
                "Apple's speech model: {}",
                Status::from_code(code, &text).describe()
            ))),
        }
    }
}

fn invalid(e: impl std::fmt::Display) -> ProviderError {
    ProviderError::InvalidResponse(e.to_string())
}
