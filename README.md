<p align="center">
  <img src="app/src-tauri/icons/source/icon.svg" width="128" alt="Splashdown's logo: a space capsule splashing down under an orange-and-white parachute">
</p>

<h1 align="center">Splashdown</h1>

<p align="center">
  <strong>Your voice, for everything.</strong><br>
  Type in any app, tell your computer what to do, hear text read back, and turn anything you hear into notes. Free and open source for Mac and Windows: no subscription, no account, and it can all run on your computer.
</p>

<p align="center">
  <a href="https://splashdown.app">Website</a> ·
  <a href="#install">Install</a> ·
  <a href="#everything-it-does">Features</a> ·
  <a href="#private-by-default">Privacy</a> ·
  <a href="docs/TROUBLESHOOTING.md">Troubleshooting</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="https://github.com/bhowerton/splashdown/issues">Issues</a>
</p>

<p align="center">
  <a href="https://github.com/bhowerton/splashdown/releases/latest"><img src="https://img.shields.io/github/v/release/bhowerton/splashdown?label=version&color=F2551F" alt="Latest version"></a>
  <a href="#what-you-need"><img src="https://img.shields.io/badge/macOS%2013%2B%20%C2%B7%20Windows%2010%2B-16337A" alt="macOS 13 or later, Windows 10 or later"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-16337A" alt="MIT or Apache-2.0"></a>
  <a href="https://github.com/bhowerton/splashdown/releases"><img src="https://img.shields.io/github/downloads/bhowerton/splashdown/total?color=16337A" alt="Downloads"></a>
  <a href="https://github.com/bhowerton/splashdown/actions/workflows/ci.yml"><img src="https://github.com/bhowerton/splashdown/actions/workflows/ci.yml/badge.svg?branch=main&event=push" alt="CI status"></a>
</p>

<p align="center">
  <a href="https://github.com/bhowerton/splashdown/releases/latest/download/Splashdown-mac.dmg"><img src="https://img.shields.io/badge/Download_for_Mac-F2551F?style=for-the-badge&logo=apple&logoColor=white" alt="Download for Mac"></a>
  <a href="https://github.com/bhowerton/splashdown/releases/latest/download/Splashdown-windows-x64-setup.exe"><img src="https://img.shields.io/badge/Download_for_Windows-16337A?style=for-the-badge&logo=data:image/svg%2bxml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI+PHBhdGggZmlsbD0iI2ZmZiIgZD0iTTAgMGgxMS4zdjExLjNIMHpNMTIuNyAwSDI0djExLjNIMTIuN3pNMCAxMi43aDExLjNWMjRIMHpNMTIuNyAxMi43SDI0VjI0SDEyLjd6Ii8+PC9zdmc+" alt="Download for Windows"></a>
</p>

<p align="center">
  <img src="docs/assets/demo.svg" width="800" alt="Animation: holding Option and saying 'um, so can we meet Tuesday… actually Thursday at 3' types 'Can we meet Thursday at 3?' into an email; then tapping Shift and saying 'make it more formal' rewrites it to 'Would Thursday at 3 p.m. work for you?'">
</p>

## Why Splashdown

