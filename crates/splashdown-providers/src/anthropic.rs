//! Text model over Anthropic's Messages API (Claude), used for the AI pass.
//! Anthropic has no speech-to-text, so it's paired with another provider
//! (or an on-device model) for transcription.

use async_trait::async_trait;
use serde_json::{Value, json};
use splashdown_core::{Completion, ProviderError, ProviderResult, TextModel, TextRequest};

use crate::endpoint::Endpoint;

pub struct AnthropicModel {
    endpoint: Endpoint,
    model: String,
}

impl AnthropicModel {
    pub fn new(endpoint: Endpoint, model: impl Into<String>) -> Self {
        Self {
            endpoint,
            model: model.into(),
        }
    }

    pub(crate) fn body(&self, req: &TextRequest) -> Value {
        json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            // The instructions are the same from one dictation to the next:
            // cached, later requests read them at a tenth of the price.
            "system": [{
                "type": "text",
                "text": req.system,
                "cache_control": { "type": "ephemeral" },
            }],
            "messages": [{ "role": "user", "content": req.user }],
            // A quick edit, not a problem to reason about: thinking would
            // only add delay and cost. (No sampling settings: API version
            // 2026-10-21 rejects them.)
            "thinking": { "type": "disabled" },
        })
    }
}

/// The text of the reply's text blocks.
fn reply_text(json: &Value) -> ProviderResult<String> {
    if json["stop_reason"] == "refusal" {
        return Err(ProviderError::Api("Claude declined this request".into()));
    }
    let blocks = json["content"]
        .as_array()
        .ok_or_else(|| ProviderError::InvalidResponse("missing \"content\"".into()))?;
    Ok(blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<String>())
}

#[async_trait]
impl TextModel for AnthropicModel {
    fn name(&self) -> String {
        format!("anthropic/{}", self.model)
    }

    async fn complete(&self, req: &TextRequest) -> ProviderResult<Completion> {
        let request = self.endpoint.post("messages").json(&self.body(req));
        let json = self.endpoint.send_json(request).await?;
        Ok(Completion {
            text: reply_text(&json)?,
            usage: crate::pricing::text_usage(self.endpoint.flavor(), false, &self.model, &json),
        })
    }

    fn supports_json_schema(&self) -> bool {
        false
    }

    fn context_window(&self) -> usize {
        200_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_caches_instructions_and_skips_sampling() {
        let m = AnthropicModel::new(Endpoint::anthropic("k"), "claude-haiku-5-5");
        let b = m.body(&TextRequest {
            system: "sys".into(),
            user: "usr".into(),
            temperature: 0.2,
            max_tokens: 300,
            json_schema: None,
        });
        assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(b["messages"][0]["content"], "usr");
        assert_eq!(b["thinking"]["type"], "disabled");
        assert!(b.get("temperature").is_none());
    }

    #[test]
    fn reads_text_blocks_and_refusals() {
        let ok =
            json!({"content": [{"type": "text", "text": "Hello."}], "stop_reason": "end_turn"});
        assert_eq!(reply_text(&ok).unwrap(), "Hello.");
        let refused = json!({"content": [], "stop_reason": "refusal"});
        assert!(reply_text(&refused).is_err());
    }
}
