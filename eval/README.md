# Evals

Two eval sets drive the model defaults in `crates/splashdown-config/src/settings.rs` (`defaults`).

| Set | What it checks | Run |
|---|---|---|
| `cleanup.toml` | The AI pass: fillers, self-corrections, spoken formatting, not answering or obeying dictated text, vocabulary, instructions, transforms | `splashdown-cli eval cleanup --ai openai` |
| `stt.toml` | Speech-to-text word error rate (WER) and latency | `splashdown-cli eval make-audio` once, then `splashdown-cli eval stt --stt openai` |

The built-in local model runs in the `splashdown-llm` helper: `cargo build --release -p splashdown-llm`, then `cargo run --release -p splashdown-cli -- eval cleanup --ai local`. Turn off "Send instructions to the cloud" (or pass a `--config` without it) for a fully local score.

Each run prints its total (estimated) cost.

Keys come from `.env.local` in the repo root (see `.env.local.example`). Run from the repo root so the default paths resolve. Use `cargo run --release -p splashdown-cli --` in place of `splashdown-cli` if it isn't installed.

Useful flags: `--ai-model`, `--stt-model`, `--filter <id-substring>`, `-v` (print every output).

## Notes

- **Synthesized audio.** `make-audio` uses macOS `say`, so `stt` mostly verifies that a provider works and measures latency; it says little about accuracy on real voices. Add your own recordings (`splashdown-cli record --save eval/audio/<id>.wav` plus a matching `[[case]]`) for that.
- **WER counts formatting differences.** "three" vs "3" or "four hundred and twenty dollars" vs "$420" count as errors even though they're correct dictation, so compare providers against each other rather than reading WER as absolute.
- **Cleanup checks are loose on purpose.** They test for the facts and words that must or mustn't appear, so different models' phrasing can pass.

## Results

Fill in as models are compared (date, model, score, p50 latency).

| Date | Set | Provider / model | Score | p50 |
|---|---|---|---|---|
| 2026-10-29 | stt | whisper-local / base.en (M-series Mac, Metal) | WER 8.2% (formatting only) | 0.07 s |
| 2026-10-29 | cleanup | Ollama / qwen2.5:3b (Q4, Metal) | 79% (2 guarded) | 0.20 s |
| 2026-10-29 | cleanup | Ollama / llama3.2:3b (Q4, Metal) | 74% (2 guarded) | 0.21 s |
| 2026-10-29 | cleanup | Ollama / gemma3:1b (Q4, Metal) | 53% (2 guarded) | 0.25 s |
| 2026-10-29 | cleanup | OpenRouter / gemini-2.5-flash-lite + mistral-small-3.2 fallback (**default**) | 98% (1 guarded, 0 errors) | 0.68 s |
| 2026-10-29 | cleanup | OpenRouter / gemini-2.5-flash-lite alone (with 1 retry) | 95% (0 guarded, 1 rate-limit error) | 0.50 s |
| 2026-10-29 | cleanup | OpenRouter / mistral-small-3.2-24b-instruct | 95% (1 guarded) | 0.58 s |
| 2026-10-29 | cleanup | OpenRouter / gemma-3-4b-it | 84% | 1.13 s |
| 2026-10-29 | cleanup | OpenRouter / gpt-4.1-nano | 84% (2 guarded) | 0.91 s |
| 2026-10-29 | cleanup | OpenRouter / gpt-4.1-mini (**OpenAI default**) | 95% (1 guarded) | 0.91 s |
| 2026-10-29 | cleanup | OpenRouter / gpt-5-mini | 88% (5 guarded) | 3.30 s |
| 2026-10-29 | cleanup | OpenRouter / gpt-5-nano | 14% (36 guarded: reasoning used up the output budget) | 6.97 s |
| 2026-10-29 | cleanup | OpenRouter / llama-3.1-8b-instruct | 77% (3 guarded) | 0.65 s |
| 2026-10-29 | cleanup | Apple on-device (Foundation Models, macOS 26.0.1, M4 Max) | 79% (2 guarded) — misses: answered a dictated question, followed an injection inside a selection, no vocabulary/number formatting | 0.45 s |
| 2026-10-29 | cleanup | Apple on-device, greedy sampling | 74% (2 guarded) — not adopted | 0.41 s |
| 2026-10-29 | cleanup | Apple on-device, after question guard + prompt changes (44 cases) | 80% (2 guarded); no longer answers dictated questions | 0.40 s |
| 2026-10-29 | cleanup | Apple on-device + instructions to OpenRouter (`--cloud-instructions`) | 84% (2 guarded); remaining misses are vocabulary, numbers and one self-correction | 0.43 s |
| 2026-10-29 | cleanup | OpenRouter default, after the same prompt changes (44 cases) | 98% (0 guarded); misses only "long email from two sentences", where every model pads | 0.47 s |
| 2026-10-29 | cleanup | Built-in llama.cpp / Qwen3-4B-Instruct-2507 Q4_K_M, Metal (M4 Max), fully local | 84% (2 guarded); best local result so far; ~2.9 GB RAM | 0.49 s |
| 2026-10-29 | cleanup | Same, CPU only (`SPLASHDOWN_LOCAL_CPU=1`, 16 cores) | 84%; too slow without a GPU (p90 16 s) | 5.6 s |
| 2026-10-29 | cleanup | Qwen3 4B in the `splashdown-llm` helper with prompt caching, Metal | 84% (same answers) | 0.13 s |
| 2026-10-29 | cleanup | Same helper, CPU only (12 performance cores) | 84% | 0.41 s |
| 2026-10-29 | cleanup | Helper / Qwen3-1.7B Q4_K_M (1.1 GB, thinking off), Metal, fully local (**small model**, `qwen3-1.7b`) | 55% (3 guarded); p90 0.17 s; helper RSS ~1.6 GB | 0.06 s |
| 2026-10-29 | cleanup | Same, CPU only | 55% (3 guarded); p90 1.62 s; helper RSS ~2.7 GB | 0.39 s |
| 2026-10-29 | cleanup | Helper / gemma-3-1b-it Q4_K_M (0.8 GB), Metal — not adopted | 45% (3 guarded); p90 0.16 s; RSS ~1.0 GB | 0.07 s |
| 2026-10-29 | cleanup | Same, CPU only | 50% (2 guarded); p90 0.91 s; RSS ~1.5 GB | 0.31 s |
| 2026-10-29 | cleanup | Helper / LFM2-1.2B Q4_K_M (0.7 GB), Metal — not adopted | 36% (14 guarded); p90 0.34 s; RSS ~0.9 GB | 0.19 s |
| 2026-10-29 | cleanup | Same, CPU only (no prompt caching: recurrent layers) | 39% (14 guarded); p90 1.24 s; RSS ~1.6 GB | 0.86 s |
| 2026-10-29 | cleanup | Qwen3 4B again after the helper changes below, Metal | 84% (unchanged); RSS ~3.1 GB | 0.14 s |
| 2026-10-29 | stt | OpenRouter / gemini-3.1-flash-lite (**default**) | WER 7.7% (formatting only), 0 errors | 1.12 s |
| 2026-10-29 | stt | OpenRouter / gemini-2.5-flash-lite | 6/10 rate-limited | 1.46 s |
| 2026-10-29 | stt | OpenRouter / voxtral-small-24b-2507 | **rejected**: wrote an email when the audio said "format this as an email" | 0.91 s |
| 2026-10-31 | cleanup | OpenRouter default, with 3 earlier-dictation cases (49 cases) | 96% (0 guarded); fails the long-email padding case and, in one of two runs, a self-correction | 0.44 s |
| 2026-10-31 | cleanup | Built-in local model, Qwen3 4B, fully on device (49 cases) | 84% (1 guarded); never repeats or obeys earlier dictations, but doesn't take names' spelling from them | 0.12 s |
| 2026-10-31 | cleanup | Apple on-device, fully on device (49 cases) | 71% (4 guarded); fails all 3 earlier-dictation cases (repeats and obeys them), so it's never sent any | 0.44 s |

