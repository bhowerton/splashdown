//! The built-in local text model: a GGUF model from Hugging
//! Face run by llama.cpp in `splashdown-llm`, a helper process shipped next
//! to the app. Nothing else to install, unlike Ollama or LM Studio.
//!
//! The helper starts on first use (or when recording starts, via
//! `warm_up`), keeps the model loaded, and exits by itself after a while
//! idle; a request after that starts it again.

/// `CREATE_NO_WINDOW`.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use splashdown_core::sync::Locked;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use splashdown_core::local_llm::{LocalLlmReply, LocalLlmRequest};
use splashdown_core::{Completion, ProviderError, ProviderResult, TextModel, TextRequest, Usage};

/// The helper's file name, next to the app's executable.
const HELPER: &str = "splashdown-llm";
/// On Windows, a second build for any x64 processor, used when the first
/// (built for AVX2 and with the Vulkan backend) can't run here.
#[cfg(windows)]
const BASIC_HELPER: &str = "splashdown-llm-basic";

pub struct LocalLlm {
    model: PathBuf,
    cpu_only: bool,
    name: String,
    helper: Arc<Mutex<Helper>>,
}

struct Helper {
    /// The builds to try, best first: one is dropped if it fails before it
    /// has ever answered (`request`).
    paths: Vec<PathBuf>,
    /// A build has answered, so a failure now is a crash or an idle exit,
    /// not a build that can't run here.
    worked: bool,
    process: Option<Process>,
}

