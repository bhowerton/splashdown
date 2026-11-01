# Changelog

What changed in each release of Splashdown. Downloads are on the [releases page](https://github.com/bhowerton/splashdown/releases).

## Unreleased

- **Much faster local speech-to-text.** A new engine runs NVIDIA's Nemotron and Parakeet models as well as Whisper. The new default, Nemotron (English), transcribes while you speak, so your text is ready almost the moment you let go of the key, and it shows the words as you speak. Nemotron 3.5 covers 32 languages. Models download when you choose them (Settings → Providers → Speech to text → On this computer), and a model is only loaded once you've chosen it; Splashdown says when it's still loading. The live words while you speak now need speech-to-text on your computer. A Whisper model downloaded before needs downloading again in the new format.
- **Notes replace the scratchpad.** ⌃⌥N opens Notes. Each note is a plain Markdown file in `Documents/Splashdown Notes`, or any folder you choose, such as an Obsidian vault or iCloud Drive. Notes has search, pins, earlier versions you can restore, and Show in Finder. Deleted notes go to the Trash. Your scratchpad notes move in the first time you open it.
- **Transcribe audio and video files.** Drop a voice memo, interview or video onto Notes (or use ⋯ → Transcribe a file…) and it becomes a note, with the time in the file at the start of each paragraph. It uses the speech-to-text you dictate with, so with a model on your computer nothing leaves it. Reads MP3, M4A, WAV, FLAC, OGG, MP4 and MOV, among others. Also `splashdown-cli transcribe-file`.
- **Dictate straight into a note.** Hold the dictation key and press N: what you say is added to your Inbox (or today's daily note, or the last note opened) instead of typed. ⌃⌥Z takes it back. Also `splashdown-cli note`, `splashdown://note`, and **Save to notes** in History.
- **Notes for coding agents.** If you allow it (Settings → Privacy, off by default), agents set up with `splashdown-mcp` can search and read your notes, and add to them.
- **Transforms.** Saved instructions you run without speaking: Clean up, Summary, To-dos, Email, Prompt for an AI and Meeting summary, or your own (Settings → Words). In Notes they work on the note or the selected text, and ⌘Z or History undoes them. Give one a shortcut and it works on the selected text in any app.
- **Bring your words from another dictation app.** `splashdown-cli words import <file>` adds the vocabulary, corrections and snippets from another app's export, a word list or a CSV file, without changing what you already have.
- **Earlier dictations as context.** What you dictated into the same app in the last few minutes goes along with the next dictation, so "it" and "that" make sense and names are spelled the same way. Kept in memory only; switch it off in Settings → Privacy.
- **Other sounds while you speak.** Settings → General can turn music and videos down or mute them while you hold the key, and put them back afterwards.
- **Fewer made-up words.** A small speech detector checks a recording has a voice in it before it's transcribed, so a fan or a click no longer comes back as "Thank you." Local Whisper output that only repeats its vocabulary hints is dropped.
- **The last word isn't cut off.** Recording carries on for a moment after you let go of the key.
- **New lines don't send messages.** Presets that type instead of paste send line breaks as Shift+Return, which chat apps take as a new line. Terminals still get Return.

## 0.1.1 — 2026-10-31

- **Windows builds are code-signed** by Point Dynamics, LLC, so Windows names the publisher instead of "Unknown". A new release can still get a SmartScreen notice until enough people have downloaded it.
- **Windows updates itself** from this version on. If you installed 0.1.0 on Windows, download 0.1.1 by hand once.
- **Install with Homebrew** on a Mac: `brew install --cask bhowerton/tap/splashdown`.
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
