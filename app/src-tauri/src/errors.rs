//! Turns provider and pipeline failures into one short line that says what
//! went wrong and what to do (shown in the overlay).

use splashdown_core::{PipelineError, ProviderError};

/// "openrouter/google/gemini…" → "OpenRouter".
pub fn provider_label(name: Option<&str>) -> &'static str {
    match name.and_then(|n| n.split('/').next()) {
        Some("openai") => "OpenAI",
        Some("openrouter") => "OpenRouter",
        Some("custom") => "Your local AI server",
        Some("local") => "Local speech-to-text",
        _ => "The provider",
    }
}

/// For a failed dictation.
pub fn pipeline(e: &PipelineError, stt: Option<&str>, llm: Option<&str>) -> String {
    match e {
        PipelineError::Stt(p) => provider(p, provider_label(stt), "speech-to-text"),
        PipelineError::Llm(p) => provider(p, provider_label(llm), "AI"),
        PipelineError::NoModel(_) => {
            "Instructions need an AI provider: choose one in Settings › Providers".into()
        }
        PipelineError::NoSelection => "Select some text first".into(),
        PipelineError::NoSpeech => "Didn't catch that".into(),
    }
}

fn provider(e: &ProviderError, who: &str, what: &str) -> String {
    match e {
        ProviderError::Auth => format!("{who} rejected the API key: check Settings › Providers"),
        ProviderError::Api(msg) => api_message(msg, who),
        ProviderError::Network(msg) if msg.contains("timed out") => {
            format!("No answer from {who}: check your connection")
        }
        ProviderError::Network(_) => format!("Can't reach {who}: are you offline?"),
        ProviderError::Unavailable(msg) if msg.contains("download") => {
            "The speech model isn't downloaded: see Settings › Providers".into()
        }
        ProviderError::Unavailable(msg) => format!("{who} unavailable: {msg}"),
        ProviderError::TooLong => format!("Too long for this {what} model"),
        ProviderError::InvalidResponse(_) => format!("{who} sent an unexpected reply; try again"),
        ProviderError::Interrupted { .. } => format!("{who}'s answer stopped part way; try again"),
    }
}

fn api_message(msg: &str, who: &str) -> String {
    let m = msg.to_lowercase();
    if m.starts_with("402")
        || m.contains("credit")
        || m.contains("insufficient")
        || m.contains("quota")
    {
        format!("{who} is out of credits: add credit or switch provider")
    } else if m.starts_with("429") || m.contains("rate limit") {
        format!("{who} is rate-limiting requests: try again in a moment")
    } else if m.starts_with('5') {
        format!("{who} is having problems right now; try again shortly")
    } else {
        format!("{who} returned an error; see the log for details")
    }
}

/// Why the AI pass was skipped (raw text inserted), from the error text the
/// pipeline keeps. Short, as it's appended to "Inserted".
pub fn fallback_reason(model_error: &str, llm: Option<&str>) -> String {
    let who = provider_label(llm);
    let m = model_error.to_lowercase();
    if m.contains("authentication") {
        format!("{who} rejected the key")
    } else if m.contains("402") || m.contains("credit") || m.contains("quota") {
        format!("{who} out of credits")
    } else if m.contains("429") || m.contains("rate") {
        format!("{who} rate-limited")
    } else if m.contains("network") || m.contains("timed out") {
        "offline?".into()
    } else {
        "AI unavailable".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OR: Option<&str> = Some("openrouter/google/gemini-2.5-flash-lite");

    #[test]
    fn classifies_provider_failures() {
        let e = |p| PipelineError::Llm(p);
        assert_eq!(
            pipeline(&e(ProviderError::Auth), None, OR),
            "OpenRouter rejected the API key: check Settings › Providers"
        );
        assert_eq!(
            pipeline(
                &e(ProviderError::Api(
                    "402 Payment Required: Insufficient credits".into()
                )),
                None,
                OR
            ),
            "OpenRouter is out of credits: add credit or switch provider"
        );
        assert_eq!(
            pipeline(
                &e(ProviderError::Api(
                    "429 Too Many Requests: slow down".into()
                )),
                None,
                OR
            ),
            "OpenRouter is rate-limiting requests: try again in a moment"
        );
        assert_eq!(
            pipeline(
                &e(ProviderError::Network("request timed out".into())),
                None,
                OR
            ),
            "No answer from OpenRouter: check your connection"
        );
        assert_eq!(
            pipeline(
                &PipelineError::Stt(ProviderError::Unavailable(
                    "speech model nemotron-en isn't downloaded".into()
                )),
                Some("local/nemotron-en"),
                None
            ),
            "The speech model isn't downloaded: see Settings › Providers"
        );
    }

    #[test]
    fn fallback_reasons_are_short() {
        assert_eq!(
            fallback_reason("network error: dns failure", OR),
            "offline?"
        );
        assert_eq!(
            fallback_reason(
                "provider returned an error: 402 Payment Required: credits",
                OR
            ),
            "OpenRouter out of credits"
        );
        assert_eq!(fallback_reason("something odd", OR), "AI unavailable");
    }
}
