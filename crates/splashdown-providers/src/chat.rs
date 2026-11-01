//! Text model over the OpenAI Chat Completions API.

use async_trait::async_trait;
use serde_json::{Value, json};
use splashdown_core::{Completion, OnText, ProviderError, ProviderResult, TextModel, TextRequest};

use crate::endpoint::{Endpoint, Flavor, Streamed, chat_content, truncate};

pub struct ChatModel {
    endpoint: Endpoint,
    model: String,
    fallbacks: Vec<String>,
    context_window: usize,
}

impl ChatModel {
    pub fn new(endpoint: Endpoint, model: impl Into<String>) -> Self {
        Self {
            endpoint,
            model: model.into(),
            fallbacks: Vec::new(),
            context_window: 128_000,
        }
    }

    /// Models OpenRouter tries, in order, if the main model errors.
    /// Ignored by other endpoints.
    pub fn with_fallbacks(mut self, models: Vec<String>) -> Self {
        self.fallbacks = models;
        self
    }

    /// Overrides the assumed context window (small local models).
    pub fn with_context_window(mut self, tokens: usize) -> Self {
        self.context_window = tokens;
        self
    }

    /// OpenAI reasoning models reject `temperature` and spend part of the
    /// token budget on hidden reasoning.
    fn is_openai_reasoning_model(&self) -> bool {
        let m = self.model.as_str();
        self.endpoint.flavor() == Flavor::OpenAi
            && (m.starts_with("gpt-5")
                || m.starts_with("o1")
                || m.starts_with("o3")
                || m.starts_with("o4"))
    }

    /// `error`, ending a stream after `text` had come: then with what that
    /// probably cost.
    fn interrupted(&self, req: &TextRequest, text: &str, error: ProviderError) -> ProviderError {
        if text.is_empty() {
            return error;
        }
        ProviderError::Interrupted {
            message: error.to_string(),
            usage: crate::pricing::estimated_text_usage(
                self.endpoint.flavor(),
                self.endpoint.is_local(),
                &self.model,
                req.system.len() + req.user.len(),
                text.len(),
            ),
        }
    }

    /// What a reply cost, from its JSON (or the last chunk of a stream).
    fn usage(&self, json: &Value) -> splashdown_core::Usage {
        crate::pricing::text_usage(
            self.endpoint.flavor(),
            self.endpoint.is_local(),
            &self.model,
            json,
        )
    }

    pub(crate) fn body(&self, req: &TextRequest) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
        });

        if self.is_openai_reasoning_model() {
            body["max_completion_tokens"] = json!(req.max_tokens + 2048);
        } else {
            body["temperature"] = json!(req.temperature);
            let key = match self.endpoint.flavor() {
                Flavor::OpenAi => "max_completion_tokens",
                _ => "max_tokens",
            };
            body[key] = json!(req.max_tokens);
        }

        if self.endpoint.flavor() == Flavor::OpenRouter {
            // Report each request's actual cost in the reply.
            body["usage"] = json!({ "include": true });
        }
        if self.endpoint.flavor() == Flavor::OpenRouter && !self.fallbacks.is_empty() {
            let mut models = vec![self.model.clone()];
            models.extend(self.fallbacks.iter().cloned());
            body["models"] = json!(models);
        }

        if let Some(schema) = &req.json_schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": { "name": "result", "strict": true, "schema": schema },
            });
        }
        body
    }
}

#[async_trait]
impl TextModel for ChatModel {
    fn name(&self) -> String {
        format!("{}/{}", self.endpoint.flavor().label(), self.model)
    }

    async fn complete(&self, req: &TextRequest) -> ProviderResult<Completion> {
        let request = self.endpoint.post("chat/completions").json(&self.body(req));
        let json = self.endpoint.send_json(request).await?;
        Ok(Completion {
            text: chat_content(&json)?,
            usage: self.usage(&json),
        })
    }

    async fn complete_streaming(
        &self,
        req: &TextRequest,
        on_text: &mut OnText<'_>,
    ) -> ProviderResult<Completion> {
        let mut body = self.body(req);
        body["stream"] = json!(true);
        if self.endpoint.flavor() == Flavor::OpenAi {
            // OpenRouter sends usage at the end already (`usage.include`).
            body["stream_options"] = json!({ "include_usage": true });
        }
        for attempt in 0..2 {
            let request = self.endpoint.post("chat/completions").json(&body);
            let mut events = match self.endpoint.send_streaming(request).await? {
                Streamed::Events(events) => events,
                Streamed::Whole(json) => {
                    return Ok(Completion {
                        text: chat_content(&json)?,
                        usage: self.usage(&json),
                    });
                }
            };
            let mut reply = StreamedReply::default();
            let failed = loop {
                match events.next().await {
                    Ok(Some(data)) if data == "[DONE]" => {
                        reply.finished = true;
                        break None;
                    }
                    Ok(Some(data)) => match reply.take(&data) {
                        Ok(true) => on_text(&reply.text),
                        Ok(false) => {}
                        Err(e) => break Some(e),
                    },
                    Ok(None) => break None,
                    Err(e) => break Some(e),
                }
            };
            match failed {
                // A rate limit or upstream hiccup before any text: once
                // more, as `send_json` does.
                Some(_) if attempt == 0 && reply.transient && reply.text.is_empty() => {
                    tokio::time::sleep(crate::endpoint::RETRY_DELAY).await;
                }
                Some(e) => return Err(self.interrupted(req, &reply.text, e)),
                None if !reply.finished => {
                    let cut = ProviderError::Network("the reply was cut off".into());
                    return Err(self.interrupted(req, &reply.text, cut));
                }
                None => {
                    return Ok(Completion {
                        usage: self.usage(&reply.usage),
                        text: reply.text,
                    });
                }
            }
        }
        unreachable!("the second attempt always returns")
    }

