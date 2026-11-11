//! Builds providers from resolved config choices.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use splashdown_config::{AiChoice, AiProvider, Secrets, Situation, SpeechChoice, SpeechProvider};
use splashdown_core::{ProviderError, ProviderResult, SpeechToText, TextModel};

use crate::{ChatAudioTranscriber, ChatModel, Endpoint, SpeechModel, TranscriptionApi};

/// Which on-device models "Automatic" may pick: Apple's (for `language`)
/// and the built-in local text model, if it's downloaded and its helper is
/// installed.
pub fn on_device(language: Option<&str>, models_dir: &Path) -> Situation {
    Situation {
        local_model: local_model_available(models_dir),
        // Only where the processor can run it: "automatic" then goes to
        // whatever else is set up instead of to a model that can't load.
        local_speech: crate::speech_library_runs_here()
            && SpeechModel::from_id(splashdown_config::defaults::LOCAL_SPEECH)
                .is_some_and(|m| m.is_downloaded(models_dir)),
        ..crate::apple::on_device(language)
    }
}

/// The default built-in model is downloaded and its helper is installed.
pub fn local_model_available(models_dir: &Path) -> bool {
    crate::LocalModel::from_id(splashdown_config::defaults::LOCAL_AI)
        .is_some_and(|m| m.is_downloaded(models_dir))
        && crate::local_llm::helper_installed()
}

pub fn speech(
    choice: &SpeechChoice,
    secrets: &Secrets,
    models_dir: &Path,
) -> ProviderResult<Arc<dyn SpeechToText>> {
    let key = |k: &Option<String>, name: &str| {
        k.clone()
            .ok_or_else(|| ProviderError::Unavailable(format!("{name} API key not set")))
    };
    Ok(match choice.provider {
        SpeechProvider::Openai => Arc::new(TranscriptionApi::new(
            Endpoint::openai(key(&secrets.openai, "OpenAI")?),
            &choice.model,
        )),
        SpeechProvider::Openrouter => Arc::new(ChatAudioTranscriber::new(
            Endpoint::openrouter(key(&secrets.openrouter, "OpenRouter")?),
            &choice.model,
        )),
        SpeechProvider::WhisperLocal => local_speech(&choice.model, models_dir)?,
        SpeechProvider::Apple => crate::apple::speech(None)?,
        SpeechProvider::Auto => {
            return Err(ProviderError::Unavailable(
                "no speech provider chosen".into(),
            ));
        }
    })
}

pub fn text_model(choice: &AiChoice, secrets: &Secrets) -> ProviderResult<Arc<dyn TextModel>> {
    let endpoint = match choice.provider {
        AiProvider::Openai => Endpoint::openai(secrets.openai.clone().unwrap_or_default()),
        AiProvider::Openrouter => {
            Endpoint::openrouter(secrets.openrouter.clone().unwrap_or_default())
        }
        AiProvider::Custom => Endpoint::custom(
            choice.base_url.clone().unwrap_or_default(),
            secrets.custom.clone(),
        ),
        AiProvider::Anthropic => {
            return Ok(Arc::new(crate::AnthropicModel::new(
                Endpoint::anthropic(secrets.anthropic.clone().unwrap_or_default()),
                &choice.model,
            )));
        }
        AiProvider::Apple => return crate::apple::language_model(),
        AiProvider::Local => return local_model(&choice.model),
        AiProvider::Auto | AiProvider::Off => {
            return Err(ProviderError::Unavailable("no AI provider chosen".into()));
        }
    };
    Ok(Arc::new(
        ChatModel::new(endpoint, &choice.model).with_fallbacks(choice.fallback_models.clone()),
    ))
}

/// A catalog id (`nemotron-en`) resolves inside `models_dir`; anything
/// else is a path to a GGUF file. Streaming models stay on the CPU.
fn local_speech(model: &str, models_dir: &Path) -> ProviderResult<Arc<dyn SpeechToText>> {
    let path = match SpeechModel::from_id(model) {
        Some(m) if !m.is_downloaded(models_dir) => {
            return Err(ProviderError::Unavailable(format!(
                "speech model {} isn't downloaded (~{} MB) — run `splashdown-cli models download {}`",
                m.id(),
                m.approx_mb(),
                m.id()
            )));
        }
        Some(m) => return load_local_speech(m, models_dir),
        None => PathBuf::from(model),
    };
    load_local_speech_file(&path)
}

/// The built-in model: a catalog id (`qwen3-4b`) in the models folder, or a
/// path to any GGUF file. `SPLASHDOWN_LOCAL_CPU=1` keeps it off the GPU, to
/// measure machines without one.
fn local_model(model: &str) -> ProviderResult<Arc<dyn TextModel>> {
    let path = match crate::LocalModel::from_id(model) {
        Some(m) => {
            let dir = splashdown_config::Paths::resolve()
                .ok_or_else(|| ProviderError::Unavailable("no home folder".into()))?
                .models_dir;
            m.path_in(&dir)
        }
        None => PathBuf::from(model),
    };
    let cpu_only = std::env::var_os("SPLASHDOWN_LOCAL_CPU").is_some_and(|v| v == "1");
    Ok(Arc::new(crate::LocalLlm::new(&path, cpu_only)?))
}