- **Free and open source, for good.** No subscription, no account, no word limits. Your own API key costs cents a month, and you can set a spending limit.
- **Works everywhere you type.** Email, Slack, Messages, docs, the browser, your terminal or code editor: any text field in any app.
- **Talks like you do, writes like you meant to.** The "um"s and do-overs disappear, and you can say "make it more formal", "turn that into a list" or "translate to Spanish", in any language.
- **Says what to do, not just what to write.** "Open Spotify", "remind me at 5 to stretch", "add dentist to my calendar on Thursday at 4", "pause the music", or your own: "deploy staging", "next slide". Nothing is sent, deleted or bought for you.
- **Reads to you, too.** Select text and say "read this to me", or "read me a summary of this", and it's read aloud in a natural voice that runs on your computer.
- **Turns anything you hear into notes.** Pick any app (a webinar in your browser, a podcast, a video, a call you're only listening to) and what it says is written into a note as it plays. Record meetings without a bot joining, transcribe voice memos and YouTube links, and ask your notes questions. Notes are plain Markdown files.
- **Private when you want it.** Run it entirely on your computer with Apple's built-in models or the included local models, so nothing leaves your machine.

<p align="center">
  <img src="docs/assets/readme/presets.png" width="266" alt="Settings, Presets: Default, Email, Texting, Work chat and Code presets, with cleanup level and tone choices">
  <img src="docs/assets/readme/insights.png" width="266" alt="Settings, Insights: words this week, words per minute, time saved, a 30-day chart and words per app">
  <img src="docs/assets/readme/privacy.png" width="266" alt="Settings, Privacy: where speech and text go for the current settings, local-only mode, and what the AI is told">
</p>

<p align="center">
  <img src="docs/assets/readme/notes.png" width="266" alt="Notes: folders and tags on the left, the notes list, and a note with its tags and a Rewrite bar">
  <img src="docs/assets/readme/meeting.png" width="266" alt="A meeting note: Summary, Transcript and My notes tabs, with the overview, decisions and action items">
  <img src="docs/assets/readme/ask.png" width="266" alt="Ask beside a note: a question about the launch, answered with numbered links to the notes it came from">
</p>

## Everything it does

### Dictate anywhere

- **Hold to talk.** Hold **Right Option** (Right Ctrl on Windows), speak, let go. The text lands in whatever field has focus, with a space before it when it needs one.
- **Clean, not rewritten.** The AI pass removes fillers and false starts and fixes punctuation, at the level you choose per app: none, light, medium or high. Spoken formatting ("new paragraph", "comma") works in any language.
- **No waiting for the AI:** with a cloud AI, its text is typed as it's written, starting a moment after you let go.
- **Live text while you speak** with speech-to-text on your computer: Nemotron writes as you speak, and your text is ready the moment you let go.
- **Hands-free** for long dictations: hold the key and press **H**, then let go. Stops on its own after 3 minutes of silence.
- **Straight into a note:** hold the key and press **N**, and what you say is added to your Inbox (or today's daily note) instead of typed. ⌃⌥Z takes it back.
- **Music steps aside.** Other sounds are turned down while you speak, as with Siri, or muted or paused if you prefer. On audio interfaces whose volume can't be changed, the music is paused instead.
- **Whisper mode** for speaking quietly, **mixed languages** in one sentence, and **background noise removed** (fans, traffic, café chatter) on your computer before your speech is transcribed, if you turn it on.

### Talk to it

<p align="center">
  <img src="docs/assets/actions.svg" width="800" alt="Animation: holding Option, tapping Shift and saying 'remind me at 5 to stretch' adds a reminder for 17:00; 'search YouTube for paella recipes' opens the search; 'deploy staging', one of your own actions, shows exactly what will run and asks first, then runs it. Music in the corner is turned down while each command is said.">
</p>

- **Instructions while you dictate.** Tap **Right Shift** mid-dictation and say what to do: "make it a bulleted list", "make it an email".
- **Spoken commands, in any language.** Hold Right Option and tap Right Shift *before* speaking: "delete that last sentence", "make that more formal", "envíalo". The AI works out what you meant; nothing depends on exact phrases. Editing commands need an app that lets Splashdown read and select its text; when one doesn't, it says so rather than guess.
- **Voice actions.** Turn them on in Settings → Actions, and the same commands can do things: "open Spotify", "remind me at 3 to call Sam", "add dentist to my calendar on Thursday at 4", "search YouTube for paella recipes", "pause the music", "start recording the meeting", or, with text selected, "email this to Sam". "Undo that" takes back an event, reminder or to-do. Nothing is sent, deleted or bought for you: an email opens as a draft, and a Shortcut asks before it runs.
- **Your own actions.** Give a link, a script or program, or keys to press a name, and say it: "deploy staging", "next slide", "open issue ENG-42" (whatever you say after the name can go into the link or the script). Splashdown shows exactly what will run and asks first, until you choose Always allow.
- **"What can I say?"** Say it (or "help") for a window with everything you can say or press, with your own keys and actions. Also in the menu bar under Help.
- **Rewrite a selection.** Select text, hold the key, say what to change.
- **Undo the AI.** **⌃⌥Z** right after it's typed swaps in exactly what you said; **⌃⌥V** pastes your last raw words.

### A style for every app

- **Presets** switch on their own by app, window title or website: Default, Email, Texting, Work chat, Code & Terminal, Notes, and your own. Each sets the tone, cleanup level, spoken language, translation, its own AI model, and whether to press Return when done.
- **Context, only if you want it.** A preset can read the text around the cursor or your clipboard so names are spelled like the thread. Both are off unless you turn them on.
- **Words that stick.** Names and terms in the window you're typing in (the email or chat on screen) are spelled right without adding them anywhere. A vocabulary list that learns from one correction (fix "Pria" to "Priya" once and it's learned, with a note saying so and one click to forget it), snippets with fill-ins (`{date}`, `{clipboard}`), and find-and-replace rules.
- **Bring your words, share your presets.** Settings → Words & language → Import… brings the vocabulary, corrections and snippets from another dictation app's export, a word list or a CSV file. Share… on a preset saves it as a small file anyone can import.

### On your computer or in the cloud

- **Your own key:** OpenAI, OpenRouter or Anthropic, with a monthly spending limit. Typical use costs cents a month.
- **Fully on your computer:** Apple's on-device speech and language models (Apple Silicon, macOS 26), or local speech-to-text (Nemotron, which writes as you speak, or Parakeet and Whisper) plus Splashdown's built-in local model, a free download with nothing else to install. Ollama and LM Studio work too.
- **Quick even on an older PC.** The built-in model uses the processor's fast instructions on Windows and Intel Macs, and the graphics chip on Windows (Vulkan) where there is one. Setup times it on your computer, and if it's slow, says so and suggests a cloud key.
- **A stand-in when the cloud fails.** If your provider is offline or out of credit, the dictation is retried on your computer, and the result says so.
- Smaller on-device models handle cleanup well but follow instructions less reliably than cloud models, so instructions can be sent to the cloud while plain dictation stays local.

### Ask a question

- **Hold the key, press A, ask.** "What's the German word for deadline?", or select text first and ask "what does this say?". The answer appears in a small panel with Insert and Copy, and can draw on your notes ("what did we decide about the launch?"). Follow-ups carry on the conversation.

### Write it for me

- **Say what to write, not the words.** Hold Right Option, tap **Right Shift** and say "reply saying I'm in and ask about parking", "tell the team the launch moved to Thursday" or "decline politely and suggest next month". It's written straight into the reply, for the app you're in and the message you're answering, in your language. There's no need to select the message: Splashdown reads the conversation around where you're typing (the email thread or chat on screen), on your computer. Settings → Privacy turns reading the window off.
- **Sounds like you.** Drafts follow your own messages in the conversation (your greeting, sign-off, capitals, emoji), and Compose learns from how you edit its drafts before sending, separately for email, work chat and texts. Settings → Words & language shows what it learned, to change or forget.
- **Change it by voice:** select the draft and say "shorter" or "warmer", or "scratch that" to take it back. To check drafts first, turn on Check drafts in a panel first: you can change the draft there by voice ("add that I'm free Thursday", "in German") and every version is kept until you Insert it.
- **No made-up facts.** Names, dates, numbers and your own details come only from what you said, the message and your notes; anything missing is left as a [placeholder]. Instructions for an AI hidden in the message you're answering are left out before anything is written.

### Hear it read back

- **"Read this to me."** Select text, hold Right Option and say it, and it's read aloud. With nothing selected, tap Right Shift first and the text you're writing is read, or your last dictation (also in the menu bar: Last dictation → Read it to me).
- **"Read me a summary of this."** Your AI writes a short summary meant for listening, and it's read as it's written, so the voice starts after the first sentence. "Read me the action items" and other questions work the same way, and the answer stays in the Ask panel, which has a Read aloud button for any answer.
- **A natural voice, on your computer.** Phonon 31M (Gradium, Apache-2.0), a 68 MB download with 25 American and British voices, reads English with numbers, dates and amounts said properly, and starts speaking in a fraction of a second even on a slow PC. Other languages use the best system voice for that language. Music pauses while it reads, and Esc stops it. Settings → AI Models → Text to speech.

### Notes

- **⌃⌥N opens Notes**, for when there's no text field around. Every note is a plain Markdown file in a folder you choose, so Obsidian, iCloud Drive and Dropbox work, and nothing is locked in.
- **Folders and tags.** Nested folders (the real ones on disk) and Obsidian-style `#tags`, nested with `/`, in a column with counts. Drag a note onto a folder, or use Move to….
- **Formatting as you write:** headings, bullets and checkboxes you can click, with the Markdown marks out of the way.
- **⌘K** finds any note, folder or tag, and anything the window can do. **⌘T** opens today's note, and beside Pinned there are To do (notes with an unticked box), Changed today and Untagged.
- **Rewrites you check before keeping.** Clean up, Summary, To-dos, Email, Prompt for an AI, or your own; the result shows what changed, and you keep it or say what to change ("warmer", "in French"). Give one a shortcut to use it on selected text in any app.
- **Ask your notes (⌘J)** about one note or all of them; answers link to the notes they came from. Turn on Find notes by meaning and Ask finds the notes about what you mean, not only ones with the same words, on your computer.
- Pins, earlier versions you can restore, and **Paste into** (⌘↩), which pastes the note into the app you used last.

### Any app's audio, meetings and transcripts

<p align="center">
  <img src="docs/assets/sources.svg" width="800" alt="Animation: a video call, a podcast app, a video in the browser, the room and a file take turns; each one's sound flows into Splashdown and becomes a note line by line, with time stamps, a summary or action items, and tags.">
</p>

- **Meeting notes without a bot.** ⌃⌥M (or Record meeting) records your microphone and what your computer plays, and writes the conversation into a note as it goes, each turn marked You or Them; at the end the other people can be told apart and named, and with your calendar the note takes the meeting's title. Write your own notes beside the live transcript, ask "What did I miss?", and get a summary of decisions, action items and open questions at the end, built around what you wrote down. A kept recording plays back from any turn, and can be transcribed again with another speech model. When Zoom, Teams, Meet or another call app starts using the microphone, Splashdown offers to record.
- **Transcribe an app's audio:** a webinar in the browser, a podcast app, a call you're only listening to. Only that app's sound is recorded, with a level meter that says whether it's hearing words.
- **Transcribe the microphone** for a lecture or a conversation in the room, hands-free.
- **Transcribe files and links:** drop a voice memo, interview or video onto Notes, or paste a YouTube or podcast link.

### Everything else

- **A dictation is never lost.** If the speech service or the AI fails, or Splashdown closes mid-way, the recording is kept and **Retry** (menu bar → Last dictation) turns it into text. A cloud provider that's down is stood in for by a model on your computer.
- **History** (off by default; optionally with recordings, so you can transcribe again later), **Insights** (words, speed, time saved, streaks), and an always-on-screen **mini bar** for clicking instead of holding a key.
- **Warns when the microphone hears nothing**, and keeps what you said if it's unplugged mid-sentence.
- **Updates** are checked daily and installed only when you choose.
- **A setup guide that has you try it.** The first run shows what Splashdown does, gets speech to text ready in about a minute (the AI follows in the background, and is optional), then has you try the main things one at a time: dictate, dictate in one of your own apps, change selected text, hear it read back, and do things. Each ticks off when it works, and any can be skipped.

### For developers

- **Coding agents:** file names you say become `@file` mentions for Claude Code or Cursor, and `splashdown-mcp` lets an agent ask you a question by voice and, if you allow it, search, read and add to your notes, and go through your meetings by date ("recap my meetings this week") ([docs/AGENTS.md](docs/AGENTS.md)).
- **Scripting:** `splashdown://toggle` links for Raycast, Alfred or Shortcuts; `splashdown-cli toggle` and `splashdown-cli listen` from a terminal; mouse side buttons as the dictation key.

<details>
<summary><strong>All the keys</strong></summary>

| To | Do |
|---|---|
| Dictate | Hold **Right Option**, speak, let go |
| Add an instruction | While holding, tap **Right Shift**, then say it ("make it a bulleted list") |
| Edit or send by voice | While holding, tap **Right Shift** *before* saying anything, then say the command ("delete that last sentence", "capitalize that", "send it") |
| Rewrite selected text | Select text, hold Right Option, say what to do with it |
| Press Return when it's typed | Tap **Return** while holding, or right after letting go (again to undo) |
| Go hands-free | While holding **Right Option**, press **H**, then let go; tap Right Option once more to finish (double-tap can be turned on in Settings) |
| Add to your notes instead of typing | While holding, press **N**; ⌃⌥Z right after takes it back |
| Ask a question | While holding, press **A**, then ask |
| Have a message written | While holding, tap **Right Shift**, then say what to write ("reply saying I'm in"); select it and say "shorter" to change it |
| Hear text read aloud | Select it, hold **Right Option** and say "read this to me" (or "read me a summary of this"); **Esc** stops it. Or menu bar → Last dictation → **Read it to me** |
| Record a meeting | **⌃⌥M** (again to stop) |
| Type exactly what you say, no tidying | While holding, press **2** (the Word for word preset) |
| Use another preset | While holding, press **1–9** |
| Change the spoken language | Menu bar → **Language** (detects it automatically by default) |
| See everything you can say | Hold **Right Option**, tap **Right Shift**, and say "what can I say?", or menu bar → Help → **What you can say…** |
| Cancel | **Esc** |
| Paste exactly what you said, without the AI | **⌃⌥V** |
| Swap the AI's version for exactly what you said | **⌃⌥Z** right after it's typed |
| Dictate a note with no text field around | **⌃⌥N** opens Notes; **Paste into** (⌘↩) pastes the note into the app you were in |
| Get around Notes | **⌘K** any note, folder, tag or action · **⌘J** Ask · **⌘T** today's note · **⌘N** new note · **⌘O** transcribe a file · **⌘L** a link · **⌘E** hide the list · **⌘⇧E** hide folders and tags |
| Dictate with the mouse | Choose **Mouse button 4** or **5** as the dictation key in Settings; hold it or double-click it like a key |
| Start or finish from Raycast, Alfred or Shortcuts | Open `splashdown://toggle` (also `start`, `stop`, `cancel`) |
| Start or finish from a terminal or script | `splashdown-cli toggle` (also `start`, `stop`, `cancel`); `splashdown-cli listen` prints what you say instead of typing it |
| Add to your notes from a terminal or launcher | `splashdown-cli note buy oat milk`, or `splashdown-cli note` (and `splashdown://note`) to dictate one hands-free |
| Script Splashdown | `splashdown-cli status --json`; `settings get`, `set`, `unset` and `list` by key (`general.language`); `models list`, `download` and `use`; `history <words>`; `say "…"` reads text in the natural voice. Exit codes: 0 worked, 1 failed, 2 wrong usage, 3 the app isn't running, 4 no such setting or model |

On Windows, Right Ctrl replaces Right Option, and Ctrl+Alt replaces ⌃⌥.

</details>

## Install

**Mac:** [download the DMG](https://github.com/bhowerton/splashdown/releases/latest/download/Splashdown-mac.dmg), open it and drag Splashdown to Applications. Or with [Homebrew](https://brew.sh):

```sh
brew install --cask bhowerton/tap/splashdown
```

Builds are universal (Apple Silicon and Intel), signed with Point Dynamics' Apple Developer ID and notarized, so macOS opens them without warnings. The first launch opens a short setup guide: one click sets Splashdown up on your computer (it downloads what it needs, about 3 GB, while you carry on), then allow Accessibility and Microphone. If you have an API key, you can use that instead.

**Windows:** [download the installer](https://github.com/bhowerton/splashdown/releases/latest/download/Splashdown-windows-x64-setup.exe) and run it; no administrator rights needed. It's code-signed by Point Dynamics, LLC. A brand-new release can still get a SmartScreen notice until enough people have downloaded it: choose **More info → Run anyway**. If you installed 0.1.0, download the latest version by hand once; it updates itself from then on.

Every version is on the [releases page](https://github.com/bhowerton/splashdown/releases), and what changed is in the [changelog](CHANGELOG.md).

## Uninstall

**Mac:** quit Splashdown and move it to the Trash, or `brew uninstall --cask splashdown`. Add `--zap` to also remove its settings, history and downloaded models.

**Windows:** Settings → Apps → Installed apps → Splashdown → Uninstall.

To remove everything it stored, including API keys, see [Deleting everything](docs/PRIVACY.md#deleting-everything).

## Private by default

Splashdown has no servers, no account and no analytics. Your recording and text go only to the provider you chose, directly from your computer, or nowhere at all with on-device models. The only other connections are ones you can see: the daily update check (which can be turned off), model downloads you start, and feedback you choose to send. API keys live in the system keychain, history is off by default, and logs never contain what you said. Local-only mode refuses every cloud provider, even with a key saved. Reading aloud happens on your computer; a spoken summary is written by the AI you chose.

Settings → Privacy shows where your words go for your current settings. The full list is in [docs/PRIVACY.md](docs/PRIVACY.md).

## What you need

- **Mac:** macOS 13 or later, Apple Silicon or Intel. Apple's on-device models need Apple Silicon and macOS 26.
- **Windows:** Windows 10 or 11, x64. ARM PCs run it through emulation.
- **For the AI:** a key for OpenAI, OpenRouter or Anthropic, or nothing at all with the on-device models. The built-in local model uses about 3 GB of memory; a smaller one (about 1.7 GB) suits 8 GB machines.

## How well it works

Every default was chosen by measurement. The cleanup prompt is tested on cases covering fillers, self-corrections, formatting, instructions, transforms, context, names on screen and prompt injection. Spoken commands, voice actions, Compose and speech-to-text have eval sets of their own. The scores for every model tried, and how to run the evals, are in [eval/README.md](eval/README.md).

**Models you download keep their own licenses**, shown next to each one in Settings:
- NVIDIA Nemotron Speech Streaming (English): NVIDIA Open Model License.
- Nemotron 3.5 ASR Streaming: OpenMDW-1.1.
- NVIDIA Parakeet TDT: CC-BY-4.0.
- NVIDIA Streaming Sortformer (tells speakers apart in meetings): NVIDIA Open Model License.
- OpenAI Whisper: MIT.
- Qwen3 and Qwen3 Embedding (finds notes by meaning): Apache-2.0.
- Gradium Phonon 31M and its voices (reads text aloud): Apache-2.0.

Built in: Silero VAD (finds the speech in a recording), MIT. The Notes editor is built on CodeMirror and Lezer, MIT ([app/ui/vendor/editor.LICENSES.txt](app/ui/vendor/editor.LICENSES.txt)).

Downloaded when you first transcribe a page link (YouTube and the like): yt-dlp, public domain (the Unlicense), and QuickJS-NG, MIT.

The speech models are the GGUF conversions published by the transcribe.cpp project.

## When something misbehaves

[Troubleshooting](docs/TROUBLESHOOTING.md) covers the common fixes: the key does nothing, the microphone hears nothing, text that didn't appear, and provider errors. For anything else, use **Send feedback** in the menu bar's Help menu, which opens a GitHub issue with your versions filled in.

## Building from source

```sh
cargo build --workspace
cargo test --workspace
scripts/run-mac-app.sh        # macOS: build, sign and launch the menu bar app
scripts\run-win-app.ps1       # Windows: build and launch the tray app
```

You need stable Rust, `cmake` (for local speech-to-text), and on macOS Xcode 26 or later; on Windows, the MSVC build tools and LLVM. [CONTRIBUTING.md](CONTRIBUTING.md) has the details, including API keys for development, the checks a change must pass, and packaging releases.

The `splashdown-cli` command-line tool runs the same pipeline without the app, which is handy for testing providers and prompts:

```sh
cargo run --release -p splashdown-cli -- clean "um so I think uh we ship friday no wait thursday"
cargo run --release -p splashdown-cli -- record                    # speak, press Enter
cargo run --release -p splashdown-cli -- eval cleanup --ai openrouter
```

<details>
<summary><strong>Project layout</strong></summary>

| Path | What |
|---|---|
| `crates/splashdown-core` | Pipeline, prompts, sanity guard, hotkey gestures (no OS code) |
| `crates/splashdown-audio` | Microphone capture, resampling, silence trimming |
| `crates/splashdown-providers` | Speech-to-text and AI providers: OpenAI-compatible, OpenRouter, Anthropic, local speech-to-text (transcribe.cpp), the built-in local model, Apple; the model catalog and the natural reading voice |
| `crates/splashdown-platform` | Hotkeys, text insertion, focus and selection, the overlay (`mac/`, `win/`) |
| `crates/splashdown-apple-shim` | Swift bridge to Apple's on-device speech and language models, calendar, system voices and the computer's sound for meetings |
| `crates/splashdown-llm` | The helper process that runs the built-in local text model (llama.cpp) |
| `crates/splashdown-config` | Settings, provider resolution, presets, keychain, paths |
| `crates/splashdown-cli` | The `splashdown-cli` developer tool and evals, and `splashdown-cli toggle`/`listen`/`note` for the running app |
| `crates/splashdown-control` | The running app's control socket: protocol, client, server |
| `crates/splashdown-mcp` | `splashdown-mcp`, voice input and notes for coding agents ([docs/AGENTS.md](docs/AGENTS.md)) |
| `app/` | The menu bar and tray app (Tauri) and its windows: Settings, setup guide, Notes, Ask, Compose and a few small panels; `app/editor` builds the Notes editor |
| `docs/` | [Architecture](docs/ARCHITECTURE.md), [roadmap](docs/ROADMAP.md) |

</details>

## Documentation

- [Privacy](docs/PRIVACY.md): what leaves your computer, what stays, and how to delete it
- [Troubleshooting](docs/TROUBLESHOOTING.md): the common fixes
- [Changelog](CHANGELOG.md): what changed in each release
- [Voice input for coding agents](docs/AGENTS.md): `splashdown-mcp` for Claude Code, Cursor and others
- [Contributing](CONTRIBUTING.md): building, checks, releases and conventions
- [Security](SECURITY.md): how to report a vulnerability
- [Architecture](docs/ARCHITECTURE.md) and [roadmap](docs/ROADMAP.md)

## Community

Bug reports, ideas and pull requests are all welcome; [CONTRIBUTING.md](CONTRIBUTING.md) is the place to start, and the [code of conduct](CODE_OF_CONDUCT.md) applies everywhere. Splashdown is free and will stay that way. If it's earned its key on your keyboard, a ⭐ helps other people find it.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. Contributions are accepted under the same terms; see [CONTRIBUTING.md](CONTRIBUTING.md). The license covers the code: if you publish a fork, please give it its own name and icon.

Releases are signed and published by Point Dynamics, LLC, which is why macOS and Windows name it as the developer.

<p align="center">
  <sub>Made by <a href="https://github.com/bhowerton">@bhowerton</a></sub>
</p>
