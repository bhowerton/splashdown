//! `splashdown-llm`: runs Splashdown's built-in local text model (a GGUF
//! file, via llama.cpp) for the app, in its own process. It reads
//! `LocalLlmRequest`s as JSON lines on stdin and answers each with one
//! `LocalLlmReply` line on stdout (see `splashdown_core::local_llm`).
//!
//! It keeps the model and its processed prompts between requests: each
//! long, unchanging system prompt (cleanup, commands, actions) is evaluated
//! once and kept in a slot of its own, so later requests only pay for the
//! new text. That's what makes CPU-only machines usable. After a
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
/// Prompts kept at once (cleanup, commands, planning actions), sharing the
/// context's `N_CTX` tokens, so keeping several costs no extra memory.
const SLOTS: usize = 3;
/// Fewest tokens in common with a slot's prompt for a request to reuse
/// that slot rather than the least recently used one.
const MIN_SHARED: usize = 64;
/// Tokens evaluated per batch while reading a prompt.
const N_BATCH: u32 = 512;
/// Same seed every time, so a given input cleans up the same way.
const SEED: u32 = 1234;
/// Starts a hybrid model's reply with an empty thought (Qwen3's template
/// does the same when thinking is off).
const NO_THINKING: &str = "<think>\n\n</think>\n\n";

/// Most tokens of a text an embedding reads; the app sends shorter pieces.
const EMBED_TOKENS: usize = 512;
/// Tokens per embedding batch, and so the most in one text.
const EMBED_BATCH: u32 = 2048;
/// Most texts in one embedding batch.
const EMBED_SEQS: u32 = 16;

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
        // Idle is counted from the last answer, not the last request: a
        // long one mustn't be taken for idleness part way.
        LAST_REQUEST.store(started.elapsed().as_secs(), Ordering::Relaxed);
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
        // A chat session for completions, an embedder for vectors: made on
        // first use, since a model is one or the other.
        let mut session = None;
        let mut embedder = None;

        let mut request = request;
        loop {
            let reply = match &request {
                LocalLlmRequest::Embed { texts, .. } => {
                    match ready(&mut embedder, || Embedder::new(&backend, &model)) {
                        Ok(e) => e.handle(texts),
                        Err(e) => error(e),
                    }
                }
                _ => match ready(&mut session, || Session::new(&backend, &model)) {
                    Ok(s) => s.handle(&request),
                    Err(e) => error(e),
                },
            };
            send(reply);
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

/// What `slot` holds, made by `make` the first time.
fn ready<T>(
    slot: &mut Option<T>,
    make: impl FnOnce() -> Result<T, String>,
) -> Result<&mut T, String> {
    if slot.is_none() {
        *slot = Some(make()?);
    }
    Ok(slot.as_mut().expect("just made"))
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

/// A loaded model with a context whose cache holds the last few prompts,
/// one per sequence (slot), sharing one pool of `N_CTX` tokens.
struct Session<'m> {
    model: &'m LlamaModel,
    template: LlamaChatTemplate,
    /// The model thinks first unless its reply starts with an empty thought.
    hybrid_thinking: bool,
    ctx: LlamaContext<'m>,
    slots: [Slot; SLOTS],
    /// Counts requests, to find the slot used least recently.
    clock: u64,
}

/// One sequence in the context's cache.
#[derive(Default)]
struct Slot {
    /// The tokens it holds, in order.
    cached: Vec<LlamaToken>,
    /// `Session::clock` when it was last used.
    used: u64,
}

impl<'m> Session<'m> {
    /// The slot used least recently (an empty one first).
    fn least_used(&self) -> usize {
        (0..SLOTS)
            .min_by_key(|&i| (!self.slots[i].cached.is_empty(), self.slots[i].used))
            .expect("there are slots")
    }

    /// The slot, other than `keep`, to empty first when room is needed.
    fn evictable(&self, keep: usize) -> Option<usize> {
        (0..SLOTS)
            .filter(|&i| i != keep && !self.slots[i].cached.is_empty())
            .min_by_key(|&i| self.slots[i].used)
    }

    /// Cells of the shared pool not held by any slot.
    fn free_cells(&self, n_ctx: usize) -> usize {
        let held: usize = self.slots.iter().map(|s| s.cached.len()).sum();
        n_ctx.saturating_sub(held)
    }

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
            .with_n_seq_max(SLOTS as u32)
            // One pool for all slots, rather than N_CTX / SLOTS each.
            .with_kv_unified(true)
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
            slots: Default::default(),
            clock: 0,
        })
    }

    fn handle(&mut self, request: &LocalLlmRequest) -> LocalLlmReply {
        match request {
            LocalLlmRequest::Load { .. } | LocalLlmRequest::Embed { .. } => LocalLlmReply::Ok {
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
                    self.slots = Default::default();
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

        // The slot whose prompt starts the same way (usually the same
        // system prompt), else the one used least recently.
        self.clock += 1;
        let (seq, prefix) = self
            .slots
            .iter()
            .enumerate()
            .map(|(i, slot)| (i, common_prefix(&slot.cached, &tokens)))
            .max_by_key(|&(i, prefix)| (prefix, std::cmp::Reverse(self.slots[i].used)))
            .expect("there are slots");
        // A few shared tokens (the template's opening) aren't worth a
        // slot holding another prompt.
        let (seq, prefix) = if prefix < MIN_SHARED {
            (self.least_used(), 0)
        } else {
            (seq, prefix)
        };
        self.slots[seq].used = self.clock;

        // Reuse the cached start of the prompt and evaluate only the rest.
        // At least the last token is evaluated again, because its logits
        // start the reply.
        let mut keep = prefix.min(tokens.len() - 1);
        let trimmed = self
            .ctx
            .clear_kv_cache_seq(Some(seq as u32), Some(keep as u32), None)
            .map_err(|e| format!("cache: {e}"))?;
        if !trimmed {
            // Models with recurrent layers (LFM2) keep one running state
            // that can't be cut back to a prefix: start from scratch.
            self.ctx.clear_kv_cache();
            self.slots = Default::default();
            self.slots[seq].used = self.clock;
            keep = 0;
        }
        self.slots[seq].cached.truncate(keep);
        // Room for the rest of the prompt and the reply: the other slots
        // give theirs up, least recently used first.
        let needed = tokens.len() - keep + max_new;
        while self.free_cells(n_ctx) < needed {
            let Some(other) = self.evictable(seq) else {
                break;
            };
            self.ctx
                .clear_kv_cache_seq(Some(other as u32), None, None)
                .map_err(|e| format!("cache: {e}"))?;
            self.slots[other] = Slot::default();
        }

        let mut batch = LlamaBatch::new(N_BATCH as usize, 1);
        let new = &tokens[keep..];
        for (chunk_index, chunk) in new.chunks(N_BATCH as usize).enumerate() {
            batch.clear();
            let start = keep + chunk_index * N_BATCH as usize;
            for (i, &t) in chunk.iter().enumerate() {
                let pos = start + i;
                let last = pos == tokens.len() - 1;
                batch
                    .add(t, pos as i32, &[seq as i32], last)
                    .map_err(|e| format!("batch: {e}"))?;
            }
            self.ctx
                .decode(&mut batch)
                .map_err(|e| format!("prompt: {e}"))?;
            self.slots[seq].cached.extend_from_slice(chunk);
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
                .add(token, pos as i32, &[seq as i32], true)
                .map_err(|e| format!("batch: {e}"))?;
            self.ctx
                .decode(&mut batch)
                .map_err(|e| format!("generation: {e}"))?;
            self.slots[seq].cached.push(token);
        }
        // The reply won't start the next prompt: give its room back.
        let prompt_end = tokens.len();
        if self.slots[seq].cached.len() > prompt_end
            && self
                .ctx
                .clear_kv_cache_seq(Some(seq as u32), Some(prompt_end as u32), None)
                .unwrap_or(false)
        {
            self.slots[seq].cached.truncate(prompt_end);
        }
        Ok(LocalLlmReply::Ok {
            text: String::from_utf8_lossy(&out).trim().to_string(),
        })
    }
}

/// An embedding model with a context that pools each text into one
/// vector.
struct Embedder<'m> {
    model: &'m LlamaModel,
    ctx: LlamaContext<'m>,
}

