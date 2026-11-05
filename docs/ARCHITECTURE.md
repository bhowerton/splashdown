# Architecture

How Splashdown fits together, for people changing it: where each part of the app lives in the code and why it's arranged that way. For setting up, the checks a change must pass, and step-by-step recipes (adding a setting, a command, a voice action, an eval case), see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Start here

A suggested reading order for your first look at the code:

1. `app/src-tauri/src/main.rs`: what starts at launch and every command the windows can call; then `state.rs`, the shared `AppState`.
2. `app/src-tauri/src/session.rs`: the dictation controller. Read `Controller::run` and `handle` there, then `start` (`session/recording.rs`), `finish` and `deliver` (`session/deliver.rs`). Its opening comment maps the files in `session/`.
3. `crates/splashdown-core/src/pipeline/`: speech-to-text (`audio.rs`), the AI pass and the sanity guard (`text.rs`).
4. `crates/splashdown-core/src/prompt.rs` and `guard.rs`: what the AI is asked, and when its answer is thrown away.
5. `crates/splashdown-core/src/gesture.rs`: how key presses become actions.
6. `crates/splashdown-platform/src/lib.rs`: the one OS API (focus, insertion, keys); `mac/` and `win/` implement it.
7. `crates/splashdown-config/src/settings.rs`: `config.toml`, one file per section in `settings/`; `settings/resolve.rs` is how "Automatic" picks providers.
8. `app/ui/common.js`, then `app/ui/settings/main.js`: how the windows talk to Rust.

Every Rust file starts with a `//!` comment saying what it owns, so opening a file and reading its first lines is a quick way to find your way around.

## The big picture

```
 key tap ──► gesture recognizer ──► session ──► audio ──► pipeline ──► insert text
 (OS hook)    (splashdown-core)     (app)       (splashdown-audio) (splashdown-core +   (splashdown-platform)
                                                            splashdown-providers)
```

Hold the dictation key and the session starts the microphone and captures the focused field. Release it and the recording goes through the pipeline: speech-to-text, snippets, an optional AI pass, then the sanity guard. The result is pasted or typed into the field it came from.

A spoken command (the instruction key tapped before anything is said) goes to the AI instead, which names an action for the session to carry out (`splashdown_core::command`). That action may be an edit ("delete the last sentence"), a question for Ask, a draft for Compose, or reading text aloud. With voice actions on, it can also answer `do`: a second request plans that into one action from a fixed catalog (`splashdown_core::action`), whose parameters are checked before `voice_actions` carries it out off the session thread.

## Crates

