//! What "Automatic" and the saved choices come to: which provider and
//! model actually run, given the keys there are, what's on this computer,
//! "only on this computer" and the spending limit.

use super::*;

impl Settings {
    /// Resolves `auto` providers and default models given which keys exist
    /// (assuming no on-device Apple models).
    pub fn resolve_speech(&self, secrets: &Secrets) -> Result<SpeechChoice, ResolveError> {
        self.resolve_speech_with(secrets, Situation::default())
    }

    /// Like `resolve_speech`, also considering Apple's on-device models.
    /// A saved key wins (the user chose that provider); otherwise the
    /// built-in speech model is preferred over Apple's. Over the spending
    /// limit, cloud speech-to-text gives way to a model on this computer.
    pub fn resolve_speech_with(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Result<SpeechChoice, ResolveError> {
        let choice = self.choose_speech(secrets, on_device)?;
        if !(on_device.over_budget && choice.destination() == Destination::Cloud) {
            return Ok(choice);
        }
        let provider = if on_device.local_speech {
            SpeechProvider::WhisperLocal
        } else if on_device.apple_speech {
            SpeechProvider::Apple
        } else {
            return Err(ResolveError::OverBudget);
        };
        let mut local = self.clone();
        local.speech = SpeechSettings {
            provider,
            model: None,
        };
        local.choose_speech(secrets, on_device)
    }

    pub(super) fn choose_speech(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Result<SpeechChoice, ResolveError> {
        let local_only = self.general.local_only;
        let provider = match self.speech.provider {
            // The built-in speech model first: about twice as accurate as
            // Apple's in the eval.
            SpeechProvider::Auto if local_only && on_device.local_speech => {
                SpeechProvider::WhisperLocal
            }
            SpeechProvider::Auto if local_only && on_device.apple_speech => SpeechProvider::Apple,
            SpeechProvider::Auto if local_only => SpeechProvider::WhisperLocal,
            SpeechProvider::Openai | SpeechProvider::Openrouter if local_only => {
                return Err(ResolveError::LocalOnly("cloud speech-to-text"));
            }
            SpeechProvider::Auto if secrets.openai.is_some() => SpeechProvider::Openai,
            SpeechProvider::Auto if secrets.openrouter.is_some() => SpeechProvider::Openrouter,
            SpeechProvider::Auto if on_device.local_speech => SpeechProvider::WhisperLocal,
            SpeechProvider::Auto if on_device.apple_speech => SpeechProvider::Apple,
            SpeechProvider::Auto => SpeechProvider::WhisperLocal,
            p => p,
        };
        require_key(provider_key_need_speech(provider), secrets)?;
        let model = self.speech.model.clone().unwrap_or_else(|| {
            match provider {
                SpeechProvider::Openai => defaults::OPENAI_STT,
                SpeechProvider::Openrouter => defaults::OPENROUTER_STT,
                SpeechProvider::WhisperLocal => defaults::LOCAL_SPEECH,
                SpeechProvider::Apple | SpeechProvider::Auto => defaults::SYSTEM_MODEL,
            }
            .to_string()
        });
        Ok(SpeechChoice { provider, model })
    }

    pub fn resolve_ai(&self, secrets: &Secrets) -> Result<Option<AiChoice>, ResolveError> {
        self.resolve_ai_with(secrets, Situation::default())
    }

    /// Like `resolve_ai`, also considering on-device models. Over the
    /// spending limit, a cloud provider gives way to Apple's model, the
    /// built-in local model, or no AI pass.
    pub fn resolve_ai_with(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Result<Option<AiChoice>, ResolveError> {
        let choice = self.choose_ai(secrets, on_device)?;
        let paid = choice
            .as_ref()
            .is_some_and(|c| c.destination() == Destination::Cloud);
        if !(on_device.over_budget && paid) {
            return Ok(choice);
        }
        let provider = if on_device.local_model {
            AiProvider::Local
        } else if on_device.apple_language_model {
            AiProvider::Apple
        } else {
            return Ok(None);
        };
        let mut local = self.clone();
        local.ai = AiSettings {
            provider,
            ..AiSettings::default()
        };
        local.choose_ai(secrets, on_device)
    }

    pub(super) fn choose_ai(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Result<Option<AiChoice>, ResolveError> {
        let local_only = self.general.local_only;
        let provider = match self.ai.provider {
            // The built-in model first: it does better than Apple's in the
            // eval (85% to 72%).
            AiProvider::Auto if local_only && on_device.local_model => AiProvider::Local,
            AiProvider::Auto if local_only && on_device.apple_language_model => AiProvider::Apple,
            AiProvider::Auto if local_only => return Ok(None),
            AiProvider::Openai | AiProvider::Openrouter | AiProvider::Anthropic if local_only => {
                return Err(ResolveError::LocalOnly("a cloud AI provider"));
            }
            AiProvider::Custom
                if local_only
                    && !is_local_url(
                        self.ai
                            .base_url
                            .as_deref()
                            .unwrap_or(defaults::CUSTOM_BASE_URL),
                    ) =>
            {
                return Err(ResolveError::LocalOnly("a server on another computer"));
            }
            AiProvider::Auto if secrets.openai.is_some() => AiProvider::Openai,
            AiProvider::Auto if secrets.openrouter.is_some() => AiProvider::Openrouter,
            AiProvider::Auto if secrets.anthropic.is_some() => AiProvider::Anthropic,
            AiProvider::Auto if on_device.local_model => AiProvider::Local,
            AiProvider::Auto if on_device.apple_language_model => AiProvider::Apple,
            AiProvider::Auto | AiProvider::Off => return Ok(None),
            p => p,
        };
        require_key(provider_key_need_ai(provider), secrets)?;
        let model = match (provider, &self.ai.model) {
            (_, Some(m)) => m.clone(),
            (AiProvider::Openai, None) => defaults::OPENAI_AI.into(),
            (AiProvider::Openrouter, None) => defaults::OPENROUTER_AI.into(),
            (AiProvider::Anthropic, None) => defaults::ANTHROPIC_AI.into(),
            (AiProvider::Local, None) => defaults::LOCAL_AI.into(),
            (AiProvider::Custom, None) => return Err(ResolveError::CustomNeedsModel),
            (_, None) => defaults::SYSTEM_MODEL.into(),
        };
        let base_url = (provider == AiProvider::Custom).then(|| {
            self.ai
                .base_url
                .clone()
                .unwrap_or_else(|| defaults::CUSTOM_BASE_URL.into())
        });
        let fallback_models = match (&self.ai.fallback_models, provider) {
            (Some(list), AiProvider::Openrouter) => list.clone(),
            (None, AiProvider::Openrouter) => vec![defaults::OPENROUTER_AI_FALLBACK.into()],
            _ => Vec::new(),
        };
        Ok(Some(AiChoice {
            provider,
            fallback_models: fallback_models
                .into_iter()
                .filter(|m| *m != model)
                .collect(),
            model,
            base_url,
        }))
    }

    /// The cloud model that handles instructions when
    /// `ai.cloud_for_instructions` is on and cleanup itself runs locally.
    /// `None` when it doesn't apply: the setting is off, local-only mode is
    /// on, cleanup already uses the cloud, or there's no key.
    pub fn resolve_instruction_ai_with(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Option<AiChoice> {
        if !self.ai.cloud_for_instructions || self.general.local_only || on_device.over_budget {
            return None;
        }
        let main = self.resolve_ai_with(secrets, on_device).ok()??;
        if main.destination() != Destination::Local {
            return None;
        }
        let provider = if secrets.openai.is_some() {
            AiProvider::Openai
        } else if secrets.openrouter.is_some() {
            AiProvider::Openrouter
        } else if secrets.anthropic.is_some() {
            AiProvider::Anthropic
        } else {
            return None;
        };
        let mut cloud = self.clone();
        cloud.ai = AiSettings {
            provider,
            ..AiSettings::default()
        };
        cloud.resolve_ai_with(secrets, on_device).ok().flatten()
    }
}

/// Where a provider sends the user's words (the Privacy tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Destination {
    /// Stays on this computer.
    Local,
    /// Sent to a cloud provider.
    Cloud,
    /// Nothing is sent: the step is off or can't run.
    Off,
}

/// A concrete speech-to-text choice after resolving `auto` and defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechChoice {
    pub provider: SpeechProvider,
    pub model: String,
}

impl SpeechChoice {
    pub fn destination(&self) -> Destination {
        match self.provider {
            SpeechProvider::WhisperLocal | SpeechProvider::Apple => Destination::Local,
            _ => Destination::Cloud,
        }
    }
}

/// A concrete text-model choice; `None` means the AI pass is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiChoice {
    pub provider: AiProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub fallback_models: Vec<String>,
}

impl AiChoice {
    pub fn destination(&self) -> Destination {
        match self.provider {
            AiProvider::Apple | AiProvider::Local => Destination::Local,
            AiProvider::Custom if self.base_url.as_deref().is_some_and(is_local_url) => {
                Destination::Local
            }
            _ => Destination::Cloud,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("{0} needs an API key (set {1})")]
    MissingKey(&'static str, &'static str),
    #[error("the custom AI provider needs a model name")]
    CustomNeedsModel,
    #[error("local-only mode is on, so {0} can't be used")]
    LocalOnly(&'static str),
    #[error(
        "the spending limit is reached and there's no speech model on this computer — raise the limit in Settings, or download a speech model"
    )]
    OverBudget,
}

/// What's true right now that provider resolution depends on, beyond the
/// settings and keys: which on-device models can run, and whether the
/// spending limit is reached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Situation {
    /// Apple's speech model is installed (Apple Silicon, macOS 26+).
    pub apple_speech: bool,
    /// Apple Intelligence is on.
    pub apple_language_model: bool,
    /// The built-in local text model is downloaded and can run.
    pub local_model: bool,
    /// The default built-in speech model is downloaded.
    pub local_speech: bool,
    /// The spending limit is reached: cloud providers aren't used.
    pub over_budget: bool,
}

/// Spending limits for cloud providers, in US dollars. Unset means none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Budget {
    pub monthly_usd: Option<f64>,
    pub daily_usd: Option<f64>,
}

