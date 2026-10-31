<p align="center">
  <img src="app/src-tauri/icons/source/icon.svg" width="128" alt="Splashdown's logo: a space capsule splashing down under an orange-and-white parachute">
</p>

<h1 align="center">Splashdown</h1>

<p align="center">
  <strong>Hold a key. Say it. It's typed.</strong><br>
  Free, open-source dictation for Mac and Windows. Works in every app. No subscription, no account.
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
- **Private when you want it.** Run it entirely on your computer with Apple's built-in models or the included local models, so nothing leaves your machine.

<p align="center">
  <img src="docs/assets/readme/presets.png" width="266" alt="Settings, Presets: Default, Email, Texting, Work chat and Code presets, with cleanup level and tone choices">
  <img src="docs/assets/readme/insights.png" width="266" alt="Settings, Insights: words this week, words per minute, time saved, a 30-day chart and words per app">
  <img src="docs/assets/readme/privacy.png" width="266" alt="Settings, Privacy: where speech and text go for the current settings, local-only mode, and what the AI is told">
</p>

## Everything it does

### Dictate anywhere

- **Hold to talk.** Hold **Right Option** (Right Ctrl on Windows), speak, let go. The text lands in whatever field has focus, with a space before it when it needs one.
- **Clean, not rewritten.** The AI pass removes fillers and false starts and fixes punctuation, at the level you choose per app: none, light, medium or high. Spoken formatting ("new paragraph", "comma") works in any language.
- **Live text while you speak**, so you can see it's listening, even when the final text comes from a cloud model.
- **Hands-free** for long dictations: hold the key and press **H**, then let go. Stops on its own after 3 minutes of silence.
- **Whisper mode** for speaking quietly, and **mixed languages** in one sentence.

### Talk to it

- **Instructions while you dictate.** Tap **Right Shift** mid-dictation and say what to do: "make it a bulleted list", "make it an email".
- **Spoken commands, in any language.** Tap Right Shift *before* speaking: "delete that last sentence", "make that more formal", "envíalo". The AI works out what you meant; nothing depends on exact phrases. Editing commands need an app that lets Splashdown read and select its text; when one doesn't, it says so rather than guess.
- **Rewrite a selection.** Select text, hold the key, say what to change.
- **Undo the AI.** **⌃⌥Z** right after it's typed swaps in exactly what you said; **⌃⌥V** pastes your last raw words.

### A style for every app

- **Presets** switch on their own by app, window title or website: Default, Email, Texting, Work chat, Code & Terminal, Notes, and your own. Each sets the tone, cleanup level, spoken language, translation, its own AI model, and whether to press Return when done.
- **Context, only if you want it.** A preset can read the text around the cursor or your clipboard so names are spelled like the thread. Both are off unless you turn them on.
- **Words that stick.** A vocabulary list that learns from your corrections, snippets with fill-ins (`{date}`, `{clipboard}`), and find-and-replace rules.

### On your computer or in the cloud

- **Your own key:** OpenAI, OpenRouter or Anthropic, with a monthly spending limit. Typical use costs cents a month.
- **Fully on your computer:** Apple's on-device speech and language models (Apple Silicon, macOS 26), or local Whisper plus Splashdown's built-in local model, a free download with nothing else to install. Ollama and LM Studio work too.
- **A stand-in when the cloud fails.** If your provider is offline or out of credit, the dictation is retried on your computer, and the result says so.
- Smaller on-device models handle cleanup well but follow instructions less reliably than cloud models, so instructions can be sent to the cloud while plain dictation stays local.

### Everything else

- **History** (off by default; optionally with recordings, so you can transcribe again later), **Insights** (words, speed, time saved, streaks), **Notes** for when there's no text field: plain Markdown files in a folder you choose (Obsidian, iCloud Drive and Dropbox work), with search, pins and earlier versions, and an always-on-screen **mini bar** for clicking instead of holding a key.
- **Warns when the microphone hears nothing**, and keeps what you said if it's unplugged mid-sentence.
- **Updates** are checked daily and installed only when you choose.

### For developers

- **Coding agents:** file names you say become `@file` mentions for Claude Code or Cursor, and `splashdown-mcp` lets an agent ask you a question by voice ([docs/AGENTS.md](docs/AGENTS.md)).
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
| Use another preset | While holding, press **1–9** |
| Change the spoken language | Menu bar → **Language** (detects it automatically by default) |
| Cancel | **Esc** |
| Paste exactly what you said, without the AI | **⌃⌥V** |
| Swap the AI's version for exactly what you said | **⌃⌥Z** right after it's typed |
| Dictate a note with no text field around | **⌃⌥N** opens Notes; **Send to** pastes the note into the app you were in |
| Dictate with the mouse | Choose **Mouse button 4** or **5** as the dictation key in Settings; hold it or double-click it like a key |
| Start or finish from Raycast, Alfred or Shortcuts | Open `splashdown://toggle` (also `start`, `stop`, `cancel`) |
| Start or finish from a terminal or script | `splashdown-cli toggle` (also `start`, `stop`, `cancel`); `splashdown-cli listen` prints what you say instead of typing it |

On Windows, Right Ctrl replaces Right Option, and Ctrl+Alt replaces ⌃⌥.

</details>

## Install

