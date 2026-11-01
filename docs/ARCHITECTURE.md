# Architecture

How Splashdown fits together, for people changing it: where each part of the app lives in the code and why it's arranged that way.

## The big picture

```
 key tap ──► gesture recognizer ──► session ──► audio ──► pipeline ──► insert text
 (OS hook)    (splashdown-core)            (app)       (splashdown-audio) (splashdown-core +   (splashdown-platform)
                                                            splashdown-providers)
```

Hold the dictation key and the session starts the microphone and captures the focused field. Release it and the recording goes through the pipeline: speech-to-text, snippets, an optional AI pass, then the sanity guard. A spoken command (the instruction key tapped before anything is said) goes to the AI instead, which names an action for the session to carry out (`splashdown_core::command`). The result is pasted or typed into the field it came from.

## Crates

| Crate | Owns | Depends on |
|---|---|---|
| `splashdown-core` | The platform-agnostic heart: provider traits (`SpeechToText`, `TextModel`), the `Pipeline`, prompts, the sanity guard, spoken commands (interpreted by the AI) and snippets, the hotkey `gesture` state machine, vocabulary learning | nothing internal |
| `splashdown-audio` | Microphone capture (cpal), device choice, resampling to 16 kHz, silence trimming, WAV encoding, and reading audio and video files in pieces for transcription (Symphonia) | `splashdown-core` |
| `splashdown-config` | `config.toml` (`Settings`), resolving "Automatic" to concrete providers, presets, API keys in the OS keychain, on-disk paths | `splashdown-core` |
| `splashdown-providers` | Implementations of the provider traits: OpenAI-compatible chat and transcription, OpenRouter audio, local speech-to-text with transcribe.cpp (feature `whisper-local`), Apple's models (via `splashdown-apple-shim`), and the `factory` that builds them from config | `splashdown-core`, `splashdown-audio`, `splashdown-config`, `splashdown-apple-shim` |
| `splashdown-platform` | Everything OS-specific: the key hook, focus/selection/URL via accessibility, clipboard and synthetic keys, the overlay, permissions, sounds. One API in `lib.rs`; `mac/` and `win/` implement it | `splashdown-core` |
| `splashdown-apple-shim` | macOS only: a Swift static library (`swift/`) exposing SpeechAnalyzer and Foundation Models through a C ABI, and the Rust side of that bridge | `splashdown-core`, `splashdown-audio` |
| `splashdown-llm` | The helper process that runs the built-in local text model with llama.cpp, so its copy of ggml never meets transcribe.cpp's. JSON lines over stdin/stdout; keeps the processed prompt between requests | `splashdown-core` |
| `splashdown-control` | The running app's control channel (see below): the JSON-line protocol, a client, and the socket/pipe server with its permission checks | `splashdown-config` |
| `splashdown-cli` | The `splashdown-cli` developer tool: record, clean up, and the evals in `eval/`; `splashdown-cli toggle`/`listen` and friends talk to the running app | `splashdown-core`, `splashdown-audio`, `splashdown-config`, `splashdown-control`, `splashdown-providers` |
| `splashdown-mcp` | An MCP server over stdio with one tool, `ask_user_by_voice`, for coding agents ([AGENTS.md](AGENTS.md)) | `splashdown-control` |
| `app/src-tauri` | The menu bar app: the session, Settings commands, history, tray menu, and the bridge to the UI thread | everything except `splashdown-cli` and `splashdown-mcp` |
| `app/ui` | Settings and onboarding windows: plain HTML, CSS and JavaScript, no build step | — |

Rules of thumb:
- **`splashdown-core` never touches the OS.** It takes time as a parameter and talks to the world through traits, which is why most of its logic is unit-tested.
- **OS code lives only in `splashdown-platform`** (and `splashdown-apple-shim`). Adding Windows means filling in `crates/splashdown-platform/src/win/mod.rs`, which already has a stub for every function.
- **Decisions about settings live in `splashdown-config`** (which provider "Automatic" means, where words go for the Privacy tab), not in the app or the UI.

## Threads

The app is a handful of long-lived threads that talk over channels. Only the session thread makes decisions.