/// Finds speech with Silero from now on (trimming silence, finding pauses,
/// telling a voice from noise) instead of by loudness. Takes a few
/// milliseconds. `SPLASHDOWN_VAD=loudness` keeps the old way, to compare.
pub fn use_speech_detector(models_dir: &Path) {
    if std::env::var_os("SPLASHDOWN_VAD").is_some_and(|v| v == "loudness") {
        return;
    }
    #[cfg(feature = "local-speech")]
    match crate::Silero::load(models_dir) {
        Ok(silero) => splashdown_audio::vad::set_detector(Some(Arc::new(silero))),
        Err(e) => tracing::warn!("speech detection by loudness: Silero didn't load: {e}"),
    }
    #[cfg(not(feature = "local-speech"))]
    let _ = models_dir;
}

/// A downloaded catalog model. It loads in the background.
#[cfg(feature = "local-speech")]
pub fn load_local_speech(
    model: SpeechModel,
    models_dir: &Path,
) -> ProviderResult<Arc<dyn SpeechToText>> {
    Ok(Arc::new(crate::LocalSpeech::load(
        &model.path_in(models_dir),
        model.profile(),
    )?))
}

/// A downloaded catalog model that loads only when it's first used: the
/// stand-in for a cloud provider that failed.
#[cfg(feature = "local-speech")]
pub fn local_speech_on_demand(
    model: SpeechModel,
    models_dir: &Path,
) -> ProviderResult<Arc<dyn SpeechToText>> {
    Ok(Arc::new(crate::LocalSpeech::on_demand(
        &model.path_in(models_dir),
        model.profile(),
    )?))
}

/// Any GGUF file (`--stt-model <path>`): treated as a multilingual model
/// that transcribes when the recording ends.
#[cfg(feature = "local-speech")]
fn load_local_speech_file(path: &Path) -> ProviderResult<Arc<dyn SpeechToText>> {
    Ok(Arc::new(crate::LocalSpeech::load(
        path,
        crate::SpeechProfile::default(),
    )?))
}

#[cfg(not(feature = "local-speech"))]
pub fn load_local_speech(_: SpeechModel, _: &Path) -> ProviderResult<Arc<dyn SpeechToText>> {
    Err(unavailable())
}

#[cfg(not(feature = "local-speech"))]
pub fn local_speech_on_demand(_: SpeechModel, _: &Path) -> ProviderResult<Arc<dyn SpeechToText>> {
    Err(unavailable())
}

#[cfg(not(feature = "local-speech"))]
fn load_local_speech_file(_: &Path) -> ProviderResult<Arc<dyn SpeechToText>> {
    Err(unavailable())
}

#[cfg(not(feature = "local-speech"))]
fn unavailable() -> ProviderError {
    ProviderError::Unavailable("this build doesn't include local speech-to-text".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(openai: bool, openrouter: bool) -> Secrets {
        Secrets {
            openai: openai.then(|| "sk-test".into()),
            openrouter: openrouter.then(|| "sk-or-test".into()),
            anthropic: None,
            custom: None,
        }
    }

    fn speech_choice(provider: SpeechProvider, model: &str) -> SpeechChoice {
        SpeechChoice {
            provider,
            model: model.into(),
        }
    }

    fn ai_choice(provider: AiProvider, model: &str) -> AiChoice {
        AiChoice {
            provider,
            model: model.into(),
            base_url: Some("http://localhost:11434/v1".into()),
            fallback_models: Vec::new(),
        }
    }

    #[test]
    fn speech_providers_need_their_key() {
        let dir = std::env::temp_dir();
        let choice = speech_choice(SpeechProvider::Openai, "gpt-4o-mini-transcribe");
        let err = speech(&choice, &keys(false, true), &dir).err().unwrap();
        assert!(err.to_string().contains("OpenAI API key not set"), "{err}");

        let stt = speech(&choice, &keys(true, false), &dir).ok().unwrap();
        assert_eq!(stt.name(), "openai/gpt-4o-mini-transcribe");

        let choice = speech_choice(SpeechProvider::Openrouter, "google/gemini-3.1-flash-lite");
        let stt = speech(&choice, &keys(false, true), &dir).ok().unwrap();
        assert_eq!(stt.name(), "openrouter/google/gemini-3.1-flash-lite");
    }

    #[test]
    fn missing_speech_model_says_how_to_get_it() {
        let empty = std::env::temp_dir().join("splashdown-factory-test-no-models");
        let choice = speech_choice(SpeechProvider::WhisperLocal, "base.en");
        let err = speech(&choice, &Secrets::default(), &empty).err().unwrap();
        assert!(
            err.to_string()
                .contains("splashdown-cli models download base.en"),
            "{err}"
        );
    }

    #[test]
    fn unresolved_choices_are_rejected() {
        let dir = std::env::temp_dir();
        assert!(
            speech(
                &speech_choice(SpeechProvider::Auto, ""),
                &keys(true, true),
                &dir
            )
            .is_err()
        );
        assert!(text_model(&ai_choice(AiProvider::Auto, "x"), &keys(true, true)).is_err());
        assert!(text_model(&ai_choice(AiProvider::Off, "x"), &keys(true, true)).is_err());
    }

    #[test]
    fn text_models_are_named_by_provider_and_model() {
        let m = text_model(
            &ai_choice(AiProvider::Custom, "qwen2.5:3b"),
            &Secrets::default(),
        )
        .ok()
        .unwrap();
        assert!(m.name().ends_with("/qwen2.5:3b"), "{}", m.name());
        let m = text_model(
            &ai_choice(AiProvider::Openai, "gpt-4.1-mini"),
            &keys(true, false),
        )
        .ok()
        .unwrap();
        assert_eq!(m.name(), "openai/gpt-4.1-mini");
    }
}