/// Whether an OpenAI-compatible base URL points at this computer.
pub fn is_local_url(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        // IPv6 literal, e.g. [::1]:8080
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    ) || host.ends_with(".localhost")
}

pub(super) enum KeyNeed {
    None,
    OpenAi,
    OpenRouter,
    Anthropic,
}

pub(super) fn provider_key_need_speech(p: SpeechProvider) -> KeyNeed {
    match p {
        SpeechProvider::Openai => KeyNeed::OpenAi,
        SpeechProvider::Openrouter => KeyNeed::OpenRouter,
        _ => KeyNeed::None,
    }
}

pub(super) fn provider_key_need_ai(p: AiProvider) -> KeyNeed {
    match p {
        AiProvider::Openai => KeyNeed::OpenAi,
        AiProvider::Openrouter => KeyNeed::OpenRouter,
        AiProvider::Anthropic => KeyNeed::Anthropic,
        _ => KeyNeed::None,
    }
}

pub(super) fn require_key(need: KeyNeed, secrets: &Secrets) -> Result<(), ResolveError> {
    match need {
        KeyNeed::OpenAi if secrets.openai.is_none() => {
            Err(ResolveError::MissingKey("OpenAI", "OPENAI_API_KEY"))
        }
        KeyNeed::OpenRouter if secrets.openrouter.is_none() => {
            Err(ResolveError::MissingKey("OpenRouter", "OPENROUTER_API_KEY"))
        }
        KeyNeed::Anthropic if secrets.anthropic.is_none() => {
            Err(ResolveError::MissingKey("Anthropic", "ANTHROPIC_API_KEY"))
        }
        _ => Ok(()),
    }
}
