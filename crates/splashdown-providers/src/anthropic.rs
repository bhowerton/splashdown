//! Text model over Anthropic's Messages API (Claude), used for the AI pass.
//! Anthropic has no speech-to-text, so it's paired with another provider
//! (or an on-device model) for transcription.

use async_trait::async_trait;
use serde_json::{Value, json};
use splashdown_core::{Completion, OnText, ProviderError, ProviderResult, TextModel, TextRequest};

use crate::endpoint::{Endpoint, Streamed, truncate};

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

    async fn complete_streaming(
        &self,
        req: &TextRequest,
        on_text: &mut OnText<'_>,
    ) -> ProviderResult<Completion> {
        let mut body = self.body(req);
        body["stream"] = json!(true);
        let request = self.endpoint.post("messages").json(&body);
        let mut events = match self.endpoint.send_streaming(request).await? {
            Streamed::Events(events) => events,
            Streamed::Whole(json) => {
                return Ok(Completion {
                    text: reply_text(&json)?,
                    usage: crate::pricing::text_usage(
                        self.endpoint.flavor(),
                        false,
                        &self.model,
                        &json,
                    ),
                });
            }
        };
        let mut reply = StreamedReply::default();
        while let Some(data) = events.next().await? {
            if reply.take(&data)? {
                on_text(&reply.text);
            }
        }
        let usage = json!({ "usage": reply.usage });
        Ok(Completion {
            text: reply.text,
            usage: crate::pricing::text_usage(self.endpoint.flavor(), false, &self.model, &usage),
        })
    }

    fn supports_json_schema(&self) -> bool {
        false
    }

    fn context_window(&self) -> usize {
        200_000
    }
}

/// A message put together from its streamed events.
#[derive(Default)]
struct StreamedReply {
    text: String,
    /// Token counts: input from `message_start`, output from `message_delta`.
    usage: serde_json::Map<String, Value>,
}

impl StreamedReply {
    /// Adds one event; true if the text grew.
    fn take(&mut self, data: &str) -> ProviderResult<bool> {
        let event: Value = serde_json::from_str(data)
            .map_err(|_| ProviderError::InvalidResponse(truncate(data, 200)))?;
        let counts = match event["type"].as_str() {
            Some("content_block_delta") => {
                return match event["delta"]["text"].as_str() {
                    Some(text) if !text.is_empty() => {
                        self.text.push_str(text);
                        Ok(true)
                    }
                    _ => Ok(false),
                };
            }
            Some("message_start") => &event["message"]["usage"],
            Some("message_delta") => {
                if event["delta"]["stop_reason"] == "refusal" {
                    return Err(ProviderError::Api("Claude declined this request".into()));
                }
                &event["usage"]
            }
            Some("error") => {
                let message = event["error"]["message"]
                    .as_str()
                    .unwrap_or("stream failed");
                return Err(ProviderError::Api(message.to_string()));
            }
            _ => return Ok(false),
        };
        if let Some(counts) = counts.as_object() {
            self.usage
                .extend(counts.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        Ok(false)
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
    fn streamed_events_make_the_reply() {
        let mut reply = StreamedReply::default();
        let grew = [
            r#"{"type":"message_start","message":{"usage":{"input_tokens":12,"cache_read_input_tokens":800,"output_tokens":1}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" there."}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}}"#,
            r#"{"type":"message_stop"}"#,
        ]
        .map(|e| reply.take(e).unwrap());
        assert_eq!(grew, [false, false, true, false, true, false, false]);
        assert_eq!(reply.text, "Hello there.");
        assert_eq!(reply.usage["input_tokens"], 12);
        assert_eq!(reply.usage["cache_read_input_tokens"], 800);
        assert_eq!(reply.usage["output_tokens"], 4);
        assert!(
            reply
                .take(
                    r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
                )
                .is_err()
        );
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
