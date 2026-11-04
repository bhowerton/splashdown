//! Settings → AI Models: the speech-to-text and AI providers and models,
//! and the default models.

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeechSettings {
    pub provider: SpeechProvider,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub provider: AiProvider,
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// OpenRouter only: models to try, in order, if the main one errors
    /// (rate limits, outages). Unset means the default fallback.
    pub fallback_models: Option<Vec<String>>,
    /// When cleanup runs on this computer (Apple, a local server), send
    /// spoken instructions and edits of selected text to a cloud model
    /// instead, if a key is saved. Plain dictation stays local.
    pub cloud_for_instructions: bool,
    /// How long the built-in model took to tidy up a typical dictation on
    /// this computer, in milliseconds (the setup guide measures it).
    pub local_ms: Option<u32>,
    /// Keep the built-in model although it's slow here: no suggestions to
    /// use a cloud key instead.
    pub slow_local_ok: bool,
}

/// The built-in model tidies up a typical dictation in under this many
/// milliseconds on a computer that's fast enough for it.
pub const LOCAL_FAST_MS: u32 = 2_000;

/// Over this, it's too slow to be pleasant: a cloud key is suggested.
pub const LOCAL_SLOW_MS: u32 = 5_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechProvider {
    #[default]
    Auto,
    Openai,
    Openrouter,
    /// On this computer, with transcribe.cpp: Nemotron, Parakeet or
    /// Whisper. Named for Whisper, its first model, so existing settings
    /// files keep working.
    WhisperLocal,
    Apple,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiProvider {
    #[default]
    Auto,
    Openai,
    Openrouter,
    /// Claude (Anthropic's API). Cleanup only.
    Anthropic,
    Custom,
    Apple,
    /// The built-in llama.cpp model (a GGUF file in the models folder).
    Local,
    Off,
}

impl std::str::FromStr for SpeechProvider {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "auto" => Self::Auto,
            "openai" => Self::Openai,
            "openrouter" => Self::Openrouter,
            "whisper-local" | "whisper" | "local" => Self::WhisperLocal,
            "apple" => Self::Apple,
            _ => {
                return Err(format!(
                    "unknown speech provider \"{s}\" (auto, openai, openrouter, whisper-local, apple)"
                ));
            }
        })
    }
}

impl std::str::FromStr for AiProvider {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "auto" => Self::Auto,
            "openai" => Self::Openai,
            "openrouter" => Self::Openrouter,
            "anthropic" | "claude" => Self::Anthropic,
            "custom" => Self::Custom,
            "apple" => Self::Apple,
            "local" => Self::Local,
            "off" | "none" => Self::Off,
            _ => {
                return Err(format!(
                    "unknown AI provider \"{s}\" (auto, openai, openrouter, anthropic, custom, apple, local, off)"
                ));
            }
        })
    }
}

/// Default models, chosen from the evals (see `eval/README.md`).
pub mod defaults {
    pub const OPENAI_STT: &str = "gpt-4o-mini-transcribe";
    /// Gemini 2.5 Flash Lite was rate-limited on most requests; Voxtral
    /// followed spoken instructions instead of transcribing them.
    pub const OPENROUTER_STT: &str = "google/gemini-3.1-flash-lite";
    /// Streams as the user speaks, so the text is ready about 0.03 s after
    /// release even on a CPU (Windows has no GPU build), with the best
    /// English of the local models and 32 languages (eval/README.md,
    /// 2026-11-01).
    pub const LOCAL_SPEECH: &str = "nemotron";
    /// 95% on the cleanup eval (via OpenRouter); gpt-4.1-nano scored 84% and
    /// the GPT-5 reasoning models took 3–7 s per cleanup.
    pub const OPENAI_AI: &str = "gpt-4.1-mini";
    pub const OPENROUTER_AI: &str = "google/gemini-2.5-flash-lite";
    /// Tied on the cleanup eval with no rate limiting, but weaker against
    /// injected instructions, so it's the fallback rather than the default.
    pub const OPENROUTER_AI_FALLBACK: &str = "mistralai/mistral-small-3.2-24b-instruct";
    pub const CUSTOM_BASE_URL: &str = "http://localhost:11434/v1";
    /// Anthropic's cheapest current model, with thinking off (see
    /// `eval/README.md` for how it scores).
    pub const ANTHROPIC_AI: &str = "claude-haiku-5-5";
    /// The built-in local model (`AiProvider::Local`).
    pub const LOCAL_AI: &str = "qwen3-4b";
    /// Model name for providers where the OS picks the model (Apple).
    pub const SYSTEM_MODEL: &str = "system";
}