| Crate | Owns | Depends on |
|---|---|---|
| `splashdown-core` | The platform-agnostic heart: provider traits (`SpeechToText`, `TextModel`), the `Pipeline`, the prompts, the sanity guard, spoken commands, voice action planning, Compose, Ask, snippets, the hotkey `gesture` state machine, vocabulary learning, meeting transcripts | nothing internal |
| `splashdown-audio` | Microphone capture (cpal), device choice, resampling to 16 kHz, finding speech (Silero through a `SpeechDetector` the providers crate sets, or loudness), silence trimming, noise reduction, WAV encoding, live audio for meetings, playing the natural voice, and reading audio and video files in pieces for transcription (Symphonia) | `splashdown-core` |
| `splashdown-config` | `config.toml` (`Settings`), resolving "Automatic" to concrete providers, presets, transforms (with the built-in ones' instructions), API keys in the OS keychain, on-disk paths (`Paths`) | `splashdown-core` |
| `splashdown-providers` | Implementations of the provider traits: OpenAI-compatible chat and transcription, OpenRouter audio, Anthropic, the built-in local text model (through `splashdown-llm`), local speech-to-text with transcribe.cpp (feature `local-speech`), Silero speech detection, the natural reading voice, Phonon (feature `natural-voice`), Apple's models (via `splashdown-apple-shim`), the catalog of downloadable models (`models.rs`), prices (`pricing.rs`), and the `factory` that builds providers from config | `splashdown-core`, `splashdown-audio`, `splashdown-config`, `splashdown-apple-shim` (macOS) |
| `splashdown-platform` | Everything OS-specific: the key hook, focus/selection/URL and the text around a field via accessibility, clipboard and synthetic keys, the overlay and mini bar, permissions, sounds, media keys and volume, which apps use the microphone. One API in `lib.rs`; `mac/` and `win/` implement it, and `win/unsupported.rs` stubs it for other systems so the shared crates still build there | `splashdown-core` |
| `splashdown-apple-shim` | macOS only: a Swift static library (`swift/`) exposing SpeechAnalyzer, Foundation Models, the calendar (EventKit), the system voices, the Core Audio tap that records the computer's sound in meetings, and screenshots, through a C ABI; plus the Rust side of that bridge | `splashdown-core`, `splashdown-audio` |
| `splashdown-llm` | The helper process that runs the built-in local text model with llama.cpp, so its copy of ggml never meets transcribe.cpp's. JSON lines over stdin/stdout; keeps processed prompts between requests | `splashdown-core` |
| `splashdown-control` | The running app's control channel (see below): the JSON-line protocol, a client, and the socket/pipe server with its permission checks | `splashdown-config` |
| `splashdown-cli` | The `splashdown-cli` developer tool: record, transcribe, clean up, try a command or a question, the evals in `eval/`, and the computer-use spike; `splashdown-cli toggle`/`listen`/`note` and friends talk to the running app | `splashdown-core`, `splashdown-audio`, `splashdown-config`, `splashdown-control`, `splashdown-providers`, `splashdown-platform`, `splashdown-apple-shim` (macOS) |
| `splashdown-mcp` | An MCP server over stdio for coding agents, with five tools (`ask_user_by_voice`, `list_notes`, `search_notes`, `read_note`, `append_note`) and two prompts; with arguments (`agents`, `install <agent>`, `uninstall`) it sets agents up with itself, through each agent's own command or install link (`setup.rs`). Ships inside the app ([AGENTS.md](AGENTS.md)) | `splashdown-control` |
| `app/src-tauri` | The menu bar (macOS) and tray (Windows) app: the session, the windows' commands, history, notes and meetings, and the bridge to the UI thread. See [The app](#the-app) | everything except `splashdown-cli`, `splashdown-mcp` and `splashdown-llm` (a separate process) |
| `app/ui` | The windows: plain HTML, CSS and JavaScript, no build step. See [The UI](#the-ui) | — |

Rules of thumb:
- **`splashdown-core` never touches the OS.** It takes time as a parameter and talks to the world through traits, which is why most of its logic is unit-tested.
- **OS code lives only in `splashdown-platform`** (and `splashdown-apple-shim`). A new OS feature needs a function in `lib.rs` and an implementation in both `mac/` and `win/` (and a stub in `win/unsupported.rs`).
- **Decisions about settings live in `splashdown-config`** (which provider "Automatic" means, where words go for the Privacy tab), not in the app or the UI.

## The app

`app/src-tauri/src`, grouped by job. Some big files will be split into folders; the jobs stay the same.

**Shell: starting up, windows and menus**
- `main.rs`: starts everything, creates `AppState`, registers every command.
- `tray.rs`: the menu bar or tray icon and its menu; opens Settings and the setup guide.
- `ui.rs`: the cloneable `Ui` handle other threads use to reach the main thread (overlay, sounds, tray status).
- `shortcuts.rs` and `actions.rs`: global shortcuts, and the actions they share with the menu.
- `triggers.rs`: the control socket and `splashdown://` links, and handing over to a copy that's already running.
- `minibar.rs`, `placement.rs`, `prompt.rs`, `help.rs`: the mini bar, where small panels go, the app's own questions panel, and the "What can I say?" window.
- `updates.rs`, `feedback.rs`, `errors.rs`: update checks, Send feedback, and turning failures into one short line.

**Settings**
- `state.rs`: `AppState`, and `AppState::update`, the one way to change settings outside the Settings window.
- `snapshot.rs`: what Settings and the setup guide show (`Snapshot`), with everything worked out from the settings.
- `commands.rs`: `get_state` / `save_settings` and the Settings window's other commands that no feature owns.
- `models.rs`: downloading models, with progress for every window.
- `engine.rs`: builds the `Engine` (providers, pipelines, and the settings the session needs) from settings; rebuilt whenever they change.
- `setup.rs` and `did.rs`: the setup guide's downloads, and what the user just did for its walk-through.
- `export.rs`: exporting history and settings, and importing them.

**The dictation session**
- `session.rs` and `session/`: the controller, one file per job (starting a recording, building the job, delivering the text, Ask and Compose, spoken commands, edits and undo, learning corrections, and requests from the control channel).
- `audio.rs`: the microphone thread.
- `preview.rs`, `chunks.rs`, `stream_typing.rs`: live words in the overlay, transcribing a long dictation in pieces while the user speaks, and typing the AI's text as it's written.
- `recent.rs`, `project.rs`, `replacements.rs`: context for the AI (recent dictations, a coding project's files) and snippet fill-ins and find-and-replace.
- `rescue.rs`: keeping a failed recording for "Retry last dictation".
- `history.rs` (with its commands), `insights.rs`, `spend.rs`: history, usage numbers, and costs with the spending limit.
- `learning.rs`: words learned from corrections, and forgetting them.

**Features beyond dictation**
- `compose.rs`: the Compose panel.
- `ask.rs`: the Ask panel.
- `reading.rs`: reading text aloud.
- `voice_actions.rs` and `my_actions.rs`: carrying out voice actions, and the user's own actions.

**Notes and meetings**
- `notes/`: the Notes window's commands (`mod.rs`, whose opening comment maps the rest), note files (`store.rs`) and their Markdown (`markdown.rs`), which folder (`folder.rs`), the window and pasting back (`window.rs`), dictating into a note (`capture.rs`), finding notes for Ask by words and by meaning (`search.rs`, `meaning.rs`), coding agents' access (`agents.rs`), transcribing files (`files.rs`) and links (`links.rs`, with the download helper in `link_helper.rs`), and serving recordings to the player (`media.rs`).
- `meeting.rs` and `meeting/`, `calls.rs`, `pill.rs`: recording a meeting into a note (the audio, the transcript, the summary, transcribing again, recovery after a crash), offering to when a call starts, and the meeting pill.

**Where state lives.** `AppState` (in `state.rs`, handed to Tauri with `app.manage`) holds what commands share: the settings, the session's `Handle`, why the engine can't run (if it can't), history, the tray menu and update status. Commands get it as `State<'_, AppState>`; other code with `app.state::<AppState>()`. The session thread owns its own state and the `Engine`; everything else reaches it only by sending it a message through `session::Handle`. What one feature owns lives in a static in its own module: the Compose and Ask panels (`PANEL`), the reading in progress (`reading.rs`), the meeting in progress (`meeting.rs`), spending (`spend.rs`), the last learned word (`learning.rs`). `config.toml` is the truth for settings: the Settings window saves through `save_settings`; anything else changes them with `AppState::update` (which saves, refreshes the menu bar and shortcuts, and tells an open Settings window), then calls `AppState::apply` when dictation needs the change, which rebuilds the engine and sends it to the session.

## Threads

The app is a handful of long-lived threads that talk over channels. Only the session thread makes decisions about a dictation.

| Thread | Code | Job |
|---|---|---|
| Main | Tauri, `app/src-tauri/src/ui.rs` | Windows, tray, overlay. AppKit objects are used only here; other threads post work with `run_on_main_thread` |
| `splashdown-keys` | `splashdown-platform/src/mac/keys.rs`, `win/keys.rs` | A CGEventTap on its own run loop (macOS) or a `WH_KEYBOARD_LL` hook with its own message loop (Windows). Every key event (and the mouse's side buttons on macOS) goes to the gesture `Recognizer`, which decides synchronously whether to swallow it |
| `splashdown-control` | `splashdown-control/src/server.rs` | Accepts control connections, each handled on a short-lived thread that waits for the session's answer |
| `splashdown-session` | `app/src-tauri/src/session.rs` and `session/` | The controller: receives `Msg`s (gesture actions, new settings, control requests, finished voice actions and drafts), drives audio, runs the pipeline, inserts text, records history |
| `splashdown-audio` | `app/src-tauri/src/audio.rs` | Owns the cpal stream (it can't move between threads) and serves start/take/stop |
| `splashdown-preview` | `app/src-tauri/src/preview.rs` | Re-transcribes the recording so far with a local model for the live words in the overlay; with a streaming model the session's `LiveStream` gives them instead |
| `splashdown-reading` | `app/src-tauri/src/reading.rs` | Reads text aloud, one piece at a time, so Esc or a new dictation can stop it |
| `transcribe-file` | `app/src-tauri/src/notes/files.rs` | Transcribes audio and video files dropped on Notes into notes, one at a time |
| `meeting`, `meeting-audio` | `app/src-tauri/src/meeting/record.rs`, `splashdown-audio/src/live.rs` | A meeting: takes both sides' audio twice a second (the microphone on its own recorder thread, the computer's sound from the Swift shim's Core Audio tap or WASAPI loopback), spools it, cuts each side into turns and writes them into the note |
| `calls` | `app/src-tauri/src/calls.rs` | Watches which apps use the microphone, to offer recording a call |
| `splashdown-llm` (separate process) | `crates/splashdown-llm` | Runs the built-in local text model when it's in use; started on demand by `splashdown-providers/src/local_llm.rs`, exits after 10 idle minutes |
| `splashdown-updates` | `app/src-tauri/src/updates.rs` | Release builds only: checks for a new version (tauri-plugin-updater, GitHub releases) 30 s after launch and then daily, if allowed in Settings. Installing happens only from the user's click |
| Tauri async runtime | the `#[tauri::command]` functions | Commands from the windows. Slow work (building providers, downloads, writing a draft) runs here or in `spawn_blocking`, never on the main thread |

**The recognizer is shared** (`Arc<Mutex<Recognizer>>`) between the key thread and the session thread. The key thread must answer "swallow or not" immediately, so state that has to be read at key time lives in the recognizer, not the session. Auto-send is the example: while the session is busy processing a dictation, a Return press toggles a flag in the recognizer, and the session reads it with `take_send()` just before inserting.

**The session thread blocks while processing.** It runs the pipeline with `block_on`, so during a network call it doesn't handle other messages; they queue and run afterward. That keeps the controller simple: one dictation at a time, in order. Anything the user must see during processing (the auto-send badge) is updated from the key thread directly through the cloneable `Ui` handle. Work that may take long or wait for the user (a voice action, a Compose draft written into the field) runs elsewhere and comes back as a message (`Msg::ActionDone`, `Msg::Composed`).

## A dictation, from key press to inserted text

1. **The key hook** (`mac/keys.rs` or `win/keys.rs`) calls the handler that `Controller::try_listen` gave `splashdown_platform::listen_keys`. The handler passes the event to `Recognizer::on_event` (`splashdown-core/src/gesture.rs`), which answers at once whether to swallow the key, and may return an `Action`. The handler sends it to the session thread as `Msg::Action`.
2. **Key down**: `Controller::run` gets the message and calls `handle`, and `Action::Start` calls `start`. It starts the microphone first (so the first word isn't lost), then captures the focused field (`Focus::capture`) and its selection, and picks a preset by app or URL (`match_preset`). It also starts what runs while the user speaks: the live preview, chunked transcription (`chunks.rs`), a streaming model's `LiveStream`, and reading the names on screen (`read_screen_words`).
3. **While holding**: `on_tick` runs every 50 ms for the level meter, the preview and the pieces. Number keys switch preset (`Action::SelectPreset`), Return toggles auto-send (on the key thread), the instruction key marks where the instruction starts (`Action::MarkInstruction`), and other keys mean the user was typing a shortcut, so the recording is silently canceled.
4. **Release** → `Action::Finish` → `finish`. Ask, Compose and `listen` recordings go their own way (`ask`, `answer`). Otherwise `prepare` stops the microphone and decides the mode (`split_recording`): Dictate, Dictate + instruction, Transform (text was selected) or Raw (the preset has the AI off). The instruction key with nothing said before it is a spoken command, which goes to `run_command` instead. `rescue::start` keeps the recording on disk until its text is in.
5. **Pipeline**: `run_pipeline` calls `Pipeline::run_streaming` (`splashdown-core/src/pipeline/audio.rs`) under `block_on`: speech-to-text (joined to the pieces already done) → snippet placeholders → the AI pass (`prompt.rs`; instructions may go to a separate cloud model) → the sanity guard (`guard.rs`), which falls back to the raw transcript if the output looks wrong. With typing as it's written on, `stream_typing.rs` types words as they arrive. An instruction may instead ask for a voice action (`do_action`), a draft (`show_draft`) or reading aloud (`read_aloud`).
6. **Insert**: `deliver` records the history entry, then inserts only if the same field still has focus (`Focus::is_current`); otherwise the text goes to the clipboard. `splashdown_platform::insert_text` pastes (saving and restoring the clipboard) or types key events, as the preset says. Return follows if auto-send was asked for.
7. **Afterward**: `check_corrections` reads the field every 0.5 s for 30 s after the insertion, then looks once more after 2 minutes if that field is still in use. When the user has fixed a name or term in what was inserted (`splashdown_core::vocab::corrections`), `learning::learn` adds it to the vocabulary at once; the overlay says "Learned …", and the menu has an item to forget it.

## Features beyond dictation

- **Compose** (`splashdown-core/src/compose.rs`, `app/src-tauri/src/compose.rs`): the user says what they want to write ("reply saying I'm in"), after the instruction key, or while holding the dictation key after pressing the Compose letter (off unless set). By default `show_draft` (`session/assist.rs`) writes the draft straight into the field; the panel is used when Settings asks for it (`compose_panel`), when text is selected, or when there's no field. With nothing selected, it reads the conversation around the field (`Focus::surrounding_text`, setting `compose_reads_window`), and `compose::their_messages` picks out the user's own earlier messages there, so the draft is written in their voice (`compose_style`).
- **Ask** (`splashdown-core/src/ask.rs`, `app/src-tauri/src/ask.rs`): a spoken question (dictation key + A, or a command that is a question) answered in a panel, drawing on the selection, the text around the cursor, the clipboard and matching excerpts of the user's notes. Insert pastes the answer; follow-up questions carry on the conversation.
- **Reading aloud** (`splashdown-core/src/aloud.rs`, `app/src-tauri/src/reading.rs`): "read this to me", a spoken summary, or the Ask panel's speaker button. English is read in the natural voice (Phonon, `splashdown-providers/src/natural_voice.rs`) when it's downloaded and chosen; otherwise the system's voices read it (the Apple shim on macOS, `splashdown-platform/src/win/speech.rs` on Windows). Nothing read this way leaves the computer.
- **Voice actions** (`splashdown-core/src/action.rs`, `app/src-tauri/src/voice_actions.rs`, `my_actions.rs`): the command interpreter says `do`, the planner picks one action from `CATALOG` and its parameters are checked in code, then `voice_actions::carry_out` does it off the session thread. Nothing in the catalog sends, deletes or pays.
- **Your writing style** (`splashdown-core/src/style.rs`, `app/src-tauri/src/style.rs`): after Compose writes a draft into the field, `watch_draft` (`session/assist.rs`) watches it for up to 10 minutes with the correction watcher; what's sent instead is kept per kind of app (`splashdown_config::style_kind`, from the built-in presets' app lists) in `style.json`. Every few edits, the AI writes a short profile from them (`style::profile_request`), which Settings shows and the user can edit, and Compose follows (`ComposeContext::style`), except when the user asks for a formal message.
- **Names on screen** (`read_screen_words` in `session/job.rs`, `splashdown_core::vocab::terms_on_screen`): at key down, the text around the field is read off the session thread, and the names and terms in it join that dictation's vocabulary (`with_screen_words`, setting `screen_words`).
- **Notes** (`app/src-tauri/src/notes/`): notes are Markdown files in a folder the user picks (`store.rs`), with no database: the folder is the truth, and other apps may change it at any time. Dropped audio and video files and page links become transcript notes (`files.rs`).
- **Meetings** (`app/src-tauri/src/meeting.rs`, `splashdown-core/src/meeting.rs`): a call recorded as two sides, You (the microphone) and Them (what the computer plays), transcribed a turn at a time into a note in `Meetings/` and summarized when it ends with the built-in "Meeting summary" transform. `calls.rs` offers to record when a call app starts using the microphone.

## Other triggers: the control channel and links

Besides the dictation key, the session takes `Msg::Control` requests (`app/src-tauri/src/session/control.rs`). They come from:
- **The control channel** (`crates/splashdown-control`): a Unix socket at `Paths::control_socket` (`~/Library/Application Support/Splashdown/control.sock`), or the named pipe `\\.\pipe\splashdown-<user>-<session>` on Windows (one per user and sign-in session). One JSON line each way per connection, answered with `{"ok":true}`, `{"ok":true,"text":"…"}` or `{"ok":false,"error":"…"}`. The requests (`Request` in `lib.rs`):
  - dictation: `toggle`, `start`, `stop`, `cancel`, and `listen` (with a `prompt`), which answers with the cleaned-up text;
  - notes: `note` (with `text`, add it to the capture note; without, start or finish a hands-free note), and for coding agents `search_notes`, `read_note` and `append_note`, which answer with JSON and only work when Settings allows them;
  - meetings: `meeting_start`, `meeting_stop`, `meeting_status`;
  - the app: `status`, `settings` (show the Settings window; a second copy of the app sends this, then quits) and `reload_settings` (`config.toml` changed outside the app).

  `splashdown-cli` and `splashdown-mcp` are its clients.
- **`splashdown://` links** (`app/src-tauri/src/triggers.rs`): the scheme is registered in `app/src-tauri/Info.plist`, and macOS delivers opened links as Tauri's `RunEvent::Opened`. `splashdown://toggle`, `start`, `stop`, `cancel` and `note` work; `listen` doesn't, since there'd be nobody to give the answer to. Links are macOS only for now.

Security: the socket is created under a temporary name, set to mode 0600, then renamed into place, and every connection's peer is checked with `getpeereid` to be the same user (others get `permission denied`). A live socket from another running copy is never replaced; a stale one is. The Windows pipe refuses remote clients and keeps the default DACL, under which only this user, SYSTEM and administrators can write. Requests are size-limited and must arrive within 5 seconds.

Control requests go through the same `Recognizer` as the key: `start_hands_free` latches a hands-free recording (so a tap of the dictation key finishes it and Esc cancels it), and `finish`/`cancel` end one. A `listen` recording drops the focused field and selection, uses the pinned or default preset, shows the question in the overlay (`OverlayState::Question`), and when it finishes, `answer` sends the text back to the requester instead of inserting it. If the recording is discarded, its reply channel is dropped and the requester hears "Canceled".

## Providers and "Automatic"

`splashdown-config::Settings::resolve_speech_with` / `resolve_ai_with` (`settings/resolve.rs`) turn the saved settings plus available keys and on-device models into a concrete `SpeechChoice` / `AiChoice`. `splashdown-providers::factory` turns those into trait objects. The order for "Automatic" is: a saved key (the user chose that provider; for the AI pass, OpenAI, then OpenRouter, then Anthropic) → the built-in models, when downloaded (Nemotron, Qwen3 4B; they beat Apple's in the eval) → Apple's on-device models → local speech-to-text with no AI pass. Local-only mode is enforced here, in one place. The models that can be downloaded, with their sizes and licenses, are listed in `splashdown-providers/src/models.rs`.

## The Apple shim

`crates/splashdown-apple-shim/swift` is a Swift package built by `build.rs` with `swift build` and linked statically. Each async entry point takes a C callback and a context pointer, and calls back exactly once; `src/ffi.rs` turns that into a Rust future with a one-shot channel. `FoundationModels` exists only on macOS 26+, so it's weak-linked (see `.cargo/config.toml`), and every Swift entry point checks `#available` first. On older systems the providers report "unavailable" instead of crashing.

## Prompts and evals

Each prompt lives next to the code that uses it: `splashdown-core/src/prompt.rs` (the AI pass: cleanup, instructions, transforms), `command.rs` (spoken commands), `action.rs` (planning voice actions), `compose.rs` and `ask.rs`; the built-in transforms' instructions, such as the meeting summary, are in `splashdown-config/src/transforms.rs`. Dictated and selected text always goes in tagged blocks, and only the `<instruction>` block is ever obeyed. Every prompt but Ask's has an eval set in `eval/`, which also covers prompt injection; [CONTRIBUTING.md](../CONTRIBUTING.md#changing-the-ai-prompts) says which to run, and [eval/README.md](../eval/README.md) has the results.

## The UI

Each window is a page in `app/ui` with its own script, loaded after `common.js`, with no bundler or modules:

| Window label | Page | Opened by |
|---|---|---|
| `settings` | `index.html` and `settings/*.js` | `tray.rs` |
| `onboarding` | `onboarding.html` (the setup guide) | `tray.rs` |
| `notes` | `notes.html` (its editor is `vendor/editor.js`, built from `app/editor`) | `notes/mod.rs` |
| `ask` | `ask.html` | `ask.rs` |
| `compose` | `compose.html` | `compose.rs` |
| `help` | `help.html` ("What can I say?") | `help.rs` |
| `call-offer` | `offer.html` | `calls.rs` |
| `meeting-pill` | `pill.html` | `pill.rs` |
| `prompt` | `prompt.html` | `prompt.rs` |

**Calling Rust.** Pages call `invoke(command, args)` and `listen(event, handler)` from `common.js`, which wrap `window.__TAURI__.core.invoke` and `window.__TAURI__.event.listen`. Argument names are camelCase in JavaScript (`{ speechOnly: true }`) and snake_case in Rust (`speech_only`); Tauri maps one to the other. Commands are spread over the modules that own them (`commands.rs`, `notes/`, `meeting.rs`, `ask.rs`, `compose.rs`, `reading.rs` and others), and all are registered in `main.rs`'s `invoke_handler`.

**Rust telling a page something.** Rust sends events with `emit` or `emit_to(window label, …)`. Each panel's module has a `WINDOW` label and a `CHANGED` event named after it (`ask-changed`, `compose-changed`, `help-changed`, `prompt-changed`, `call-offer-changed`) that carries the panel's new state. Others: `settings-reloaded`, `show-tab` and `update-status` (Settings), `model-download` and `model-downloaded`, `setup-progress`, `reading-changed`, `meeting-status` and `meeting-message`, and the Notes window's `notes-*` events.

**Adding a window** means a page in `app/ui`, a module that builds it with `WebviewWindowBuilder`, and its label in the `windows` list of `app/src-tauri/capabilities/settings.json`. That file grants the core permissions (events, dragging a window); a window left out of it can't listen for events.

**The pages are compiled into the app** (`frontendDist` in `tauri.conf.json`), so a change in `app/ui` shows up only after a rebuild: `scripts/run-mac-app.sh` or `scripts\run-win-app.ps1` does it.

**The Notes window's script** is split into `app/ui/notes/`, one file per part (the list, folders, the note, meetings, rewrites, Ask, ⌘K, …), loaded in order by `notes.html`. They're classic scripts sharing one global scope (so the window still opens as a plain file): a later file may use an earlier one's names when it loads, not the reverse; pass a later function as `(...args) => later(...args)`.

**Opened directly in a browser**, `invoke` goes to the page's `mockInvoke` instead, so a page can be styled without the app. Only Settings (`settings/mock.js`), the setup guide and Notes (a `mockInvoke` at the end of `onboarding.js`, and `notes/preview.js`) have one; the other windows need the app.

## Where a user's data lives

Locations come from `Paths` in `crates/splashdown-config/src/lib.rs`.

| What | macOS | Windows |
|---|---|---|
| Settings (`config.toml`), history (`history.jsonl`, with recordings in `audio/` when they're kept), spending (`spend.json`), Insights (`insights.json`), the writing style (`style.json`), the mini bar's place, a failed recording kept for Retry, a meeting's audio while it records (`meetings/`), the control socket | `~/Library/Application Support/Splashdown` | `%APPDATA%\Splashdown` |
| Downloaded models: speech, the built-in text model, voices, the link helper | `~/Library/Caches/Splashdown/models` | `%LOCALAPPDATA%\Splashdown\models` |
| Logs (`splashdown.log`) | `~/Library/Logs/Splashdown` | `%LOCALAPPDATA%\Splashdown\logs` |
| Notes: Markdown files, with earlier versions in `.splashdown/` and meeting recordings in `Attachments/` | `~/Documents/Splashdown Notes`, unless the user picks another folder | `Documents\Splashdown Notes`, the same |
| API keys | Keychain | Credential Manager |

[PRIVACY.md](PRIVACY.md#deleting-everything) lists what to remove to delete everything.
