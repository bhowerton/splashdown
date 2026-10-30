//! Apple's on-device models, with the same API on every OS (always
//! unavailable off macOS).

use std::sync::Arc;

use serde::Serialize;
use splashdown_config::Situation;
use splashdown_core::{ProviderError, ProviderResult, SpeechToText, TextModel};

/// Where Apple's speech model stands on this Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechState {
    Ready,
    /// Supported, but the model for the language isn't installed yet.
    NeedsDownload,
    Downloading,
    Unavailable,
}

/// Availability of both Apple models, for Settings and onboarding.
#[derive(Debug, Clone, Serialize)]
pub struct AppleStatus {
    pub speech: SpeechState,
    /// A sentence explaining `speech` when it isn't ready.
    pub speech_detail: String,
    pub language_model: bool,
    pub language_model_detail: String,
}

impl AppleStatus {
    pub fn on_device(&self) -> Situation {
        Situation {
            apple_speech: self.speech == SpeechState::Ready,
            apple_language_model: self.language_model,
            ..Situation::default()
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use futures_executor::block_on;
    use splashdown_apple_shim::Status;

    pub fn status(language: Option<&str>) -> Option<AppleStatus> {
        let (speech, _) = block_on(splashdown_apple_shim::speech_status(language));
        let llm = splashdown_apple_shim::language_model_status();
        Some(AppleStatus {
            speech: match speech {
                Status::Available => SpeechState::Ready,
                Status::NeedsDownload => SpeechState::NeedsDownload,
                Status::NotReady => SpeechState::Downloading,
                _ => SpeechState::Unavailable,
            },
            speech_detail: speech.describe(),
            language_model: llm == Status::Available,
            language_model_detail: llm.describe(),
        })
    }

    pub async fn prepare_speech(language: Option<&str>) -> Result<(), String> {
        splashdown_apple_shim::prepare_speech(language)
            .await
            .map_err(|s| s.describe())
    }

    pub fn speech(language: Option<&str>) -> ProviderResult<Arc<dyn SpeechToText>> {
        match block_on(splashdown_apple_shim::speech_status(language)).0 {
            Status::Available => Ok(Arc::new(splashdown_apple_shim::AppleSpeech)),
            other => Err(ProviderError::Unavailable(format!(
                "Apple's speech model: {}",
                other.describe()
            ))),
        }
    }

    pub fn language_model() -> ProviderResult<Arc<dyn TextModel>> {
        Ok(Arc::new(splashdown_apple_shim::AppleLanguageModel::new()?))
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    const ONLY_MAC: &str = "Apple's on-device models are only on macOS";

    pub fn status(_language: Option<&str>) -> Option<AppleStatus> {
        None
    }

    pub async fn prepare_speech(_language: Option<&str>) -> Result<(), String> {
        Err(ONLY_MAC.into())
    }

    pub fn speech(_language: Option<&str>) -> ProviderResult<Arc<dyn SpeechToText>> {
        Err(ProviderError::Unavailable(ONLY_MAC.into()))
    }

    pub fn language_model() -> ProviderResult<Arc<dyn TextModel>> {
        Err(ProviderError::Unavailable(ONLY_MAC.into()))
    }
}

/// Availability of Apple's models for `language` (ISO code, or the system
/// language); `None` off macOS. Takes a few milliseconds.
pub fn status(language: Option<&str>) -> Option<AppleStatus> {
    imp::status(language)
}

/// Which Apple models `Settings::resolve_*_with` may pick.
pub fn on_device(language: Option<&str>) -> Situation {
    status(language).map_or_else(Situation::default, |s| s.on_device())
}

/// Downloads Apple's speech model for `language` (from Apple; can take a
/// minute).
pub async fn prepare_speech(language: Option<&str>) -> Result<(), String> {
    imp::prepare_speech(language).await
}

pub(crate) fn speech(language: Option<&str>) -> ProviderResult<Arc<dyn SpeechToText>> {
    imp::speech(language)
}

pub(crate) fn language_model() -> ProviderResult<Arc<dyn TextModel>> {
    imp::language_model()
}
