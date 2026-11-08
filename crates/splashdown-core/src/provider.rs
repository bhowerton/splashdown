//! Interfaces for speech-to-text and text models. Implementations
//! live in `splashdown-providers`; the core only depends on these traits.

use async_trait::async_trait;

use crate::AudioClip;

pub type ProviderResult<T> = std::result::Result<T, ProviderError>;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The provider can't run here (wrong OS, model not downloaded,
    /// Apple Intelligence off, local server not running, ...).
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("authentication failed — check the API key")]
    Auth,
    #[error("network error: {0}")]
    Network(String),
    #[error("provider returned an error: {0}")]
    Api(String),
    #[error("unexpected response: {0}")]
    InvalidResponse(String),
    /// Input exceeds what the model can take.
    #[error("input too long for this model")]
    TooLong,
    /// A streamed reply stopped part way. What it had cost by then
    /// (estimated) still counts toward spending.
    #[error("the reply stopped part way: {message}")]
    Interrupted { message: String, usage: Usage },
}

/// Hints passed to speech-to-text to improve accuracy.
#[derive(Debug, Clone, Default)]
pub struct SttHints {
    /// Custom vocabulary: names, jargon, product terms.
    pub vocabulary: Vec<String>,
    /// ISO-639-1 language code; `None` means auto-detect.
    pub language: Option<String>,
    /// Languages the speaker mixes, possibly mid-sentence (ISO codes).
    /// Only used when `language` is auto-detect.
    pub languages: Vec<String>,
}

impl SttHints {
    /// The languages being mixed, by name, when that applies.
    pub fn mixed_languages(&self) -> Vec<&'static str> {
        if self.language.is_some() || self.languages.len() < 2 {
            return Vec::new();
        }
        self.languages
            .iter()
            .filter_map(|c| crate::language::english_name(c))
            .collect()
    }

    /// A short prompt for speech models: preferred spellings, and the
    /// languages the speaker switches between.
    pub fn prompt(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.vocabulary.is_empty() {
            parts.push(format!("Vocabulary: {}.", self.vocabulary.join(", ")));
        }
        let mixed = self.mixed_languages();
        if !mixed.is_empty() {
            parts.push(format!(
                "The speaker switches between {}, sometimes mid-sentence; write each part in the language spoken.",
                list(&mixed)
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

/// "English, Spanish and French".
pub fn list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub usage: Usage,
    /// When each word was said, from the start of the clip, where the
    /// model says (local models); empty otherwise. For subtitles.
    pub words: Vec<crate::subtitles::TimedWord>,
}

/// A text model's answer and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub text: String,
    pub usage: Usage,
}

/// What provider calls cost, for the spending summary and budget. Prices
/// come from the provider's reply when it reports one (OpenRouter), else
/// from a built-in price list, so most figures are estimates.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    /// US dollars, for the calls whose price is known.
    pub cost_usd: f64,
    /// Some call's price wasn't known (an unlisted model, a remote custom
    /// server), so `cost_usd` leaves it out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unpriced: bool,
    /// A cloud provider failed and a model on this computer stood in (see
    /// `crate::fallback`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_device_fallback: bool,
}

impl Usage {
    /// On this computer: nothing to pay.
    pub const FREE: Usage = Usage {
        cost_usd: 0.0,
        unpriced: false,
        on_device_fallback: false,
    };

    pub fn priced(cost_usd: f64) -> Self {
        Self {
            cost_usd,
            ..Self::FREE
        }
    }

    /// A paid call whose price isn't known.
    pub fn unknown() -> Self {
        Self {
            unpriced: true,
            ..Self::FREE
        }
    }
}

impl std::ops::Add for Usage {
    type Output = Usage;
    fn add(self, other: Usage) -> Usage {
        Usage {
            cost_usd: self.cost_usd + other.cost_usd,
            unpriced: self.unpriced || other.unpriced,
            on_device_fallback: self.on_device_fallback || other.on_device_fallback,
        }
    }
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, other: Usage) {
        *self = *self + other;
    }
}

#[derive(Debug, Clone)]
pub struct TextRequest {
    pub system: String,
    /// User message, with transcript/selection/context in tagged blocks.
    pub user: String,
    pub temperature: f32,
    pub max_tokens: u32,
    /// JSON schema for structured output (smart split).
    pub json_schema: Option<serde_json::Value>,
}

#[async_trait]
pub trait SpeechToText: Send + Sync {
    /// Short identifier for logs and history, e.g. `openai/gpt-4o-mini-transcribe`.
    fn name(&self) -> String;
    async fn transcribe(&self, audio: &AudioClip, hints: &SttHints) -> ProviderResult<Transcript>;
    /// Gets ready for audio that's probably coming (recording just
    /// started): a local model loads itself if it was let go. Returns at
    /// once.
    fn warm_up(&self) {}
    /// False while a local model is still loading (the overlay says so).
    fn ready(&self) -> bool {
        true
    }
    /// A transcription that runs while the user speaks, for models that
    /// can. `None` means transcribe the recording when it ends.
    fn stream(&self, _hints: &SttHints) -> Option<Box<dyn SpeechStream>> {
        None
    }
}

/// Transcribes as the audio arrives, so the text is ready moments after
/// the user stops speaking, and partial text can be shown meanwhile.
pub trait SpeechStream: Send {
    /// Adds 16 kHz mono audio. Returns at once; the work happens elsewhere.
    fn feed(&mut self, samples: &[f32]);
    /// The text so far (it may still change), if it changed since the last
    /// call.
    fn partial(&mut self) -> Option<String>;
    /// Waits for the last audio fed and returns the final text.
    fn finish(self: Box<Self>) -> ProviderResult<String>;
}

/// Called with a streamed reply so far (see `TextModel::complete_streaming`).
pub type OnText<'a> = dyn for<'s> FnMut(&'s str) + Send + 'a;

#[async_trait]
pub trait TextModel: Send + Sync {
    /// Short identifier for logs and history, e.g. `openrouter/google/gemini-2.5-flash-lite`.
    fn name(&self) -> String;
    async fn complete(&self, req: &TextRequest) -> ProviderResult<Completion>;
    /// Like `complete`, calling `on_text` with the reply so far each time
    /// more of it arrives. Models that can't stream answer all at once and
    /// never call it.
    async fn complete_streaming(
        &self,
        req: &TextRequest,
        _on_text: &mut OnText<'_>,
    ) -> ProviderResult<Completion> {
        self.complete(req).await
    }
    fn supports_json_schema(&self) -> bool;
    /// Context window in tokens.
    fn context_window(&self) -> usize;
    /// Gets ready for a request that's probably coming (recording just
    /// started): a local model loads itself. Returns at once.
    fn warm_up(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_language_prompt() {
        let mut h = SttHints {
            vocabulary: vec!["Tauri".into()],
            language: None,
            languages: vec!["en".into(), "es".into()],
        };
        assert_eq!(
            h.prompt().unwrap(),
            "Vocabulary: Tauri. The speaker switches between English and Spanish, sometimes mid-sentence; write each part in the language spoken."
        );
        h.language = Some("en".into());
        assert_eq!(h.prompt().unwrap(), "Vocabulary: Tauri.");
        assert_eq!(list(&["A", "B", "C"]), "A, B and C");
    }
}