/// The helpers running, by process id, so they can all be stopped before
/// an update replaces their file (`stop_all_helpers`).
static RUNNING: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Stops every running helper. Windows can't replace a program's file
/// while it runs, so the updater would fail on it; one is started again
/// when it's next needed.
pub fn stop_all_helpers() {
    let pids = std::mem::take(&mut *RUNNING.locked());
    for pid in pids {
        #[cfg(windows)]
        let mut command = {
            use std::os::windows::process::CommandExt;
            let mut c = Command::new("taskkill");
            c.args(["/PID", &pid.to_string(), "/F"])
                .creation_flags(CREATE_NO_WINDOW);
            c
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut c = Command::new("kill");
            c.arg(pid.to_string());
            c
        };
        let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

/// How long the helper has to answer one request. Far longer than the
/// slowest computer takes to load the model and write a reply: past it the
/// helper is hung (a graphics driver, usually), and waiting on would hold
/// dictation up for good.
const ANSWER_LIMIT: Duration = Duration::from_secs(5 * 60);

struct Process {
    child: Child,
    stdin: ChildStdin,
    /// The helper's replies, a line each, read on a thread of their own
    /// so waiting for one can give up.
    replies: Receiver<std::io::Result<String>>,
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
        let helper = helper_paths()?;
        let stem = model
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model");
        Ok(Self {
            name: format!("local/{stem}{}", if cpu_only { " (cpu)" } else { "" }),
            model: model.to_path_buf(),
            cpu_only,
            helper: Arc::new(Mutex::new(Helper {
                paths: helper,
                worked: false,
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
        let reply = tokio::task::spawn_blocking(move || helper.locked().request(&request))
            .await
            .map_err(|e| ProviderError::Unavailable(format!("local model: {e}")))??;
        match reply {
            LocalLlmReply::Ok { text } => Ok(Completion {
                text,
                usage: Usage::FREE,
            }),
            LocalLlmReply::TooLong => Err(ProviderError::TooLong),
            LocalLlmReply::Vectors { .. } => Err(ProviderError::InvalidResponse(
                "local model: unexpected reply".into(),
            )),
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
            if let Err(e) = helper.locked().request(&request) {
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

/// The built-in embedding model (`models::Embedder`), run by the same
/// helper in a process of its own: one vector per piece of text, so notes
/// can be found by what they mean.
pub struct LocalEmbedder {
    model: PathBuf,
    helper: Mutex<Helper>,
}

/// What the embedding model is told a question is for (Qwen3 Embedding
/// reads questions with an instruction, and notes without).
const FIND_NOTES: &str = "Given a question, retrieve notes that help answer it";

impl LocalEmbedder {
    /// Checks the model and helper exist; starts nothing yet.
    pub fn new(model: &Path) -> ProviderResult<Self> {
        if !model.is_file() {
            return Err(ProviderError::Unavailable(format!(
                "the embedding model isn't downloaded ({})",
                model.display()
            )));
        }
        Ok(Self {
            model: model.to_path_buf(),
            helper: Mutex::new(Helper {
                paths: helper_paths()?,
                worked: false,
                process: None,
            }),
        })
    }

    /// A vector for each of `texts` (notes, or pieces of them), each of
    /// unit length. Blocks while the model works.
    pub fn embed(&self, texts: &[String]) -> ProviderResult<Vec<Vec<f32>>> {
        let request = LocalLlmRequest::Embed {
            model: self.model.clone(),
            cpu_only: false,
            texts: texts.to_vec(),
        };
        let reply = self.helper.locked().request(&request)?;
        match reply {
            LocalLlmReply::Vectors { vectors } if vectors.len() == texts.len() => Ok(vectors),
            LocalLlmReply::Error { message } => Err(ProviderError::Unavailable(format!(
                "embedding model: {message}"
            ))),
            _ => Err(ProviderError::InvalidResponse(
                "embedding model: unexpected reply".into(),
            )),
        }
    }

    /// The vector for a question, to compare with the notes' vectors.
    pub fn embed_question(&self, question: &str) -> ProviderResult<Vec<f32>> {
        let text = format!("Instruct: {FIND_NOTES}\nQuery:{question}");
        Ok(self.embed(&[text])?.remove(0))
    }
}

impl Helper {
    /// Sends one request and waits for its reply, starting the helper if
    /// it isn't running. A helper that has exited (idle, or crashed) is
    /// restarted once. A build that fails before it has ever answered (it
    /// can't start, or its graphics driver fails) is given up for the
    /// next one, if there is one.
    fn request(&mut self, request: &LocalLlmRequest) -> ProviderResult<LocalLlmReply> {
        let line = serde_json::to_string(request)
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
        let mut attempt = 0;
        loop {
            let result = match self.process.as_mut() {
                Some(process) => process.round_trip(&line, ANSWER_LIMIT),
                None => match self.spawn() {
                    Ok(process) => {
                        let process = self.process.insert(process);
                        process.round_trip(&line, ANSWER_LIMIT)
                    }
                    Err(e) => Err(std::io::Error::other(e.to_string())),
                },
            };
            match result {
                Ok(reply) => {
                    self.worked = true;
                    return Ok(reply);
                }
                Err(e) => {
                    self.stop();
                    // A helper that hung once would hang again: it isn't
                    // asked a second time (a build that has never answered
                    // still gives way to the next one, below).
                    let hung = e.kind() == std::io::ErrorKind::TimedOut;
                    if hung && (self.worked || self.paths.len() <= 1) {
                        return Err(ProviderError::Unavailable(format!(
                            "local model stopped: {e}"
                        )));
                    }
                    if !self.worked && self.paths.len() > 1 {
                        let failed = self.paths.remove(0);
                        tracing::warn!(
                            "{} failed before answering ({e}); using {} instead",
                            failed.display(),
                            self.paths[0].display()
                        );
                        continue;
                    }
                    attempt += 1;
                    if attempt == 2 {
                        return Err(ProviderError::Unavailable(format!(
                            "local model stopped: {e}"
                        )));
                    }
                }
            }
        }
    }

    fn spawn(&self) -> ProviderResult<Process> {
        let mut command = Command::new(&self.paths[0]);
        // The helper is a console program: no console window on Windows.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                ProviderError::Unavailable(format!("couldn't start the local model: {e}"))
            })?;
        let stdin = child.stdin.take().expect("piped");
        let mut stdout = BufReader::new(child.stdout.take().expect("piped"));
        let (send, replies) = channel();
        // Ends when the helper does (its output closes), or when nobody
        // waits for replies any more.
        let reading = std::thread::Builder::new()
            .name("splashdown-llm-replies".into())
            .spawn(move || {
                loop {
                    let mut line = String::new();
                    let read = match stdout.read_line(&mut line) {
                        Ok(0) => return,
                        Ok(_) => Ok(line),
                        Err(e) => Err(e),
                    };
                    let failed = read.is_err();
                    if send.send(read).is_err() || failed {
                        return;
                    }
                }
            });
        if let Err(e) = reading {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProviderError::Unavailable(format!(
                "couldn't start the local model: {e}"
            )));
        }
        RUNNING.locked().push(child.id());
        Ok(Process {
            child,
            stdin,
            replies,
        })
    }

    fn stop(&mut self) {
        if let Some(mut p) = self.process.take() {
            let pid = p.child.id();
            RUNNING.locked().retain(|&r| r != pid);
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
    }
}

impl Process {
    fn round_trip(&mut self, line: &str, limit: Duration) -> std::io::Result<LocalLlmReply> {
        writeln!(self.stdin, "{line}")?;
        self.stdin.flush()?;
        let reply = match self.replies.recv_timeout(limit) {
            Ok(reply) => reply?,
            Err(RecvTimeoutError::Timeout) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("no answer in {} s", limit.as_secs()),
                ));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the helper exited",
                ));
            }
        };
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
    helper_paths().is_ok()
}

/// The helper builds to use here, best first. On Windows the main build
/// needs AVX2 and the Vulkan loader (from the graphics driver; without it,
/// Windows would show an error box for the missing `vulkan-1.dll`), so the
/// basic build goes first where those are missing, and second otherwise.
fn helper_paths() -> ProviderResult<Vec<PathBuf>> {
    let main = helper_path()?;
    #[cfg(windows)]
    {
        let basic = main.with_file_name(format!("{BASIC_HELPER}{}", std::env::consts::EXE_SUFFIX));
        if basic.is_file() && std::env::var_os("SPLASHDOWN_LLM_HELPER").is_none() {
            return Ok(if fast_build_runs_here() {
                vec![main, basic]
            } else {
                tracing::info!("using the basic local model helper (no AVX2 or Vulkan here)");
                vec![basic]
            });
        }
    }
    Ok(vec![main])
}

/// The processor has what the main Windows build was compiled for, and
/// there's a Vulkan loader to link to.
#[cfg(windows)]
fn fast_build_runs_here() -> bool {
    #[cfg(target_arch = "x86_64")]
    let cpu = std::arch::is_x86_feature_detected!("avx2")
        && std::arch::is_x86_feature_detected!("fma")
        && std::arch::is_x86_feature_detected!("f16c")
        && std::arch::is_x86_feature_detected!("bmi2");
    #[cfg(not(target_arch = "x86_64"))]
    let cpu = false;
    let vulkan = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .is_some_and(|root| root.join("System32").join("vulkan-1.dll").is_file());
    cpu && vulkan
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A stand-in for the helper: a shell script.
    fn helper(name: &str, body: &str) -> Helper {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sd-llm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Helper {
            paths: vec![path],
            worked: false,
            process: None,
        }
    }

    #[test]
    fn a_helper_that_answers_is_heard_and_one_that_hangs_isn_t_waited_for() {
        // One reply per request, as the real helper gives.
        let mut answers = helper(
            "answers.sh",
            r#"while read line; do echo '{"status":"ok","text":"hi"}'; done"#,
        );
        let process = answers.spawn().unwrap();
        let process = answers.process.insert(process);
        for _ in 0..2 {
            let reply = process.round_trip("{}", Duration::from_secs(5)).unwrap();
            assert_eq!(reply, LocalLlmReply::Ok { text: "hi".into() });
        }

        // Takes the request and never answers.
        let mut hangs = helper("hangs.sh", "read line; sleep 30");
        let process = hangs.spawn().unwrap();
        let process = hangs.process.insert(process);
        let started = std::time::Instant::now();
        let error = process
            .round_trip("{}", Duration::from_millis(300))
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
        hangs.stop();

        // Exits without a word.
        let mut exits = helper("exits.sh", "read line");
        let process = exits.spawn().unwrap();
        let process = exits.process.insert(process);
        let error = process
            .round_trip("{}", Duration::from_secs(5))
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
