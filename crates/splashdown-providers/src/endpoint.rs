//! HTTP endpoints: OpenAI, OpenRouter or any OpenAI-compatible base URL
//! (Ollama, LM Studio, Groq, ...), and Anthropic's Messages API.

use std::time::Duration;

use serde_json::Value;
use splashdown_core::ProviderError;

pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1";
/// The Messages API version we're written against (no sampling settings).
const ANTHROPIC_VERSION: &str = "2026-10-21";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    OpenAi,
    OpenRouter,
    Custom,
    Anthropic,
}

impl Flavor {
    pub fn label(self) -> &'static str {
        match self {
            Flavor::OpenAi => "openai",
            Flavor::OpenRouter => "openrouter",
            Flavor::Custom => "custom",
            Flavor::Anthropic => "anthropic",
        }
    }
}

#[derive(Clone)]
pub struct Endpoint {
    pub(crate) flavor: Flavor,
    base_url: String,
    api_key: Option<String>,
    pub(crate) http: reqwest::Client,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("flavor", &self.flavor)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Endpoint {
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self::new(Flavor::OpenAi, OPENAI_BASE_URL, Some(api_key.into()))
    }

    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self::new(
            Flavor::OpenRouter,
            OPENROUTER_BASE_URL,
            Some(api_key.into()),
        )
    }

    pub fn anthropic(api_key: impl Into<String>) -> Self {
        Self::new(Flavor::Anthropic, ANTHROPIC_BASE_URL, Some(api_key.into()))
    }

    pub fn custom(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self::new(Flavor::Custom, &base_url.into(), api_key)
    }

    fn new(flavor: Flavor, base_url: &str, api_key: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("HTTP client");
        Self {
            flavor,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.filter(|k| !k.is_empty()),
            http,
        }
    }

    pub fn flavor(&self) -> Flavor {
        self.flavor
    }

    /// A custom server on this computer (free to use).
    pub fn is_local(&self) -> bool {
        self.flavor == Flavor::Custom && splashdown_config::is_local_url(&self.base_url)
    }

    /// Checks that the endpoint is reachable and the key is accepted, with
    /// one cheap request that costs nothing.
    pub async fn verify(&self) -> Result<(), ProviderError> {
        let path = match self.flavor {
            // Returns the key's own info; fails with 401 for a bad key.
            Flavor::OpenRouter => "key",
            Flavor::OpenAi | Flavor::Custom | Flavor::Anthropic => "models",
        };
        self.send_json(self.authed(self.http.get(format!("{}/{path}", self.base_url))))
            .await
            .map(|_| ())
    }

    pub(crate) fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.authed(self.http.post(format!("{}/{path}", self.base_url)))
    }

    fn authed(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.flavor == Flavor::Anthropic {
            if let Some(key) = &self.api_key {
                req = req.header("x-api-key", key);
            }
            return req.header("anthropic-version", ANTHROPIC_VERSION);
        }
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        if self.flavor == Flavor::OpenRouter {
            // Attribution headers OpenRouter uses for its app directory.
            req = req
                .header("HTTP-Referer", "https://splashdown.app")
                .header("X-Title", "Splashdown");
        }
        req
    }

    /// Sends the request and returns the JSON body, mapping HTTP and API
    /// errors to `ProviderError`. Rate limits and transient server errors
    /// are retried once after a short pause (only for requests whose body
    /// can be cloned, i.e. JSON, not multipart uploads).
    pub(crate) async fn send_json(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<Value, ProviderError> {
        let retry = req.try_clone();
        match (Self::send_once(req).await, retry) {
            (Err(Failure::Transient(_)), Some(retry)) => {
                tokio::time::sleep(RETRY_DELAY).await;
                Self::send_once(retry).await.map_err(Failure::into_error)
            }
            (result, _) => result.map_err(Failure::into_error),
        }
    }

    async fn send_once(req: reqwest::RequestBuilder) -> Result<Value, Failure> {
        let resp = req
            .send()
            .await
            .map_err(|e| Failure::Fatal(network_error(e)))?;
        Self::read_reply(resp).await
    }

    /// Sends a request that asked for a streamed reply (`"stream": true`).
    /// Errors, and servers that answer all at once anyway, come back as
    /// `Streamed::Whole`, read like `send_json` (with its one retry).
    pub(crate) async fn send_streaming(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<Streamed, ProviderError> {
        let retry = req.try_clone();
        match (Self::stream_once(req).await, retry) {
            (Err(Failure::Transient(_)), Some(retry)) => {
                tokio::time::sleep(RETRY_DELAY).await;
                Self::stream_once(retry).await.map_err(Failure::into_error)
            }
            (result, _) => result.map_err(Failure::into_error),
        }
    }

    async fn stream_once(req: reqwest::RequestBuilder) -> Result<Streamed, Failure> {
        // A long answer from a slow model takes more than the usual 60 s;
        // a stalled one is caught between chunks instead (`Events::next`).
        let resp = req
            .timeout(STREAM_TIMEOUT)
            .send()
            .await
            .map_err(|e| Failure::Fatal(network_error(e)))?;
        let events = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if resp.status().is_success() && events {
            return Ok(Streamed::Events(Events {
                resp,
                buf: Vec::new(),
                lines: EventLines::default(),
            }));
        }
        Self::read_reply(resp).await.map(Streamed::Whole)
    }

    /// A reply's JSON body, or the error it reports.
    async fn read_reply(resp: reqwest::Response) -> Result<Value, Failure> {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| Failure::Fatal(network_error(e)))?;
        let json: Option<Value> = serde_json::from_str(&body).ok();

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(Failure::Fatal(ProviderError::Auth));
        }
        // OpenRouter can report upstream errors with a 200 status and the
        // real code inside the body.
        let error = json.as_ref().and_then(|j| j.get("error"));
        let code = error
            .and_then(|e| e.get("code"))
            .and_then(Value::as_u64)
            .unwrap_or(u64::from(status.as_u16()));
        let message = match error {
            Some(e) => e
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| e.to_string(), str::to_string),
            None if !status.is_success() => truncate(&body, 300),
            None => {
                return json.ok_or_else(|| {
                    Failure::Fatal(ProviderError::InvalidResponse(truncate(&body, 300)))
                });
            }
        };
        let err = ProviderError::Api(format!("{status}: {message}"));
        if matches!(code, 429 | 500 | 502 | 503 | 504) {
            Err(Failure::Transient(err))
        } else {
            Err(Failure::Fatal(err))
        }
    }
}