**Mac:** [download the DMG](https://github.com/bhowerton/splashdown/releases/latest/download/Splashdown-mac.dmg), open it and drag Splashdown to Applications. Or with [Homebrew](https://brew.sh):

```sh
brew install --cask bhowerton/tap/splashdown
```

Builds are universal (Apple Silicon and Intel), signed with Point Dynamics' Apple Developer ID and notarized, so macOS opens them without warnings. The first launch opens a short setup guide: choose how it should understand you, then allow Accessibility and Microphone.

**Windows:** [download the installer](https://github.com/bhowerton/splashdown/releases/latest/download/Splashdown-windows-x64-setup.exe) and run it; no administrator rights needed. It's code-signed by Point Dynamics, LLC. A brand-new release can still get a SmartScreen notice until enough people have downloaded it: choose **More info → Run anyway**. If you installed 0.1.0, download the latest version by hand once; it updates itself from then on.

Every version is on the [releases page](https://github.com/bhowerton/splashdown/releases), and what changed is in the [changelog](CHANGELOG.md).

## Uninstall

**Mac:** quit Splashdown and move it to the Trash, or `brew uninstall --cask splashdown`. Add `--zap` to also remove its settings, history and downloaded models.

**Windows:** Settings → Apps → Installed apps → Splashdown → Uninstall.

To remove everything it stored, including API keys, see [Deleting everything](docs/PRIVACY.md#deleting-everything).

## Private by default

Splashdown has no servers, no account and no analytics. Your recording and text go only to the provider you chose, directly from your computer, or nowhere at all with on-device models. The only other connections are ones you can see: the daily update check (which can be turned off), model downloads you start, and feedback you choose to send. API keys live in the system keychain, history is off by default, and logs never contain what you said. Local-only mode refuses every cloud provider, even with a key saved.

Settings → Privacy shows where your words go for your current settings. The full list is in [docs/PRIVACY.md](docs/PRIVACY.md).

## What you need

- **Mac:** macOS 13 or later, Apple Silicon or Intel. Apple's on-device models need Apple Silicon and macOS 26.
- **Windows:** Windows 10 or 11, x64. ARM PCs run it through emulation.
- **For the AI:** a key for OpenAI, OpenRouter or Anthropic, or nothing at all with the on-device models. The built-in local model uses about 3 GB of memory; a smaller one (about 1.7 GB) suits 8 GB machines.

## How well it works

Every default was chosen by measurement. The cleanup prompt is tested on 49 cases covering fillers, self-corrections, formatting, instructions, context and prompt injection: the default OpenRouter model scores 96%, the built-in local model (Qwen3 4B) 84%, Apple's on-device model 71%. Results and how to run them are in [eval/README.md](eval/README.md).

## When something misbehaves

[Troubleshooting](docs/TROUBLESHOOTING.md) covers the common fixes: the key does nothing, the microphone hears nothing, text that didn't appear, and provider errors. For anything else, use **Send feedback** in the menu bar, which opens a GitHub issue with your versions filled in.

## Building from source

```sh
cargo build --workspace
cargo test --workspace
scripts/run-mac-app.sh        # macOS: build, sign and launch the menu bar app
scripts\run-win-app.ps1       # Windows: build and launch the tray app
```

You need stable Rust, `cmake` (for local Whisper), and on macOS Xcode 26 or later; on Windows, the MSVC build tools and LLVM. [CONTRIBUTING.md](CONTRIBUTING.md) has the details, including API keys for development, the checks a change must pass, and packaging releases.

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
| `crates/splashdown-providers` | Speech-to-text and AI providers: OpenAI-compatible, OpenRouter, local Whisper, Apple |
| `crates/splashdown-platform` | Hotkeys, text insertion, focus and selection, the overlay (`mac/`, `win/`) |
| `crates/splashdown-apple-shim` | Swift bridge to Apple's on-device speech and language models |
| `crates/splashdown-config` | Settings, provider resolution, presets, keychain, paths |
| `crates/splashdown-cli` | The `splashdown-cli` developer tool and evals, and `splashdown-cli toggle`/`listen` for the running app |
| `crates/splashdown-control` | The running app's control socket: protocol, client, server |
| `crates/splashdown-mcp` | `splashdown-mcp`, voice input for coding agents ([docs/AGENTS.md](docs/AGENTS.md)) |
| `app/` | The menu bar app (Tauri) and its Settings and onboarding windows |
| `docs/` | [Architecture](docs/ARCHITECTURE.md), [spec](docs/SPEC.md), [roadmap](docs/ROADMAP.md) |

</details>

## Documentation

- [Privacy](docs/PRIVACY.md): what leaves your computer, what stays, and how to delete it
- [Troubleshooting](docs/TROUBLESHOOTING.md): the common fixes
- [Changelog](CHANGELOG.md): what changed in each release
- [Voice input for coding agents](docs/AGENTS.md): `splashdown-mcp` for Claude Code, Cursor and others
- [Contributing](CONTRIBUTING.md): building, checks, releases and conventions
- [Security](SECURITY.md): how to report a vulnerability
- [Architecture](docs/ARCHITECTURE.md), [spec](docs/SPEC.md) and [roadmap](docs/ROADMAP.md)

## Community

Bug reports, ideas and pull requests are all welcome; [CONTRIBUTING.md](CONTRIBUTING.md) is the place to start, and the [code of conduct](CODE_OF_CONDUCT.md) applies everywhere. Splashdown is free and will stay that way. If it's earned its key on your keyboard, a ⭐ helps other people find it.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. Contributions are accepted under the same terms; see [CONTRIBUTING.md](CONTRIBUTING.md). The license covers the code: if you publish a fork, please give it its own name and icon.

Releases are signed and published by Point Dynamics, LLC, which is why macOS and Windows name it as the developer.

<p align="center">
  <sub>Made by <a href="https://github.com/bhowerton">@bhowerton</a></sub>
</p>
