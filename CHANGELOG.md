# Changelog

What changed in each release of Splashdown. Downloads are on the [releases page](https://github.com/bhowerton/splashdown/releases).

## 0.1.1 — 2026-10-31

- **Windows builds are code-signed** by Point Dynamics, LLC, so Windows names the publisher instead of "Unknown". A new release can still get a SmartScreen notice until enough people have downloaded it.
- **Windows updates itself** from this version on. If you installed 0.1.0 on Windows, download 0.1.1 by hand once.
- **Install with Homebrew** on a Mac: `brew tap bhowerton/tap`, `brew trust --cask bhowerton/tap/splashdown`, then `brew install --cask splashdown`.
- Releases are now built, signed and notarized by GitHub Actions.
- New [privacy](docs/PRIVACY.md) and [troubleshooting](docs/TROUBLESHOOTING.md) guides.

## 0.1.0 — 2026-10-30

The first public release.

**Dictation**
- Hold Right Option (Right Ctrl on Windows), speak, let go: the text appears in whatever field has focus, in any app or website.
- An optional AI pass removes "um"s and false starts, at the cleanup level each preset chooses (none, light, medium, high). Spoken formatting ("new paragraph", "comma") works in any language.
- Instructions while dictating (tap Right Shift, then say "make it a bulleted list"), and rewriting selected text by voice.
- Spoken commands in any language: tap Right Shift first, then say "delete that last sentence", "make that more formal" or "send it".
- Hands-free (hold the key and press H), whisper mode for speaking quietly, live text while you speak, and mixed-language dictation.

**Presets**
- Default, Email, Texting, Work chat, Code & Terminal and Notes, matched by app, window title or website. Each sets tone, cleanup, language, translation, its own AI model, auto-send and a shortcut.

**Providers and privacy**
- Your own OpenAI, OpenRouter or Anthropic key with a monthly spending limit, or fully on your computer: Apple's on-device models, or local Whisper plus the built-in local model. Ollama and LM Studio work too.
- When a cloud provider fails, the dictation is retried on your computer.
- No servers, no account, no telemetry. History is off by default.

**And more**
- History, snippets, a vocabulary that learns from your corrections, Insights, a scratchpad, a mini bar, Undo AI edit, `@file` mentions for coding agents, a CLI, `splashdown://` links and an MCP server.
- Mac: signed and notarized, universal (Apple Silicon and Intel), macOS 13 or later. Windows: preview, unsigned.