pub(crate) const RETRY_DELAY: Duration = Duration::from_millis(400);
/// The longest a streamed reply may take in all.
const STREAM_TIMEOUT: Duration = Duration::from_secs(600);
/// The longest wait for the next piece of a streamed reply.
const STREAM_IDLE: Duration = Duration::from_secs(30);

/// A reply to a request for a streamed answer.
pub(crate) enum Streamed {
    Events(Events),
    /// The whole answer at once.
    Whole(Value),
}

/// A server-sent event stream: each event's `data`, in order.
pub(crate) struct Events {
    resp: reqwest::Response,
    /// Received bytes not yet split into lines.
    buf: Vec<u8>,
    lines: EventLines,
}

impl Events {
    /// The next event's data, or `None` when the stream ends.
    pub(crate) async fn next(&mut self) -> Result<Option<String>, ProviderError> {
        loop {
            while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=end).collect();
                if let Some(data) = self.lines.push(&String::from_utf8_lossy(&line)) {
                    return Ok(Some(data));
                }
            }
            let chunk = tokio::time::timeout(STREAM_IDLE, self.resp.chunk())
                .await
                .map_err(|_| ProviderError::Network("the reply stopped coming".into()))?;
            match chunk.map_err(network_error)? {
                Some(bytes) => self.buf.extend_from_slice(&bytes),
                None => {
                    // A last line without its line break, then what's left.
                    let rest = std::mem::take(&mut self.buf);
                    let last = self.lines.push(&String::from_utf8_lossy(&rest));
                    return Ok(last.or_else(|| self.lines.push("")));
                }
            }
        }
    }
}

/// Puts an event stream's lines back together into events.
#[derive(Default)]
struct EventLines {
    /// The current event's data so far.
    data: String,
}

impl EventLines {
    /// Takes one line; returns an event's data when a blank line ends it.
    /// Comments, `event:` and `id:` lines are skipped.
    fn push(&mut self, line: &str) -> Option<String> {
        let line = line.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            return (!self.data.is_empty()).then(|| std::mem::take(&mut self.data));
        }
        if let Some(data) = line.strip_prefix("data:") {
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
        }
        None
    }
}

enum Failure {
    /// Worth one retry: rate limit or upstream hiccup.
    Transient(ProviderError),
    Fatal(ProviderError),
}

impl Failure {
    fn into_error(self) -> ProviderError {
        match self {
            Failure::Transient(e) | Failure::Fatal(e) => e,
        }
    }
}

fn network_error(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Network("request timed out".into())
    } else {
        ProviderError::Network(e.to_string())
    }
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Extracts `choices[0].message.content` from a chat completion.
pub(crate) fn chat_content(json: &Value) -> Result<String, ProviderError> {
    let content = &json["choices"][0]["message"]["content"];
    match content {
        Value::String(s) => Ok(s.clone()),
        // Some providers return content as an array of parts.
        Value::Array(parts) => Ok(parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("")),
        Value::Null => Ok(String::new()),
        other => Err(ProviderError::InvalidResponse(truncate(
            &other.to_string(),
            200,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_string_and_array_content() {
        let j = json!({"choices": [{"message": {"content": "Hello."}}]});
        assert_eq!(chat_content(&j).unwrap(), "Hello.");
        let j = json!({"choices": [{"message": {"content": [{"type": "text", "text": "He"}, {"type": "text", "text": "llo"}]}}]});
        assert_eq!(chat_content(&j).unwrap(), "Hello");
        let j = json!({"choices": [{"message": {"content": null}}]});
        assert_eq!(chat_content(&j).unwrap(), "");
    }

    #[test]
    fn debug_redacts_key() {
        let e = Endpoint::openai("sk-secret");
        assert!(!format!("{e:?}").contains("sk-secret"));
    }

    #[test]
    fn event_lines_make_events() {
        let mut lines = EventLines::default();
        let mut events = Vec::new();
        for line in [
            ": OPENROUTER PROCESSING\n",
            "\n",
            "event: content_block_delta\r\n",
            "data: {\"a\":1}\r\n",
            "\r\n",
            "data:two\n",
            "data: lines\n",
            "\n",
            "data: [DONE]\n",
            "",
        ] {
            events.extend(lines.push(line));
        }
        assert_eq!(events, ["{\"a\":1}", "two\nlines", "[DONE]"]);
    }

    #[test]
    fn truncates_on_char_boundary() {
        assert_eq!(truncate("héllo", 2), "hé…");
        assert_eq!(truncate("hi", 5), "hi");
    }
}