| Thread | Code | Job |
|---|---|---|
| Main | Tauri, `app/src-tauri/src/ui.rs` | Windows, tray, overlay. AppKit objects are used only here; other threads post work with `run_on_main_thread` |
| `splashdown-keys` | `splashdown-platform/src/mac/keys.rs` | A CGEventTap on its own run loop. Every key event (and the mouse's side buttons) goes to the gesture `Recognizer`, which decides synchronously whether to swallow it |
| `splashdown-control` | `splashdown-control/src/server.rs` | Accepts control-socket connections, each handled on a short-lived thread that waits for the session's answer |
| `splashdown-session` | `app/src-tauri/src/session.rs` | The controller: receives `Msg`s (gesture actions, new settings, redo requests), drives audio, runs the pipeline, inserts text, records history |
| Audio | `app/src-tauri/src/audio.rs` | Owns the cpal stream (it can't move between threads) and serves start/take/stop |
| Preview | `app/src-tauri/src/preview.rs` | Re-transcribes the recording so far with a local model for the live words in the overlay; with a streaming model the session's `LiveStream` gives them instead |
| `transcribe-file` | `app/src-tauri/src/notes/files.rs` | Transcribes audio and video files dropped on Notes into notes, one at a time |
| `meeting`, `meeting-audio` | `app/src-tauri/src/meeting.rs`, `splashdown-audio/src/live.rs` | A meeting: takes both sides' audio twice a second (the microphone on its own recorder thread, the computer's sound from the Swift shim's Core Audio tap or WASAPI loopback), spools it, cuts each side into turns and writes them into the note |
| `splashdown-llm` (separate process) | `crates/splashdown-llm` | Runs the built-in local text model when it's in use; started on demand by `splashdown-providers/src/local_llm.rs`, exits after 10 idle minutes |
| `splashdown-updates` | `app/src-tauri/src/updates.rs` | Release builds only: checks for a new version (tauri-plugin-updater, GitHub releases) 30 s after launch and then daily, if allowed in Settings. Installing happens only from the user's click |
| Tauri async runtime | `app/src-tauri/src/commands.rs` | Settings commands. Slow work (building providers, downloads) runs here or in `spawn_blocking`, never on the main thread |

**The recognizer is shared** (`Arc<Mutex<Recognizer>>`) between the key thread and the session thread. The key thread must answer "swallow or not" immediately, so state that has to be read at key time lives in the recognizer, not the session. Auto-send is the example: while the session is busy processing a dictation, a Return press toggles a flag in the recognizer, and the session reads it with `take_send()` just before inserting.

**The session thread blocks while processing.** It runs the pipeline with `block_on`, so during a network call it doesn't handle other messages; they queue and run afterward. That keeps the controller simple: one dictation at a time, in order. Anything the user must see during processing (the auto-send badge) is updated from the key thread directly through the cloneable `Ui` handle.

## A dictation, step by step

1. **Key down** → `Recognizer::on_event` returns `Action::Start` → the session starts the microphone first (so the first word isn't lost), then captures the focused field (`Focus::capture`), its selection, and matches a preset by app or URL.
2. **While holding**: number keys switch preset, Return toggles auto-send, the instruction key marks where the instruction starts, other keys mean the user was typing a shortcut, so the recording is silently canceled.
3. **Release** → `Action::Finish` → the session stops the microphone and decides the mode: Dictate, Dictate + instruction, Transform (text was selected) or Raw (preset has the AI off).
4. **Pipeline** (`splashdown-core/src/pipeline.rs`): speech-to-text → spoken punctuation and commands → snippet placeholders → the AI pass (instructions may go to a separate cloud model) → the sanity guard, which falls back to the raw transcript if the output looks wrong.
5. **Insert** (`splashdown-platform`): only if the same field still has focus; otherwise the text goes to the clipboard. Paste saves and restores the clipboard; Type sends key events.
6. **Afterward**: history entry, auto-send Return if requested, and a check ~20 s later for words the user retyped (vocabulary suggestions).

## Other triggers: the control socket and links

Besides the dictation key, the session takes `Msg::Control` requests (`app/src-tauri/src/session/control.rs`). They come from:
- **The control socket** (`crates/splashdown-control`): a Unix socket at `Paths::control_socket` (`~/Library/Application Support/Splashdown/control.sock`), or the named pipe `\\.\pipe\splashdown` on Windows. One JSON line each way per connection: `{"cmd":"toggle"|"start"|"stop"|"cancel"}` or `{"cmd":"listen","prompt":"…"}`, answered with `{"ok":true}`, `{"ok":true,"text":"…"}` or `{"ok":false,"error":"…"}`. The `splashdown-cli` CLI and `splashdown-mcp` are its clients.
- **`splashdown://` links** (`app/src-tauri/src/triggers.rs`): the scheme is registered in `app/src-tauri/Info.plist`, and macOS delivers opened links as Tauri's `RunEvent::Opened`. Links can toggle, start, stop and cancel, not listen.

Security: the socket is created under a temporary name, set to mode 0600, then renamed into place, and every connection's peer is checked with `getpeereid` to be the same user (others get `permission denied`). A live socket from another running copy is never replaced; a stale one is. The Windows pipe refuses remote clients and keeps the default DACL, under which only this user, SYSTEM and administrators can write. Requests are size-limited and must arrive within 5 seconds.

Control requests go through the same `Recognizer` as the key: `start_hands_free` latches a hands-free recording (so a tap of the dictation key finishes it and Esc cancels it), and `finish`/`cancel` end one. A `listen` recording drops the focused field and selection, uses the pinned or default preset, shows the question in the overlay (`OverlayState::Question`), and when it finishes, `answer` sends the text back to the requester instead of inserting it. If the recording is discarded, its reply channel is dropped and the requester hears "Canceled".

## Providers and "Automatic"

`splashdown-config::Settings::resolve_speech_with` / `resolve_ai_with` turn the saved settings plus available keys and on-device models into a concrete `SpeechChoice` / `AiChoice`. `splashdown-providers::factory` turns those into trait objects. The order for "Automatic" is: a saved key (the user chose that provider) → Apple's on-device models → local speech-to-text with no AI pass. Local-only mode is enforced here, in one place.

## The Apple shim

`crates/splashdown-apple-shim/swift` is a Swift package built by `build.rs` with `swift build` and linked statically. Each async entry point takes a C callback and a context pointer, and calls back exactly once; `src/ffi.rs` turns that into a Rust future with a one-shot channel. `FoundationModels` exists only on macOS 26+, so it's weak-linked (see `.cargo/config.toml`), and every Swift entry point checks `#available` first. On older systems the providers report "unavailable" instead of crashing.

## Prompts and evals

Prompts are in `splashdown-core/src/prompt.rs`. Dictated and selected text always goes in tagged blocks, and only the `<instruction>` block is ever obeyed. Any prompt change should be checked with `splashdown-cli eval cleanup` against `eval/cleanup.toml` (see [eval/README.md](../eval/README.md)), which also covers prompt injection.

## The UI

`app/ui` is served by Tauri from disk, with no bundler. The pages call Rust through `window.__TAURI__.core.invoke` (the commands in `commands.rs`) and render a `Snapshot` of the current state. Opened directly in a browser, each page falls back to sample data so it can be styled without the app.
