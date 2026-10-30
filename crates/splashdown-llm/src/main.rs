//! `splashdown-llm`: runs Splashdown's built-in local text model (a GGUF
//! file, via llama.cpp) for the app, in its own process. It reads
//! `LocalLlmRequest`s as JSON lines on stdin and answers each with one
//! `LocalLlmReply` line on stdout (see `splashdown_core::local_llm`).
//!
//! It keeps the model and its processed prompt between requests: the long,
//! unchanging system prompt is evaluated once, so later requests only pay
//! for the new text. That's what makes CPU-only machines usable. After a
//! while without requests it exits, returning its memory; the app starts it
//! again on demand.

use std::io::{BufRead, Write};
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use splashdown_core::local_llm::{LocalLlmReply, LocalLlmRequest};

/// Exit after this long without a request, to give the memory back.
const IDLE_EXIT: Duration = Duration::from_secs(10 * 60);
/// Context size: room for the cleanup prompt (~1,000 tokens), the user's
/// text and the reply. Larger contexts cost memory for the whole session.
const N_CTX: u32 = 4096;
/// Tokens evaluated per batch while reading a prompt.
const N_BATCH: u32 = 512;
/// Same seed every time, so a given input cleans up the same way.
const SEED: u32 = 1234;
/// Starts a hybrid model's reply with an empty thought (Qwen3's template
/// does the same when thinking is off).
const NO_THINKING: &str = "<think>\n\n</think>\n\n";

/// Seconds since start of the last request, for the idle watchdog.
static LAST_REQUEST: AtomicU64 = AtomicU64::new(0);

fn main() {
    let mut backend = match LlamaBackend::init() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("splashdown-llm: llama.cpp didn't start: {e}");
            std::process::exit(1);
        }
    };
    backend.void_logs();

    let started = Instant::now();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(15));
            let idle = started.elapsed().as_secs() - LAST_REQUEST.load(Ordering::Relaxed);
            if idle >= IDLE_EXIT.as_secs() {
                std::process::exit(0);
            }
        }
    });

    let mut requests = std::io::stdin().lock().lines().map_while(|line| {
        let line = line.ok()?;
        LAST_REQUEST.store(started.elapsed().as_secs(), Ordering::Relaxed);
        Some(serde_json::from_str::<LocalLlmRequest>(&line).map_err(|e| e.to_string()))
    });
    let mut out = std::io::stdout().lock();
    let mut send = |reply: LocalLlmReply| {
        let line = serde_json::to_string(&reply).expect("replies serialize");
        // The app is gone if this fails; the next read ends the loop.
        let _ = writeln!(out, "{line}").and_then(|()| out.flush());
    };

    // One model at a time; a request for another model reloads.
    let mut next = requests.next();
    while let Some(request) = next.take() {
        let request = match request {
            Ok(r) => r,
            Err(e) => {
                send(error(format!("bad request: {e}")));
                next = requests.next();
                continue;
            }
        };
        let (path, cpu_only) = request.target();
        let (path, cpu_only) = (path.clone(), cpu_only);
        let model = match load(&backend, &path, cpu_only) {
            Ok(m) => m,
            Err(e) => {
                send(error(e));
                next = requests.next();
                continue;
            }
        };
        let mut session = match Session::new(&backend, &model) {
            Ok(s) => s,
            Err(e) => {
                send(error(e));
                next = requests.next();
                continue;
            }
        };

        let mut request = request;
        loop {
            send(session.handle(&request));
            match requests.next() {
                Some(Ok(r)) if r.target() == (&path, cpu_only) => request = r,
                // Another model, a bad line, or the end of input.
                other => {
                    next = other;
                    break;
                }
            }
        }
    }
}

fn error(message: impl Into<String>) -> LocalLlmReply {
    LocalLlmReply::Error {
        message: message.into(),
    }
}

fn load(backend: &LlamaBackend, path: &Path, cpu_only: bool) -> Result<LlamaModel, String> {
    if !path.is_file() {
        return Err(format!("model not found at {}", path.display()));
    }
    // All layers on the GPU (llama.cpp caps this at the model's count).
    let gpu_layers = if cpu_only { 0 } else { 1000 };
    let params = LlamaModelParams::default().with_n_gpu_layers(gpu_layers);
    LlamaModel::load_from_file(backend, path, &params)
        .map_err(|e| format!("couldn't load {}: {e}", path.display()))
}

/// A loaded model with a context whose cache holds the last prompt.
struct Session<'m> {
    model: &'m LlamaModel,
    template: LlamaChatTemplate,
    /// The model thinks first unless its reply starts with an empty thought.
    hybrid_thinking: bool,
    ctx: LlamaContext<'m>,
    /// The tokens currently in the context's cache, in order.
    cached: Vec<LlamaToken>,
}

