//! What provider calls cost, for the spending summary and budget.
//!
//! OpenRouter reports each request's actual cost, which is used as-is.
//! For OpenAI and Anthropic the cost is worked out from the tokens the
//! reply reports and the list prices below. Prices change: update the table
//! and `PRICES_AS_OF` together. A model that isn't listed counts as
//! "unpriced" rather than guessed.

use serde_json::Value;
use splashdown_core::Usage;

use crate::endpoint::Flavor;

/// When the list prices below were last checked.
pub const PRICES_AS_OF: &str = "2026-10-29";

/// US dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TokenPrices {
    input: f64,
    /// Prompt tokens served from the provider's cache.
    cached_input: f64,
    /// Anthropic: prompt tokens written to the cache (5-minute entries).
    cache_write: f64,
    output: f64,
}

/// Text models, by provider and model id.
fn text_prices(flavor: Flavor, model: &str) -> Option<TokenPrices> {
    let p = |input, cached_input, cache_write, output| TokenPrices {
        input,
        cached_input,
        cache_write,
        output,
    };
    Some(match (flavor, model) {
        (Flavor::OpenAi, "gpt-4.1-mini") => p(0.40, 0.10, 0.40, 1.60),
        (Flavor::OpenAi, "gpt-4.1-nano") => p(0.10, 0.025, 0.10, 0.40),
        (Flavor::OpenAi, "gpt-4.1") => p(2.00, 0.50, 2.00, 8.00),
        // Cache reads at 0.1× and 5-minute cache writes at 1.25× input.
        (Flavor::Anthropic, "claude-haiku-5-5") => p(0.10, 0.01, 0.125, 0.50),
        (Flavor::Anthropic, "claude-sonnet-5-5") => p(2.00, 0.20, 2.50, 10.00),
        _ => return None,
    })
}

/// OpenAI transcription models: token prices (audio in, text out) and the
/// per-minute rate used when the reply reports only a duration.
fn transcription_prices(model: &str) -> Option<(TokenPrices, f64)> {
    let tokens = |audio_input, output| TokenPrices {
        input: audio_input,
        cached_input: audio_input,
        cache_write: audio_input,
        output,
    };
    Some(match model {
        "gpt-4o-mini-transcribe" => (tokens(3.00, 5.00), 0.003),
        "gpt-4o-transcribe" => (tokens(6.00, 10.00), 0.006),
        "whisper-1" => (tokens(0.0, 0.0), 0.006),
        _ => return None,
    })
}