    fn supports_json_schema(&self) -> bool {
        // Local servers vary; only trust the hosted APIs.
        self.endpoint.flavor() != Flavor::Custom
    }

    fn context_window(&self) -> usize {
        self.context_window
    }
}

/// A chat completion put together from its streamed chunks.
#[derive(Default)]
struct StreamedReply {
    text: String,
    /// The chunk that reported usage (the last one).
    usage: Value,
    /// A reason to stop arrived (a `finish_reason`, or `[DONE]`): the reply
    /// is whole.
    finished: bool,
    /// The error that ended it is worth a retry (rate limit, server error).
    transient: bool,
}

impl StreamedReply {
    /// Adds one chunk; true if the text grew.
    fn take(&mut self, data: &str) -> ProviderResult<bool> {
        let chunk: Value = serde_json::from_str(data)
            .map_err(|_| ProviderError::InvalidResponse(truncate(data, 200)))?;
        // OpenRouter reports an upstream failure mid-stream this way.
        if let Some(error) = chunk.get("error") {
            let message = error["message"]
                .as_str()
                .map_or_else(|| error.to_string(), str::to_string);
            let code = error["code"]
                .as_u64()
                .or_else(|| error["code"].as_str().and_then(|c| c.parse().ok()));
            self.transient = matches!(code, Some(429 | 500 | 502 | 503 | 504));
            return Err(ProviderError::Api(message));
        }
        if chunk.get("usage").is_some_and(|u| !u.is_null()) {
            self.usage = chunk.clone();
        }
        if chunk["choices"][0]["finish_reason"].is_string() {
            self.finished = true;
        }
        match chunk["choices"][0]["delta"]["content"].as_str() {
            Some(delta) if !delta.is_empty() => {
                self.text.push_str(delta);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> TextRequest {
        TextRequest {
            system: "sys".into(),
            user: "usr".into(),
            temperature: 0.2,
            max_tokens: 300,
            json_schema: None,
        }
    }

    #[test]
    fn openai_uses_max_completion_tokens() {
        let m = ChatModel::new(Endpoint::openai("k"), "gpt-4.1-nano");
        let b = m.body(&req());
        assert_eq!(b["max_completion_tokens"], 300);
        assert!(b.get("max_tokens").is_none());
        assert!((b["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(b["messages"][0]["role"], "system");
    }

    #[test]
    fn reasoning_models_drop_temperature() {
        let m = ChatModel::new(Endpoint::openai("k"), "gpt-5-nano");
        let b = m.body(&req());
        assert!(b.get("temperature").is_none());
        assert_eq!(b["max_completion_tokens"], 2348);
    }

    #[test]
    fn openrouter_sends_fallback_list() {
        let m = ChatModel::new(Endpoint::openrouter("k"), "a/primary")
            .with_fallbacks(vec!["b/backup".into()]);
        assert_eq!(m.body(&req())["models"], json!(["a/primary", "b/backup"]));
        let m =
            ChatModel::new(Endpoint::openai("k"), "gpt-4.1-nano").with_fallbacks(vec!["x".into()]);
        assert!(m.body(&req()).get("models").is_none());
    }

    #[test]
    fn streamed_chunks_make_the_reply() {
        let mut reply = StreamedReply::default();
        let grew = [
            r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
            r#"{"choices":[{"delta":{"content":"Hello"}}]}"#,
            r#"{"choices":[{"delta":{"content":" there."}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":3,"cost":0.0001}}"#,
        ]
        .map(|c| reply.take(c).unwrap());
        assert_eq!(grew, [false, true, true, false]);
        assert_eq!(reply.text, "Hello there.");
        assert_eq!(reply.usage["usage"]["completion_tokens"], 3);
        assert!(reply.finished);

        let mut reply = StreamedReply::default();
        assert!(
            reply
                .take(r#"{"error":{"code":502,"message":"upstream failed"}}"#)
                .is_err()
        );
        assert!(reply.transient && !reply.finished);
        let mut reply = StreamedReply::default();
        assert!(
            reply
                .take(r#"{"error":{"code":400,"message":"bad request"}}"#)
                .is_err()
        );
        assert!(!reply.transient);
    }

    #[test]
    fn openrouter_uses_max_tokens() {
        let m = ChatModel::new(Endpoint::openrouter("k"), "google/gemini-2.5-flash-lite");
        let b = m.body(&req());
        assert_eq!(b["max_tokens"], 300);
        assert_eq!(m.name(), "openrouter/google/gemini-2.5-flash-lite");
    }
}