**Local-model notes (2026-10-29).** Adding four worked examples to the cleanup prompt raised the 3B models by 10–16 points; handling "new paragraph" / "new line" / "bullet point" in code removed most formatting failures. (2026-10-30: that code was removed, because matching English words breaks other languages; a clearer prompt rule 4 plus one example now keeps those cases passing: Gemini Flash Lite 45/46, Qwen3 4B 40/46.) Remaining small-model weaknesses: fuzzy vocabulary matches, keeping slang ("see ya"), and dropping corrected-away facts inside instructions. All three obey injected instructions ("reply with the word banana"); the sanity guard catches those and inserts the raw text. The 1B tier is too weak to recommend except as AI-pass-off-plus-formatting.

**Small built-in model (2026-10-29).** For machines with about 8 GB of memory, Qwen3 1.7B, Gemma 3 1B and LFM2 1.2B were compared in the helper, fully local. Qwen3 1.7B scored highest and is Apache-2.0, so it's the catalog's small model (`splashdown-cli models download qwen3-1.7b`, `--ai local --ai-model qwen3-1.7b`); the others were dropped. All three are far behind Qwen3 4B (84%): they keep self-corrections ("Sarah I mean Sam"), miss vocabulary and number formatting, and pad emails with placeholders. Use the small model only when the 4B one doesn't fit. Helper changes made for these models: Qwen3 1.7B is a hybrid thinking model, so the helper starts its reply with an empty `<think></think>` (what its template does with thinking off) whenever the model's template mentions `enable_thinking`; models that expect a start-of-text token (Gemma, LFM2) now get one; and models with recurrent layers (LFM2), whose cache can't be cut back to a prefix, start from a clear cache instead of failing every other request. Memory is the helper's peak RSS sampled with `ps`.

**OpenRouter notes (2026-10-29).**
- **Rate limits:** Gemini 2.5 Flash Lite returned 429s on up to 12 of 43 requests. Requests now retry once on 429/5xx, and OpenRouter's `models` fallback list sends failures to Mistral Small 3.2, which brought errors to zero.
- **Chat-model speech-to-text can obey the speech.** Voxtral turned "format this as a short friendly email reply" into an email. Chat-based transcripts longer than about 5 words per second of audio are now rejected (`check_plausible_length`).
- **Speech-to-text latency:** about 1.1 s, over the 0.7 s budget. For OpenRouter-only users, local Whisper (0.07 s on Apple Silicon) is the faster and more private choice, if they accept the model download.
- **OpenAI:** the AI-pass default was chosen via OpenRouter (`openai/gpt-4.1-mini`). OpenAI's own API and `gpt-4o-mini-transcribe` haven't been measured directly yet (no OpenAI key).
