//! Text model over the OpenAI Chat Completions API.

use async_trait::async_trait;
use serde_json::{Value, json};
use splashdown_core::{Completion, ProviderResult, TextModel, TextRequest};

use crate::endpoint::{Endpoint, Flavor, chat_content};

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
            usage: crate::pricing::text_usage(
                self.endpoint.flavor(),
                self.endpoint.is_local(),
                &self.model,
                &json,
            ),
        })
    }

    fn supports_json_schema(&self) -> bool {
        // Local servers vary; only trust the hosted APIs.
        self.endpoint.flavor() != Flavor::Custom
    }

    fn context_window(&self) -> usize {
        self.context_window
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
    fn openrouter_uses_max_tokens() {
        let m = ChatModel::new(Endpoint::openrouter("k"), "google/gemini-2.5-flash-lite");
        let b = m.body(&req());
        assert_eq!(b["max_tokens"], 300);
        assert_eq!(m.name(), "openrouter/google/gemini-2.5-flash-lite");
    }
}
