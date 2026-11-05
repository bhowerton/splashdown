//! `splashdown-mcp` — voice input and notes for coding agents. An MCP
//! server over stdio. `ask_user_by_voice`: Splashdown shows the question,
//! records the user's spoken answer hands-free, and the cleaned-up text
//! goes back to the agent. `search_notes`, `read_note` and `append_note`
//! reach the user's notes, if Settings allows it. It talks to the running
//! app over its control socket (`splashdown-control`). With arguments it
//! sets agents up with itself instead (`splashdown-mcp install
//! claude-code`; `setup.rs`). Setup: docs/AGENTS.md.

mod mcp;
mod setup;

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use splashdown_control::{ClientError, Request};

use crate::mcp::Step;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = setup::main(&args) {
        std::process::exit(code);
    }
    let out = Arc::new(Mutex::new(std::io::stdout()));
    // The `tools/call` waiting for the user, if any. The app records one
    // answer at a time.
    let waiting: Arc<Mutex<Option<Value>>> = Arc::default();

    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match mcp::handle(&line) {
            Step::Send(msg) => write(&out, &msg),
            Step::Ask { id, question } => {
                {
                    let mut w = lock(&waiting);
                    if w.is_some() {
                        let busy =
                            Err("Already waiting for the user's answer to another question".into());
                        write(&out, &mcp::response(id, mcp::tool_result(busy)));
                        continue;
                    }
                    *w = Some(id.clone());
                }
                let (out, waiting) = (out.clone(), waiting.clone());
                std::thread::spawn(move || {
                    let answer = ask(question);
                    let mut w = lock(&waiting);
                    // Canceled meanwhile: the client expects no reply.
                    if w.as_ref() == Some(&id) {
                        *w = None;
                        write(&out, &mcp::response(id, mcp::tool_result(answer)));
                    }
                });
            }
            Step::Control { id, request } => {
                let out = out.clone();
                std::thread::spawn(move || {
                    let result = match splashdown_control::send(&request) {
                        Ok(text) => Ok(text.unwrap_or_else(|| "Done".into())),
                        Err(ClientError::NotRunning) => {
                            Err("Splashdown isn't running, so its notes can't be reached.".into())
                        }
                        Err(e) => Err(e.to_string()),
                    };
                    write(&out, &mcp::response(id, mcp::tool_result(result)));
                });
            }
            Step::Canceled(id) => {
                let mut w = lock(&waiting);
                if w.as_ref() == Some(&id) {
                    *w = None;
                    // Stop the recording; the asking thread's reply is dropped.
                    std::thread::spawn(|| {
                        let _ = splashdown_control::send(&Request::Cancel);
                    });
                }
            }
            Step::Nothing => {}
        }
    }
}

/// Asks the running app to record an answer to `question`.
fn ask(question: String) -> Result<String, String> {
    match splashdown_control::send(&Request::Listen {
        prompt: Some(question),
    }) {
        Ok(Some(text)) if !text.trim().is_empty() => Ok(text),
        Ok(_) => Err("The user didn't say anything. Ask in the chat instead.".into()),
        Err(ClientError::NotRunning) => Err(
            "Splashdown isn't running, so the user can't answer by voice. Ask in the chat instead."
                .into(),
        ),
        Err(e) => Err(format!("No spoken answer ({e}). Ask in the chat instead.")),
    }
}

fn write(out: &Mutex<std::io::Stdout>, msg: &Value) {
    let mut out = out.lock().unwrap_or_else(|e| e.into_inner());
    // `Value`'s compact form never contains a raw newline.
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
