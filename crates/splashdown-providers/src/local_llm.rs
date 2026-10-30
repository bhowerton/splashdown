//! The built-in local text model (spec §5.6): a GGUF model from Hugging
//! Face run by llama.cpp in `splashdown-llm`, a helper process shipped next
//! to the app. Nothing else to install, unlike Ollama or LM Studio.
//!
//! The helper starts on first use (or when recording starts, via
//! `warm_up`), keeps the model loaded, and exits by itself after a while
//! idle; a request after that starts it again.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use splashdown_core::local_llm::{LocalLlmReply, LocalLlmRequest};
use splashdown_core::{Completion, ProviderError, ProviderResult, TextModel, TextRequest, Usage};

/// The helper's file name, next to the app's executable.
const HELPER: &str = "splashdown-llm";

pub struct LocalLlm {
    model: PathBuf,
    cpu_only: bool,
    name: String,
    helper: Arc<Mutex<Helper>>,
}

struct Helper {
    path: PathBuf,
    process: Option<Process>,
}

struct Process {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl LocalLlm {
    /// Checks the model and helper exist; starts nothing yet. `cpu_only`
    /// keeps the model off the GPU, to measure machines without one.
    pub fn new(model: &Path, cpu_only: bool) -> ProviderResult<Self> {
        if !model.is_file() {
            return Err(ProviderError::Unavailable(format!(
                "the local model isn't downloaded ({})",
                model.display()
            )));
        }
        let helper = helper_path()?;
        let stem = model
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model");
        Ok(Self {
            name: format!("local/{stem}{}", if cpu_only { " (cpu)" } else { "" }),
            model: model.to_path_buf(),
            cpu_only,
            helper: Arc::new(Mutex::new(Helper {
                path: helper,
                process: None,
            })),
        })
    }
}

#[async_trait]
impl TextModel for LocalLlm {
    fn name(&self) -> String {
        self.name.clone()
    }

    async fn complete(&self, req: &TextRequest) -> ProviderResult<Completion> {
        let request = LocalLlmRequest::Complete {
            model: self.model.clone(),
            cpu_only: self.cpu_only,
            system: req.system.clone(),
            user: req.user.clone(),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
        };
        let helper = self.helper.clone();
        let reply = tokio::task::spawn_blocking(move || {
            helper
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .request(&request)
        })
        .await
        .map_err(|e| ProviderError::Unavailable(format!("local model: {e}")))??;
        match reply {
            LocalLlmReply::Ok { text } => Ok(Completion {
                text,
                usage: Usage::FREE,
            }),
            LocalLlmReply::TooLong => Err(ProviderError::TooLong),
            LocalLlmReply::Error { message } => Err(ProviderError::Unavailable(format!(
                "local model: {message}"
            ))),
        }
    }

    /// Starts the helper and loads the model in the background.
    fn warm_up(&self) {
        let request = LocalLlmRequest::Load {
            model: self.model.clone(),
            cpu_only: self.cpu_only,
        };
        let helper = self.helper.clone();
        std::thread::spawn(move || {
            if let Err(e) = helper
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .request(&request)
            {
                tracing::warn!("local model didn't start: {e}");
            }
        });
    }

    fn supports_json_schema(&self) -> bool {
        false
    }

    fn context_window(&self) -> usize {
        // The helper's context size (it answers `TooLong` beyond it).
        4096
    }
}

impl Helper {
    /// Sends one request and waits for its reply, starting the helper if
    /// it isn't running. A helper that has exited (idle, or crashed) is
    /// restarted once.
    fn request(&mut self, request: &LocalLlmRequest) -> ProviderResult<LocalLlmReply> {
        let line = serde_json::to_string(request)
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
        for attempt in 0..2 {
            if self.process.is_none() {
                self.process = Some(self.spawn()?);
            }
            let process = self.process.as_mut().expect("just started");
            match process.round_trip(&line) {
                Ok(reply) => return Ok(reply),
                Err(e) => {
                    self.stop();
                    if attempt == 1 {
                        return Err(ProviderError::Unavailable(format!(
                            "local model stopped: {e}"
                        )));
                    }
                }
            }
        }
        unreachable!("the loop returns")
    }

    fn spawn(&self) -> ProviderResult<Process> {
        let mut child = Command::new(&self.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                ProviderError::Unavailable(format!("couldn't start the local model: {e}"))
            })?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        Ok(Process {
            child,
            stdin,
            stdout,
        })
    }

    fn stop(&mut self) {
        if let Some(mut p) = self.process.take() {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
    }
}

impl Process {
    fn round_trip(&mut self, line: &str) -> std::io::Result<LocalLlmReply> {
        writeln!(self.stdin, "{line}")?;
        self.stdin.flush()?;
        let mut reply = String::new();
        if self.stdout.read_line(&mut reply)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the helper exited",
            ));
        }
        serde_json::from_str(&reply)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Whether the helper is where the app will look for it.
pub(crate) fn helper_installed() -> bool {
    helper_path().is_ok()
}

/// `splashdown-llm` next to the running executable (inside the app bundle,
/// or in `target/<profile>` during development). `SPLASHDOWN_LLM_HELPER`
/// overrides it.
fn helper_path() -> ProviderResult<PathBuf> {
    if let Some(p) = std::env::var_os("SPLASHDOWN_LLM_HELPER") {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe()
        .map_err(|e| ProviderError::Unavailable(format!("can't find the app: {e}")))?;
    let path = exe.with_file_name(format!("{HELPER}{}", std::env::consts::EXE_SUFFIX));
    if path.is_file() {
        Ok(path)
    } else {
        Err(ProviderError::Unavailable(format!(
            "the local model helper is missing ({}); build it with `cargo build -p splashdown-llm`",
            path.display()
        )))
    }
}