impl<'m> Embedder<'m> {
    fn new(backend: &LlamaBackend, model: &'m LlamaModel) -> Result<Self, String> {
        let threads = threads();
        // A whole batch is one micro-batch: models that read in both
        // directions need all of a text at once.
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(EMBED_BATCH))
            .with_n_batch(EMBED_BATCH)
            .with_n_ubatch(EMBED_BATCH)
            .with_n_seq_max(EMBED_SEQS)
            .with_embeddings(true)
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
        let ctx = model
            .new_context(backend, params)
            .map_err(|e| format!("couldn't create a context: {e}"))?;
        Ok(Self { model, ctx })
    }

    fn handle(&mut self, texts: &[String]) -> LocalLlmReply {
        match self.embed(texts) {
            Ok(vectors) => LocalLlmReply::Vectors { vectors },
            Err(e) => {
                self.ctx.clear_kv_cache();
                error(e)
            }
        }
    }

    /// Each text's vector, of unit length. Texts longer than
    /// `EMBED_TOKENS` are cut short; several share each batch.
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let vocab = self.model.vocab();
        let tokenized: Vec<Vec<LlamaToken>> = texts
            .iter()
            .map(|text| {
                let mut tokens = vocab.tokenize(text.as_bytes(), true, true);
                tokens.truncate(EMBED_TOKENS);
                if tokens.is_empty() {
                    tokens.push(vocab.bos());
                }
                tokens
            })
            .collect();
        let mut vectors = Vec::with_capacity(texts.len());
        let mut batch = LlamaBatch::new(EMBED_BATCH as usize, EMBED_SEQS as i32);
        let mut start = 0;
        while start < tokenized.len() {
            // As many texts as fit.
            let mut end = start;
            let mut used = 0;
            while end < tokenized.len()
                && end - start < EMBED_SEQS as usize
                && used + tokenized[end].len() <= EMBED_BATCH as usize
            {
                used += tokenized[end].len();
                end += 1;
            }
            batch.clear();
            for (seq, tokens) in tokenized[start..end].iter().enumerate() {
                batch
                    .add_sequence(tokens, seq as i32, true)
                    .map_err(|e| format!("batch: {e}"))?;
            }
            self.ctx.clear_kv_cache();
            self.ctx
                .decode(&mut batch)
                .map_err(|e| format!("embedding: {e}"))?;
            for seq in 0..end - start {
                let v = self
                    .ctx
                    .embeddings_seq_ith(seq as i32)
                    .map_err(|e| format!("embedding: {e}"))?;
                let norm = v
                    .iter()
                    .map(|x| x * x)
                    .sum::<f32>()
                    .sqrt()
                    .max(f32::EPSILON);
                vectors.push(v.iter().map(|x| x / norm).collect());
            }
            start = end;
        }
        Ok(vectors)
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
