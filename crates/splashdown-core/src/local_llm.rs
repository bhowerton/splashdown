//! Messages between the app and `splashdown-llm`, the helper process that
//! runs the built-in local text model. One JSON object per line on the
//! helper's stdin and stdout. The model runs in its own process because
//! llama.cpp and whisper.cpp each bundle a different copy of the ggml
//! engine, which can't share one binary.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum LocalLlmRequest {
    /// Load the model now (while the user is still speaking), so the first
    /// completion doesn't wait for it. Replies `Ok` with empty text.
    Load { model: PathBuf, cpu_only: bool },
    Complete {
        model: PathBuf,
        cpu_only: bool,
        system: String,
        user: String,
        temperature: f32,
        max_tokens: u32,
    },
}

impl LocalLlmRequest {
    /// Which model, and where it runs.
    pub fn target(&self) -> (&PathBuf, bool) {
        match self {
            LocalLlmRequest::Load { model, cpu_only }
            | LocalLlmRequest::Complete {
                model, cpu_only, ..
            } => (model, *cpu_only),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LocalLlmReply {
    Ok {
        text: String,
    },
    /// The prompt doesn't fit in the model's context.
    TooLong,
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_is_tagged_json() {
        let req = LocalLlmRequest::Load {
            model: "/m.gguf".into(),
            cpu_only: false,
        };
        let line = serde_json::to_string(&req).unwrap();
        assert_eq!(line, r#"{"cmd":"load","model":"/m.gguf","cpu_only":false}"#);
        assert_eq!(serde_json::from_str::<LocalLlmRequest>(&line).unwrap(), req);

        let reply: LocalLlmReply = serde_json::from_str(r#"{"status":"too_long"}"#).unwrap();
        assert_eq!(reply, LocalLlmReply::TooLong);
    }
}