fn tokens(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// OpenRouter's reported cost, when the reply carries one.
fn reported_cost(json: &Value) -> Option<f64> {
    json.get("usage")?.get("cost")?.as_f64()
}

/// The cost of a text completion, from its reply. `local` marks a custom
/// server on this computer, which costs nothing.
pub(crate) fn text_usage(flavor: Flavor, local: bool, model: &str, json: &Value) -> Usage {
    if let Some(cost) = reported_cost(json) {
        return Usage::priced(cost);
    }
    if flavor == Flavor::Custom && local {
        return Usage::FREE;
    }
    let (Some(prices), Some(usage)) = (text_prices(flavor, model), json.get("usage")) else {
        return Usage::unknown();
    };
    let per_token = |price: f64| price / 1_000_000.0;
    let cost = match flavor {
        // `input_tokens` excludes the cached parts, which are reported
        // separately.
        Flavor::Anthropic => {
            tokens(usage, "input_tokens") * per_token(prices.input)
                + tokens(usage, "cache_read_input_tokens") * per_token(prices.cached_input)
                + tokens(usage, "cache_creation_input_tokens") * per_token(prices.cache_write)
                + tokens(usage, "output_tokens") * per_token(prices.output)
        }
        // `prompt_tokens` includes the cached part.
        _ => {
            let prompt = tokens(usage, "prompt_tokens");
            let cached = usage
                .get("prompt_tokens_details")
                .map_or(0.0, |d| tokens(d, "cached_tokens"));
            (prompt - cached) * per_token(prices.input)
                + cached * per_token(prices.cached_input)
                + tokens(usage, "completion_tokens") * per_token(prices.output)
        }
    };
    Usage::priced(cost)
}

/// The cost of an OpenAI transcription, from its reply (token or duration
/// usage) or, failing that, the audio's length.
pub(crate) fn transcription_usage(model: &str, json: &Value, audio_seconds: f64) -> Usage {
    let Some((prices, per_minute)) = transcription_prices(model) else {
        return Usage::unknown();
    };
    let usage = json.get("usage");
    match usage.and_then(|u| u.get("type")).and_then(Value::as_str) {
        Some("tokens") => {
            let u = usage.expect("checked");
            Usage::priced(
                (tokens(u, "input_tokens") * prices.input
                    + tokens(u, "output_tokens") * prices.output)
                    / 1_000_000.0,
            )
        }
        Some("duration") => {
            Usage::priced(tokens(usage.expect("checked"), "seconds") / 60.0 * per_minute)
        }
        _ => Usage::priced(audio_seconds / 60.0 * per_minute),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn openrouter_reported_cost_wins() {
        let reply =
            json!({"usage": {"prompt_tokens": 1000, "completion_tokens": 50, "cost": 0.000123}});
        let u = text_usage(Flavor::OpenRouter, false, "anything", &reply);
        assert!(close(u.cost_usd, 0.000123) && !u.unpriced);
    }

    #[test]
    fn openai_tokens_with_cache() {
        let reply = json!({"usage": {
            "prompt_tokens": 1_000_000,
            "prompt_tokens_details": {"cached_tokens": 400_000},
            "completion_tokens": 100_000,
        }});
        let u = text_usage(Flavor::OpenAi, false, "gpt-4.1-mini", &reply);
        // 600k × $0.40 + 400k × $0.10 + 100k × $1.60, per million.
        assert!(close(u.cost_usd, 0.24 + 0.04 + 0.16), "{u:?}");
    }

    #[test]
    fn anthropic_counts_cache_reads_and_writes() {
        let reply = json!({"usage": {
            "input_tokens": 100,
            "cache_read_input_tokens": 1000,
            "cache_creation_input_tokens": 0,
            "output_tokens": 100,
        }});
        let u = text_usage(Flavor::Anthropic, false, "claude-haiku-5-5", &reply);
        assert!(close(
            u.cost_usd,
            (100.0 * 0.10 + 1000.0 * 0.01 + 100.0 * 0.50) / 1e6
        ));
    }

    #[test]
    fn unknown_models_are_unpriced_and_local_servers_free() {
        let reply = json!({"usage": {"prompt_tokens": 10, "completion_tokens": 10}});
        assert!(text_usage(Flavor::OpenAi, false, "gpt-9", &reply).unpriced);
        assert!(text_usage(Flavor::Custom, false, "qwen", &reply).unpriced);
        assert_eq!(
            text_usage(Flavor::Custom, true, "qwen", &reply),
            Usage::FREE
        );
    }

    #[test]
    fn transcription_by_tokens_duration_or_length() {
        let by_tokens =
            json!({"usage": {"type": "tokens", "input_tokens": 1000, "output_tokens": 100}});
        let u = transcription_usage("gpt-4o-mini-transcribe", &by_tokens, 0.0);
        assert!(close(u.cost_usd, (1000.0 * 3.0 + 100.0 * 5.0) / 1e6));
        let by_time = json!({"usage": {"type": "duration", "seconds": 30}});
        assert!(close(
            transcription_usage("whisper-1", &by_time, 0.0).cost_usd,
            0.003
        ));
        assert!(close(
            transcription_usage("gpt-4o-mini-transcribe", &json!({}), 60.0).cost_usd,
            0.003
        ));
        assert!(transcription_usage("other", &json!({}), 60.0).unpriced);
    }
}