impl<'m> Session<'m> {
    fn new(backend: &LlamaBackend, model: &'m LlamaModel) -> Result<Self, String> {
        let template = model
            .chat_template(None)
            .map_err(|e| format!("model has no usable chat template: {e}"))?;
        // Hybrid models (Qwen3) think unless told not to, through a template
        // variable that llama.cpp's built-in templates don't know about.
        let hybrid_thinking = template
            .to_str()
            .is_ok_and(|t| t.contains("enable_thinking"));
        let threads = threads();
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(N_CTX.min(model.n_ctx_train())))
            .with_n_batch(N_BATCH)
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
        let ctx = model
            .new_context(backend, params)
            .map_err(|e| format!("couldn't create a context: {e}"))?;
        Ok(Self {
            model,
            template,
            hybrid_thinking,
            ctx,
            cached: Vec::new(),
        })
    }

    fn handle(&mut self, request: &LocalLlmRequest) -> LocalLlmReply {
        match request {
            LocalLlmRequest::Load { .. } => LocalLlmReply::Ok {
                text: String::new(),
            },
            LocalLlmRequest::Complete {
                system,
                user,
                temperature,
                max_tokens,
                ..
            } => match self.complete(system, user, *temperature, *max_tokens) {
                Ok(reply) => reply,
                Err(e) => {
                    // The cache may be half-updated; start clean next time.
                    self.ctx.clear_kv_cache();
                    self.cached.clear();
                    error(e)
                }
            },
        }
    }

    fn complete(
        &mut self,
        system: &str,
        user: &str,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<LocalLlmReply, String> {
        let messages = [("system", system), ("user", user)]
            .into_iter()
            .map(|(role, text)| LlamaChatMessage::new(role.into(), text.into()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("bad prompt: {e}"))?;
        let mut prompt = self
            .model
            .apply_chat_template(&self.template, &messages, true)
            .map_err(|e| format!("chat template: {e}"))?;
        if self.hybrid_thinking {
            // What the model's own template adds for `enable_thinking=false`:
            // an empty thought, so it answers straight away.
            prompt.push_str(NO_THINKING);
        }
        let vocab = self.model.vocab();
        // llama.cpp's built-in templates leave out the start-of-text token;
        // models that expect one (Gemma, LFM2) go off the rails without it.
        let mut tokens = vocab.tokenize(prompt.as_bytes(), false, true);
        if vocab.should_add_bos() && tokens.first() != Some(&vocab.bos()) {
            tokens.insert(0, vocab.bos());
        }

        let n_ctx = self.ctx.n_ctx() as usize;
        let room = n_ctx.saturating_sub(tokens.len());
        if room < 32 {
            return Ok(LocalLlmReply::TooLong);
        }
        let max_new = (max_tokens as usize).clamp(16, room);

        // Reuse the cached start of the prompt (usually the whole system
        // prompt) and evaluate only the rest. At least the last token is
        // evaluated again, because its logits start the reply.
        let mut keep = common_prefix(&self.cached, &tokens).min(tokens.len() - 1);
        let trimmed = self
            .ctx
            .clear_kv_cache_seq(Some(0), Some(keep as u32), None)
            .map_err(|e| format!("cache: {e}"))?;
        if !trimmed {
            // Models with recurrent layers (LFM2) keep one running state
            // that can't be cut back to a prefix: start from scratch.
            self.ctx.clear_kv_cache();
            keep = 0;
        }
        self.cached.truncate(keep);

        let mut batch = LlamaBatch::new(N_BATCH as usize, 1);
        let new = &tokens[keep..];
        for (chunk_index, chunk) in new.chunks(N_BATCH as usize).enumerate() {
            batch.clear();
            let start = keep + chunk_index * N_BATCH as usize;
            for (i, &t) in chunk.iter().enumerate() {
                let pos = start + i;
                let last = pos == tokens.len() - 1;
                batch
                    .add(t, pos as i32, &[0], last)
                    .map_err(|e| format!("batch: {e}"))?;
            }
            self.ctx
                .decode(&mut batch)
                .map_err(|e| format!("prompt: {e}"))?;
            self.cached.extend_from_slice(chunk);
        }

        let mut sampler = if temperature <= 0.0 {
            LlamaSampler::greedy()
        } else {
            LlamaSampler::chain_simple([
                LlamaSampler::top_k(40),
                LlamaSampler::top_p(0.95, 1),
                LlamaSampler::temp(temperature),
                LlamaSampler::dist(SEED),
            ])
        };

        let mut out = Vec::new();
        for pos in (tokens.len()..).take(max_new) {
            // `sample` also accepts the token into the sampler's history.
            let token = sampler.sample(&self.ctx, batch.n_tokens() - 1);
            if vocab.is_eog(token) {
                break;
            }
            out.extend(vocab.token_to_piece(token, false, None));
            if pos + 1 >= n_ctx {
                break;
            }
            batch.clear();
            batch
                .add(token, pos as i32, &[0], true)
                .map_err(|e| format!("batch: {e}"))?;
            self.ctx
                .decode(&mut batch)
                .map_err(|e| format!("generation: {e}"))?;
            self.cached.push(token);
        }
        Ok(LocalLlmReply::Ok {
            text: String::from_utf8_lossy(&out).trim().to_string(),
        })
    }
}

fn common_prefix(a: &[LlamaToken], b: &[LlamaToken]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// CPU threads for llama.cpp: the performance cores where we can tell (the
/// efficiency cores slow it down), otherwise half the logical CPUs, which
/// is the physical core count on most PCs.
fn threads() -> i32 {
    #[cfg(target_os = "macos")]
    if let Some(n) = std::process::Command::new("sysctl")
        .args(["-n", "hw.perflevel0.physicalcpu"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<i32>().ok())
        .filter(|&n| n > 0)
    {
        return n;
    }
    let logical = std::thread::available_parallelism().map_or(4, |n| n.get() as i32);
    (logical / 2).max(1)
}
