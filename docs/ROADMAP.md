# Splashdown — Roadmap to a polished 1.0

**Updated:** 2026-10-29. This file is the order of work.

**Status key:** ✅ done · 🟡 partly done · ⬜ not started

## Where things stand

| Milestone | Status | Notes |
|---|---|---|
| M0 Workspace + CI | 🟡 | CI workflow pinned to Xcode 26; it hasn't run yet because the repo isn't on GitHub |
| M1 Core pipeline + evals | ✅ | OpenAI's own API and `gpt-4o-mini-transcribe` not measured yet (no key) |
| M2 Mac platform layer | ✅ | In daily use. App compatibility pass still to do |
| M3 Windows | 🟡 | First run on a real Windows PC on 2026-10-30: builds there and in CI, dictation works. Shipped as a preview in v0.1.0 (unsigned). Detailed per-app checks still to do |
| M4 App shell | ✅ | Settings (all tabs), onboarding, presets, history, privacy, menu bar, overlay by the text cursor. Left: a VoiceOver walkthrough on a real build |
| M4b Power features | ✅ | Learning vocabulary, snippets, spoken commands in any language, auto-send, other triggers, field context, languages and translation |
| M5 Apple on-device | ✅ | Apple speech and cleanup in daily use (cleanup 80%, 0.40 s). Weak-link fallback on macOS 13–15 not tested on a real machine |
| M5b Built-in local model | 🟡 | `splashdown-llm` helper with Qwen3 4B: 84% fully local, 0.13 s on a Mac GPU, 0.41 s CPU-only. Small model for ~8 GB machines: Qwen3 4B compact (IQ3_XXS: 74%, 2.6 GB on the CPU), or Apple's on a Mac with Apple Intelligence. Windows (CPU/Vulkan) untested |
| Open-source readiness | ✅ | Dual MIT/Apache-2.0 license, CONTRIBUTING, ARCHITECTURE, code of conduct, security policy, issue templates, `SAFETY` comments (lint-enforced), license checks (cargo-deny) and third-party notices (cargo-about) |
| M6 Tester beta | ⬜ | Needs signed, notarized release builds |

## Order of work

### 1. Live verification of M2
- ✅ First real dictations on Mac: overlay never takes focus; paste works; Esc cancels; hands-free; instruction key; transform.
- ⬜ Compatibility pass over ~20 apps: TextEdit, Notes, Mail, Safari, Chrome, Firefox, Arc, Slack, Discord, Messages, Notion, Obsidian, VS Code, Cursor, Terminal, iTerm2, Word, Google Docs, Gmail, ChatGPT/Claude web. Record paste, selection and focus behavior per app.

### 2. M2b — Speed and reliability (new)
How fast and how reliably words land is the core of a dictation app; most of what users judge happens here.
- ✅ **Live text while speaking** (2026-10-31): with speech-to-text on this computer, a streaming model (Nemotron) shows the words as you speak and finishes the text on release. With other local models the preview re-transcribes about every 0.6 s. Cloud speech-to-text has no preview: only the model the user chose is ever loaded. ⬜ Streaming the final text from cloud providers (OpenAI realtime).
- ✅ **Faster final text for long dictations:** after 8 s of recording, the audio up to each pause (≥ 0.4 s) is transcribed in the background; on release only the stretch after the last pause is left. The pieces are joined before the AI pass, and if any fails the whole recording is transcribed instead. Plain dictation only (not instructions or transforms). Not yet timed on real long dictations.
- ⏸ **Compressed upload (Opus): deferred.** The 10-minute hands-free cap keeps WAV under OpenAI's 25 MB limit (about 19 MB at most), and a typical 10 s dictation is 320 KB, which uploads in well under a second. Revisit if slow connections show up in testing.
- 🟡 **First-word safety:** ✅ mic open time is logged for every dictation (`mic … opened in N ms`). ⬜ Add a pre-roll buffer only if testing shows clipped first words.
- ✅ **Microphone handling:**
  - A device picker in Settings → General.
  - "Automatic" uses the system default unless it's Bluetooth (detected via Core Audio's transport type), in which case it uses the built-in mic, so AirPods don't drop into call mode.
  - The device is chosen afresh for each dictation, so plugging and unplugging between dictations just works, and an unplugged choice falls back to Automatic.
  - ✅ **A second-choice microphone** (2026-11-01): Settings → General → "If it's not connected": the first of the chosen microphone and this one that's connected is used, then Automatic. Meetings use the same order.
  - ✅ **Mic unplugged mid-recording** (2026-10-30): the recording finishes at once with what was captured, and the result line starts "Microphone disconnected". Relies on cpal reporting `DeviceNotAvailable`; not yet tried with a real unplug.
  - ✅ **Mic hears nothing** (2026-10-30): after 2.5 s of near digital silence (raw level below 0.0004, quieter than any real room) the pill says the microphone may be muted or the wrong one, and goes back to listening as soon as sound arrives. A recording that heard nothing ends with the same hint instead of "Didn't catch that".
- ✅ **Layout-aware shortcuts:** the keys for Cmd+C/V/Z are looked up on the current layout with `UCKeyTranslate` (with Command held, so "Dvorak – QWERTY ⌘" works), on the main thread at startup and as each dictation starts.
- ✅ **Actionable errors:** a rejected key, out of credits, rate-limited, provider down, offline or timed out, and model not downloaded each get one overlay line naming the provider and the fix. When the AI was skipped, the reason is shown next to "raw text".
- ✅ **Hands-free silence auto-stop:** after 3 minutes without speech, as well as the 10-minute cap.
- ✅ **On-device stand-in** (2026-10-30): when cloud speech-to-text or the cloud AI fails (offline, outage, rate limit, bad key), the same dictation is retried on this computer: Apple's speech or local Whisper, and Apple's model or the built-in local model, whichever is installed. The result line adds "done on this computer (cloud didn't answer)". Nothing is set up for this; without an on-device model the cloud error is shown as before.
- ✅ **Local Whisper lets go of memory** (2026-10-30): the model (up to about 1.5 GB) is unloaded after 10 unused minutes and reloaded as the next recording starts, like the built-in local model's helper.

### 3. M4 — Feels like a real app
- ✅ **Onboarding window:** welcome → choose AI (free & private on this computer — Apple's models or local Whisper plus the built-in cleanup model; or an OpenRouter, OpenAI or Anthropic key, each verified; an existing key is detected) → permissions → practice (detects the first real dictation) → done, with a cheat sheet. Opens on first run; "Run the setup guide again" is in About.
- ✅ **Hotkey capture:** one key picker per hotkey, showing the key as a keycap: click it and press the key (or a side mouse button, for dictation), or pick from its ▾ list (Fn, mouse buttons, reset to default). The global hotkey is paused while choosing, for at most 15 s. A legend under the pickers shows what the dictation key does (hold, double-tap, + Return, Esc), with inline tips about conflicts (Option and accented characters, AltGr on Windows, Shift, left-side keys, Fn setup).
- ✅ **Launch at login:** offered at the end of setup (on by default) and switchable in General. Uses `tauri-plugin-autostart` (a Launch Agent on Mac).
- ✅ **Presets and per-app rules**: Default, Email, Texting, Work chat, Code & Terminal, Notes, plus your own (Texting was split out of Chat on 2026-10-30: personal messages and work chat want different styles). Matched by app (with `*` prefix wildcards), window title or **browser URL**, read through Accessibility from the page's web area. Each preset sets AI on/off, extra instructions and Paste or Type. Presets tab with "Add app or website" from recently used apps. *Gap #2.*
- ✅ **Presets redesign:** apps by name and icon, Where / How it writes / Extras sections, tone choices with examples, Try it per preset, drag to reorder with conflict warnings, templates, a *By app* view; per preset: auto-send, spoken language, AI model and a global shortcut. ✅ App names and icons on Windows (icons 2026-11-01, from the executable; ⬜ not yet seen on Windows).
- ✅ **Switch preset while recording:** hold the dictation key and press 1–9; the pill shows "Listening · Email".
- ✅ **History**: History tab with search, copy, copy what was said, **redo as another preset**, delete and clear; keep-history switch; "Paste last raw transcript" (⌃⌥V and menu bar) and "Copy last result". ✅ Optionally keep the audio (off by default, needs history on), with "Transcribe again" using the current speech settings. *Gap #3.*
- ✅ **Privacy page**: where speech and text go, recomputed from the actual settings; **local-only mode** (refuses cloud providers and non-local servers, enforced in config resolution); switch to stop sending the app name; what's kept on this computer.
- ✅ **Sounds:** quiet system "Tink" on start and "Pop" on stop, on by default, switchable.
- 🟡 **Overlay polish:**
  - ✅ Fade in and out (opacity steps, canceled by newer state changes), and a spinner while working.
  - ✅ Respects Reduce Motion; announces states to VoiceOver.
  - ✅ It already appears on the screen with the focused window: `NSScreen.mainScreen` is that screen, not the primary display.
  - ✅ Placed just below the text cursor (above it when there's no room), read with `AXBoundsForRange` on macOS and, on Windows (2026-11-01), the thread's Win32 caret or else UI Automation's caret range; bottom center when the app doesn't report a caret. Switchable in General.
  - ⬜ Scale animation (needs AppKit's animator; opacity-only for now).
- ✅ **Menu bar:**
  - Icon states: red while recording; an attention dot while setup is incomplete.
  - Preset submenu: "Automatic (by app)" or pin a preset for every dictation.
  - "Paste last raw transcript" and "Copy last result". (A Recent submenu was dropped: with history off by default it only repeated "Copy last result"; the History tab covers the rest.)
- 🟡 **Settings design pass:** ✅ visible keyboard focus, screen-reader labels for icon-only and generated controls, `aria-current` on the active tab, inline hotkey and duplicate-snippet warnings, empty states for history. ⬜ A full VoiceOver walkthrough on a real build (a static pass labeled the remaining fields and made progress and practice status live regions).

### 3b. M4b — Power features
Cheap features that close visible gaps.
- ✅ **Dictionary that learns:** about 20 s after an insertion (or at the next dictation), the field is re-read. Retyped words that look like names or terms become suggestions in General → Vocabulary (+ Add / Not now). Only the word is kept; switchable.
- ✅ **Snippets:** spoken trigger → saved text ("my address"), anywhere in a sentence. The AI sees a placeholder and never rewords the saved text; a trigger said on its own is inserted without an AI call. Fill-ins and replacements were added later (see "Second batch of gaps").
- ✅ **Hands-free by hold + H (2026-10-30):** hold the dictation key and press a letter (H by default, changeable in General) to carry on hands-free. Double-tap is now off by default because Claude's Quick Entry (and others) can use a double-tap of Option; it can be turned back on, and mouse buttons always double-click.
- ✅ **Spoken commands, in any language (2026-10-30):** tap the instruction key before saying anything, then say the command. The AI turns it into one of a few actions (replace "that", delete the last word or sentence, select, bold, italic, press Return, undo, write something new), returned as JSON (`splashdown_core::command`), and the app carries it out. Nothing matches words: an earlier version recognized fixed English phrases ("send it", "delete the last sentence"), which missed rewordings ("delete *that* last word") and every other language, so all phrase lists were removed: from commands, spoken formatting (now done by the AI pass, prompt rule 4), file mentions ("dot") and the sanity guard (whose checks are now length, emptiness, question marks in any script and snippets). Tested in English, Spanish, French and German on Gemini Flash Lite and the built-in Qwen3 4B: 13 of 13 each. Commands need an AI model (any provider, including on-device).
- ✅ **Field context per preset:** the text around the cursor (up to 1,000 characters before and 200 after) goes to the AI as reference only, so names are spelled like the thread and instructions in it are ignored. Mac only for now.
- ✅ **Cleanup levels, undo and insights (2026-10-30):**
  - **Cleanup levels** per preset: None, Light (fillers and punctuation only), Medium (as before) and High (tightens rambling, fixes grammar; a High-only example in the prompt makes small models actually do it).
  - **Undo AI edit:** ⌃⌥Z or the menu bar, within 2 minutes and in the same app, undoes the paste and pastes what was said instead. Not offered after a send, for typed insertions or for transforms.
  - **Coding agents:** "Tag files" per preset (on for Code & Terminal): file names said aloud become `@path` mentions of files in the project, found from Terminal's working folder or an editor's open file (`AXDocument`), plus recently changed file names as spelling hints. ⬜ Code identifiers (variables, functions) aren't recognized yet.
  - **Insights:** words per day, speaking speed, time saved compared with typing at 40 wpm, a streak, a 30-day chart and words per app. Kept locally (`insights.json`), switchable.
- ✅ **Second batch of gaps (2026-10-30):**
  - **Clipboard context** per preset: the clipboard's text (up to 2,000 characters) goes to the AI in its own reference-only `<clipboard>` block, used for spelling and when an instruction mentions it ("summarize what I copied"). Shown on the Privacy tab.
  - **Code names for coding agents:** with file tags on, the 40 most-used `snake_case`/`camelCase`/`PascalCase` names in the open file and the 8 most recently changed source files join the vocabulary, alongside recent file names.
  - **Voice editing:** first built on fixed English phrases, then replaced by spoken commands interpreted by the AI (see "Spoken commands, in any language"). Selecting text uses `AXSelectedTextRange` (UI Automation's TextPattern on Windows, untested), so it needs an app that allows it.
  - **Mixed languages:** "Languages you mix" (with auto-detect) tells the speech model and the AI that the speaker may switch mid-sentence, and to keep each part in its language. Gemini on OpenRouter already transcribed an English–Spanish sentence correctly; the hint matters most for Whisper. Apple's speech model takes one language at a time.
  - **Smarter snippets:** fill-ins `{date}`, `{day}`, `{time}`, `{iso-date}`, `{clipboard}`; and find-and-replace rules on the final text (whole words, ignoring case, or a regex with `$1` groups), checked when saved.
- ✅ **Languages:** auto-detect by default, a Language submenu in the menu bar, a note when the English-only Whisper model is chosen, and a per-preset "Translate into" option.
- ✅ **Other triggers:** a `splashdown://` URL scheme and a CLI (for Raycast and Alfred), and mouse side buttons as hotkeys. Built on a local control socket (`splashdown-control`). Links and mouse buttons are macOS-only for now; the CLI and MCP server use a named pipe on Windows (untested).
- ✅ **Mini bar:** an optional round button that stays on screen (Settings → General). Click to start a hands-free dictation into the focused field, click again to finish; it never takes focus, shows recording and working, can be dragged anywhere and remembers where. Right-click for Settings or to hide it (macOS). ⬜ Windows build untested, and has no tooltip or right-click menu yet.
- ✅ **Confirm before canceling a long recording:** Esc on a recording over 30 s asks "Press Esc again to discard"; otherwise recording carries on.
- ✅ **Local-only mode:** one switch on the Privacy tab that refuses every cloud provider (enforced in config resolution).
- ✅ **Auto-send:** Return while holding the dictation key, or right after letting go, presses Return once the text is typed; press again to undo. The overlay shows "↵ Send".
- ✅ **Anthropic key (Claude Haiku 5.5) for cleanup**, with prompt caching. ⬜ Score it on the eval (needs a key).
- ✅ **Spending and budgets:** per-request cost from every provider, a local per-day and per-month record, monthly and daily limits that switch to on-device models, and links to each provider's own limits.
- ✅ **Send instructions to the cloud:** with a local cleanup model, instructions and transforms can go to a cloud model while plain dictation stays local (off by default).
- ✅ **Scratchpad** (replaced by Notes, §7b): ⌃⌥N or the menu bar opens a small always-on-top notepad to dictate into when there's no text field. Autosaved locally (`scratchpad.json`, the current note plus the last 20), with Copy, New note and **Send to {app}**, which re-activates the app that was in front and pastes the note there. ⬜ Not yet tried in the running app; Windows untested.
- ✅ **Background noise reduction:** an opt-in switch (General → While you speak) that runs RNNoise (`nnnoiseless`, pure Rust) on the recording before speech-to-text, then whisper mode's boost if that's on too. About 3 ms per second of 48 kHz audio. In a test with white noise or rumble mixed into `say` output at 0–5 dB, it brought back words the silence trimmer had cut and recordings it had rejected as "no speech". `splashdown-cli transcribe --denoise` compares it on any WAV.
- ✅ **History retention and export:** "Delete after" (Forever, 30, 7 or 1 day) deletes old entries and their recordings; "Export…" saves the history to Downloads as Markdown or JSON.
- ✅ **Settings between computers:** About → Settings exports `config.toml` without API keys and imports one after a confirmation.

### 4. M3 — Windows
- ✅ **First run** (2026-10-30): built and launched on a Windows PC with `scripts/run-win-app.ps1`, and dictation works. CI builds, tests and packages it on `windows-latest`. Shipped as a preview installer in v0.1.0 (unsigned; no update files, so 0.1.0 Windows installs update by hand once).
- ✅ **Filled in before the first run** (2026-10-30): cursor position and selecting a range through UI Automation's TextPattern (spoken commands, smart spacing and field context), checked against the expected text so a field that counts characters differently gets nothing selected rather than the wrong text; the Chromium paste check (by executable name); the Windows version in feedback reports; app names in Presets from the executable's description (via App Paths or a running copy), and their icons (2026-11-01). ✅ The pill following the caret (2026-11-01; not yet tried on a Windows PC).
- 🟡 **Still to check one by one on Windows** (each was written blind, before the first run):
  - Hotkeys with `WH_KEYBOARD_LL` (held modifiers' auto-repeats are filtered; AltGr's fake Ctrl is ignored).
  - Selection, focus, password fields and page URLs with UI Automation (URL: the enclosing document's value, best effort).
  - Insertion with `SendInput` and the clipboard (memory formats only are snapshotted; GDI-handle formats such as metafiles are lost).
  - The overlay as a `WS_EX_NOACTIVATE` layered window drawn with GDI (region-clipped pill without anti-aliasing or shadow; no screen-reader announcements yet).
  - UIPI detection (admin windows): insertion fails and the text is copied instead.
  - Terminal-safe copy fallback (console window classes).
  - Microphone privacy check from the privacy settings' registry values (only an explicit "Deny" is reported).
- ⬜ Local text model on Windows: llama.cpp's Vulkan backend (`llama-cpp-2` feature `vulkan`) needs the Vulkan SDK at build time, so it isn't enabled; CPU only for now.

### 5. Identity, distribution and trust → M6 beta
- ✅ **App icon:** a hand-drawn vector capsule under a parachute over a night ocean (`app/src-tauri/icons/source/`), with menu bar template glyphs at 1× and 2×.
- 🟡 **Name and trademark:** renamed to Splashdown. A search found no live SPLASHDOWN mark for software (classes 9/42); the dead ones were video games. `splashdown.dev` is an unrelated developer CLI called `splash`, so ours was renamed from `splash` to `splashdown-cli`. ⬜ Register splashdown.app, and have counsel confirm before launch.
- ✅ **Mac:** `scripts/package-mac.sh` builds a universal (Apple Silicon + Intel) DMG with the hardened runtime, the audio-input entitlement and the usage descriptions, signed with Point Dynamics' Developer ID. `NOTARIZE=1` notarizes and staples both the app and the DMG; the first notarized build passed Gatekeeper on 2026-10-29.
- ⬜ **Windows:**
  - ✅ Code signing: Azure Artifact Signing, certificate "Point Dynamics, LLC" (2026-10-30).
  - MSI/NSIS installers for x64 and ARM64.
- ✅ **Auto-update** with signed manifests (Tauri updater, from GitHub Releases): a daily check (switchable), "Update to vX.Y.Z…" in the menu bar, and install only on request from About. Release steps are in CONTRIBUTING. ⬜ Not tried end to end until there's a release.
- 🟡 **Feedback without telemetry:** ✅ "Send feedback…" (About and the menu bar) opens a pre-filled GitHub issue with versions and providers, no keys or text. ⬜ Local crash logs.
- ⬜ **Public presence:** pick a license (MIT vs Apache-2.0), a landing page with a demo GIF, docs, and a privacy statement ("your text goes only to the provider you choose").
- 🟡 **Package managers:** a Homebrew cask in the `bhowerton/homebrew-tap` tap (passes `brew audit --strict --online`); the main homebrew-cask repository needs more stars first. ⬜ winget, once the installer is signed.
- 🟡 **CI and release builds** (2026-10-30): `release.yml` builds the notarized DMG and the Windows installer on a version tag and drafts a GitHub release with one `latest.json` for both; CI leaves an unsigned Windows test installer on every push to `main`. README download links use fixed file names under `releases/latest/download/`. v0.1.0 was published by hand with `gh`. The `release` environment exists (approval required; `v*` tags and `main` only) with its secrets, and a manual test run on 2026-10-30 built a notarized DMG and a Windows installer with update files for both: releases from tags are on. ✅ Windows code signing through Azure Artifact Signing as Point Dynamics, LLC (identity validated 2026-10-30), from the release workflow.

### 6. On-device AI
- ✅ **M5 Apple:** SpeechAnalyzer and Foundation Models through the Swift shim. Free and private on Apple Silicon with macOS 26. Wired into Settings, onboarding and "Automatic"; a question guard and prompt changes made the on-device cleanup safer. Still to do: test the weak-link fallback on macOS 13–15.
- 🟡 **M5b built-in local model:** the `splashdown-llm` helper (llama.cpp, prompt caching) and a catalog chosen by the cleanup eval (Qwen3 4B; Qwen3 4B compact for machines with less than 12 GB of memory, which beat Qwen3 1.7B, Granite, Ministral, SmolLM3 and Phi-4-mini there; Qwen3 1.7B kept for those who chose it), with a Settings option, download and "Automatic" support. Setup picks by installed memory (2026-11-01). Still to do: test on Windows (CPU and Vulkan).

### 7. Engineering quality (ongoing)
- ⬜ Resource budgets: near-zero idle CPU, unload Whisper and local models after an idle period, measure battery impact.
- ⬜ Security pass:
  - No transcript text in logs by default.
  - Clipboard-restore races.
  - Prompt-injection defences re-run on the eval after every prompt change.
- ⬜ Beta group, feedback form, known-issues list.

### 7b. Notes you own
Notes as Markdown files in a folder the user picks, replacing the scratchpad. Storage: `~/Documents/Splashdown Notes/` (Windows: `Documents\Splashdown Notes\`) by default, or any folder, such as an Obsidian vault, iCloud Drive or Dropbox. Versions and pins sit in a `.splashdown/` folder inside it; nothing goes in a database or a cloud.
- ✅ **N1 Notes window and storage** (2026-10-31; ⬜ still to try in the running app):
  - One `.md` file per note, with short front matter.
  - A list with search, pins and subfolder groups, and versions (50 per note) with Restore.
  - Changes made in other apps picked up by re-reading the folder while the window has focus.
  - Rename, Show in Finder, and Move to Trash.
  - The scratchpad's notes are moved in, and Send to {app} is kept.
  - A folder picker that offers to move the notes.
  - A fallback folder when macOS refuses Documents access.
  - A synced-folder notice on the Privacy tab.
  - 10 unit tests.
- ✅ **N2 Transforms** (2026-10-31; ⬜ still to try in the running app):
  - Clean up, Summary, To-dos, Email, Prompt for an AI, plus your own, edited on Settings → Words.
  - In Notes they run on the note or a selection, keeping a version first, and ⌘Z undoes them.
  - A transform with a shortcut works on selected text in any app, or else on the last dictation.
  - 5 eval cases, all passing on the default and built-in local models.
  - To-dos answers `NONE` when there are no tasks, rather than inventing some.
- ✅ **N3 Capture without a window** (2026-10-31; ⬜ still to try in the running app):
  - Hold the dictation key and press N to add to Inbox, today's daily note or the last note opened. ⌃⌥Z takes it back.
  - `splashdown-cli note [text]` and `splashdown://note`.
  - History's **Save to notes**.
  - Optionally, the recording is kept in `Attachments/`.
  - ⬜ The mini bar's "Dictate a note".
- ✅ **N4 Notes for agents** (2026-10-31): `search_notes`, `read_note` and `append_note` in `splashdown-mcp`, through the control socket. Off by default, with separate switches for reading and writing on the Privacy tab.
- ✅ **N6 The Notes window, redesigned** (2026-11-01; ⬜ still to try in the running app and on Windows):
  - Record meeting, Transcribe (a file or a link) and Ask in the toolbar; ⌘K for every note and action; the ⋯ menu keeps only Rename, Show in Finder, Edit rewrites, Change folder and Trash.
  - A live-preview Markdown editor (CodeMirror 6, bundled from `app/editor` into `app/ui/vendor/editor.js`): marks hide off the cursor's line, bullets draw, `- [ ]` is a clickable checkbox; the file stays exactly what's typed.
  - The list grouped by day, with each note's kind (meeting, file, link, dictation) and filters for pinned, meetings and transcripts.
  - Rewrites shown as a word diff (Myers) with Original and each try, Keep / Keep the original, and a typed instruction to try again; **Say how…** for a one-off instruction (`notes_transform`'s `extra`).
  - Ask in Notes (`notes_ask`): this note or all notes (5 best matches), answers citing notes as `[n]` that open them, follow-ups.
  - Meeting notes as Summary / Transcript / My notes / Whole note tabs, parsed from the one Markdown file; a player for the kept recording (`notes-media://`, served in ranges from `Attachments/` only) with clickable turn times; Summarize again.
  - While a meeting records: the live transcript beside a My notes editor written under `## My notes` (`meeting_set_notes`, `Store::set_section`), fed to the summary; What did I miss?
  - An empty note shows ways to fill it; a link pasted into one offers to transcribe it; **Paste into {app}** replaces Send to.
- ✅ **N8 Folders and tags** (2026-11-01; ⬜ to try in the running app and on Windows): a third column (library views, the folder tree, the tag tree, with counts and folding remembered); folders are the notes folder's real subfolders (`Store::folders`, `create_folder`, `move_folder` with pins, the last note and path-keyed versions following, `delete_folder` to the Trash, `move_note`); tags are Obsidian's (`tags:` in front matter, `#tag` in text outside code, nested with `/`; `Summary::tags`, `Note::front_tags`, `set_front_tags`), shown as chips above the note and highlighted in the editor. Notes and folders are dragged with the pointer rather than HTML drag and drop, which the window's file-drop handler takes over (on macOS too). A new note goes into the folder (or gets the tag) being looked at.
- ✅ **N7 Transcribe an app's audio** (2026-11-01; ⬜ to try with speech playing, and on Windows): Transcribe → An app's audio… lists the apps that play sound (playing ones first; helper processes count as their app, WebKit's as Safari's) and records only the chosen one into a note as it plays, through the meeting recorder with one side and no microphone (`meeting::Source::App`, `source: app-audio`, top of the notes folder, summarized with Summary). macOS: Core Audio process objects (`splashdown_platform::audio_apps`) and the shim's tap on those processes (`CATapDescription(stereoMixdownOfProcesses:)`). Windows: audio sessions on active speakers, grouped by executable, and process loopback of the app's main process tree (`app_loopback`, `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`, Windows 10 2004+). "All of this computer's sound" uses the meetings' tap. Transcribe → The microphone is the same one-sided recorder on the microphone alone (`Source::Microphone`, `source: recording`), for lectures and conversations in the room.
- 🟡 **N5 Meeting notes** (decided 2026-10-31): record a call with no bot as two sides, You (the microphone) and Them (the computer's sound), transcribed in pieces into a note, with a summary at the end. ✅ The two-sided transcript (turns in time order, You/Them paragraphs, speaker echo dropped), checked with `splashdown-cli meeting transcribe` on a stereo recording; ✅ the **Meeting summary** transform (eval: 4/4 on the cloud default, 3/4 on the built-in local model). ✅ Recording (2026-10-31): the microphone plus the computer's sound through a Core Audio process tap in the Swift shim (macOS 14.2+; cpal's loopback records a headset's microphone instead) and WASAPI loopback on Windows; `splashdown-cli meeting record`. ✅ In the app: ⌃⌥M, the menu bar (with the time beside the icon on macOS), Notes' ⋯ menu and `splashdown-cli meeting start|stop|status`; a note in `Meetings/` written a turn at a time; a bar in Notes with levels, Stop and "Still in a meeting?"; the summary at the top (asking once before a cloud AI); a spool that lets the next launch finish a meeting the app closed on; stopping after 10 quiet minutes or 4 hours; Settings → Notes → Meetings. ✅ Call detection (2026-11-01): when a call app (Zoom, Teams, Slack, FaceTime, Webex, Discord, WhatsApp, Skype, or a browser) has used the microphone for a few seconds, a small panel in the top right offers to record, without taking focus; when the recorded app has let go of the microphone for 20 seconds, it offers to stop. Core Audio's process objects on macOS 14+, the microphone privacy records on Windows. Nothing records until a click, and "Don't offer again" turns it off. Turns are transcribed beside the recording, so a busy speech model never holds up capture or Stop, and the computer's sound follows a change of output device mid-call. ✅ A meeting pill (2026-11-01): a small bar at the top of the screen with the time, both sides' levels, Note, Stop and "Keep recording", draggable and never taking focus; on by default on Windows (whose tray can't show the time), a setting on macOS. ⬜ Not yet tried in the running app or on Windows; a changeable shortcut; echo cancellation beyond the word filter.

### 7c. Other planned features
The working list of improvements after 0.1.1, in rough order. Mark items here as they land.

**Quick wins (2026-10-31; ⬜ still to try each in the running app):**
- ✅ **Keep recording briefly after release** (2026-10-31): 200 ms more audio after the key is let go (not after Esc), before the stop sound, so the last word isn't cut off (`AudioWorker::finish`).
- ✅ **New lines that don't send** (2026-10-31): when a preset types rather than pastes, line breaks are sent as Shift+Return, so chat apps don't send the message halfway through; terminals (a list in `splashdown-platform`) keep a plain Return. Pasted text was already safe.
- ✅ **Recent dictations as context** (2026-10-31): up to 3 dictations from the last 5 minutes into the same app go to the AI in an `<earlier>` block, held in memory only; a Privacy-tab switch, on by default. Skipped with field context on, and for Apple's on-device model, which repeats and obeys them. Eval: 3 new cases, all pass on the default model; the built-in local model never repeats or obeys them.
- ✅ **Quieter computer while dictating** (2026-10-31): General → "Other sounds while you speak": turn down to a quarter (the default since 2026-11-02, Siri-style; before, leave), mute, or leave, from just after the start sound until the recording ends. The volume is put back only if the user didn't change it meanwhile. CoreAudio on macOS, `IAudioEndpointVolume` on Windows (builds; untested on a PC).
- ✅ **Model-based silence detection** (2026-10-31): after the volume-based trim, a clip needs about 0.1 s that the `earshot` speech detector (pure Rust, 40 KB model, any language) takes for a voice, so fans, clicks and hums don't reach the speech model. The speech eval's clips all still pass (WER unchanged). Local Whisper output that only repeats its hints (3+ words) is treated as silence.
- ✅ **Silero speech detection** (2026-11-01): Silero VAD v6.2 (MIT, 885 KB, built into the app and run by transcribe.cpp on the CPU, about 5,000× real time) now decides where speech is: whether a recording has any, trimming (keeping whatever either Silero or loudness takes for speech, so whispered word starts survive), the pauses long dictations and files are cut at (or, in fast continuous speech, the longest gap between words), and meeting turns. `earshot` and loudness remain the fallback if it can't load. Measured (eval/README.md): quiet speech Nemotron 9.0% → 5.1% and Whisper turbo 8.5% → 6.2% WER; clean and noisy speech unchanged; a synthesized music clip that Whisper turned into "you" is now "no speech". `SPLASHDOWN_VAD=loudness` turns it off to compare.

**Next:**
- ✅ **Type the AI's output as it streams** (2026-11-01; ⬜ to try in the running app and on Windows): OpenAI-compatible APIs (OpenAI, OpenRouter, Ollama, LM Studio…) and Anthropic stream their reply, and whole words are typed into the field as they arrive (Settings → General → Type the AI's text as it's written, on by default). At the end the final text, after the guard, replacements and file tags, is compared with what was typed: the rest is typed, or a differing end is deleted and retyped. Not for transforms, notes, snippets (placeholders), terminals or password fields, and a reply the guard would already reject (too long, or wrapped in tags, a code fence or quotes) stops being typed. `splashdown-cli clean --stream` shows the pieces as they arrive. ⬜ The built-in local model and Apple's answer all at once.
- ✅ **Fast local speech models** (2026-10-31; ⬜ still to try in the running app and on Windows):
  - Local speech-to-text now runs on `transcribe-cpp`, replacing `whisper-rs`.
  - **Nemotron 3.5 streaming** (32 languages) is the default on Mac and Windows: it transcribes while you speak, so text is ready about 0.03 s after release (eval clips, M-series CPU), and it had the best English of the local models. It also feeds the live preview. ⬜ Check it keeps up on a Windows laptop's CPU; Parakeet v3 is the fallback there if not.
  - Also offered: Nemotron streaming (English), Parakeet v2/v3, and Whisper base.en, small.en and large-v3-turbo as GGUF.
  - Earlier Whisper downloads (`ggml-*.bin`) need downloading again in the new format.
  - ⬜ Stream with noise reduction or whisper mode on.
  - ⬜ Vulkan on Windows.
  - ⬜ Real-voice eval clips.
- ✅ **"Ask" panel** (2026-11-01; ⬜ to try in the running app and on Windows): hold the dictation key, press A (Settings → General → Hotkeys) and ask; or say a question as a spoken command (instruction key with nothing dictated), which the command interpreter now recognizes. The answer streams into a small panel with Copy and Insert (into the app you asked from); Esc closes it. Selected text is what the question is about, and parts of up to three notes that match the question's rarer words go along (Settings → Privacy, on by default), so "what did we decide about the launch?" can be answered from your notes. Uses the model for instructions when one is set. `splashdown-cli ask` answers from the command line. ✅ Follow-ups (2026-11-01): type one into the panel, or ask again by voice while it's open, and the last six questions and answers go along ("and in French?"). "Help me write" with nothing selected already works as a question.
- 🟡 **Voice actions (2026-11-02; ⬜ to try in the running app and on Windows):** a spoken command can now do something beyond the text field. The command interpreter answers `do`, and a second request picks one action from a fixed catalog (`splashdown_core::action`): open or switch apps, web search (or YouTube, Maps, Wikipedia, Amazon, images, news), websites by name, Splashdown's own actions (meetings, notes, presets, whisper mode, hands-free, retry), to-dos in today's note, media and volume, calendar events and reminders (EventKit on macOS; an `.ics` file and a to-do on Windows), email drafts (`mailto:`), and Shortcuts (macOS, confirmed unless always allowed). The model only picks an action and its parameters; Rust checks each one, and an app, site, Shortcut or preset named only in selected text is refused. Text selected while speaking is the action's material ("email this to Sam"). "Undo that" takes back an event, reminder or to-do. Settings → Actions (off by default) turns each on or off. Scored by `eval/commands.toml` and `eval/actions.toml` (eval/README.md). ⬜ Messages and contacts, the user's own actions (links, scripts, keys), MCP tools.
- ⬜ **Run cloud and local speech recognition together** when a local model is installed, and use the first good result, instead of waiting for the cloud to fail.
- ✅ **A complete CLI** (2026-10-31): `status [--json]`; `settings list`, `get`, `set`, `unset` and `path` by dotted key, refusing unknown keys and wrong kinds of value; `models use`; `history [words] [--json]`; `words import`; `meeting record`, `transcribe`, `start`, `stop` and `status`; exit codes 0/1/2, 3 for the app not running, 4 for no such setting or model. A running app picks up changes at once.
- ✅ **Import vocabulary and snippets** from other dictation apps (2026-10-31; the Settings button 2026-11-01): `splashdown-cli words import <file>` reads a dictionary export (vocabulary and replacements), a settings file's `custom_words`, snippets as JSON (`[{name, text}]`) and word lists (text or CSV, with `misspelling,correction` lines), merges without changing anything already there, and tells the running app to reload. Settings → Words & language → Import… does the same after a confirmation, and the website says you can bring your words.
- ✅ **File transcription** (2026-10-31; ⬜ still to try in the running app and on Windows): drop audio or video files onto Notes, or pick them from its ⋯ menu, and each becomes a transcript note with a timestamp per paragraph, written as it goes, with progress and Stop. Reads WAV, AIFF, MP3, M4A, MP4/MOV, FLAC, OGG Vorbis and WebM/MKV with a pure-Rust decoder; `splashdown-cli transcribe-file`. ✅ Links (2026-11-01): Notes → ⋯ → Transcribe a link… downloads a link to an audio or video file (up to 2 GB), or a page's audio with yt-dlp when it's installed (YouTube and the like; found on the PATH or where Homebrew and winget put it), transcribes it into a note that links back, then deletes the download. yt-dlp is asked for AAC (m4a) audio first, since the decoder can't read Opus. ✅ No Homebrew needed (2026-11-01): the first page link offers to download the link helper, the official standalone yt-dlp (latest release, checked against its SHA2-256SUMS) and QuickJS-NG v0.17.0 (pinned SHA-256) as the JavaScript runtime YouTube now requires, into the models folder; yt-dlp updates itself weekly, and right away when a site refuses a download (YouTube's 403 to an old copy), then retries. Tried on a YouTube Short. ⬜ Opus, speaker labels, subtitle files.

**Later:**
- ⬜ **Multi-turn voice editing:** dictate a draft, keep speaking changes against a preview, and insert it when done (pairs with Notes transforms).
- ⬜ **Structured edits:** "change every Tuesday to Thursday" becomes find-and-replace operations applied exactly, rather than a rewrite.
- ⬜ **Vocabulary that only suggests a word after confirming the user's edit replaced Splashdown's own insertion,** plus starter word lists by field.
- ⬜ **Mic choice:** follow the system default live, never pick virtual or aggregate inputs automatically, and notice a closed laptop lid.
- ⬜ **Phone as a microphone** over the local network.
- ✅ **Shareable presets** as files (2026-11-01): Share… on a preset saves `Splashdown preset <name>.toml` to Downloads (without its shortcut); New preset → Import a preset file… adds one, renamed if the name is taken. ⬜ A gallery in GitHub Discussions.
- ✅ **Pin history entries** (2026-11-01): Pin in Settings → History keeps an entry at the top and out of "Delete after" and the 300-entry limit.
- ✅ **Never lose a dictation** (2026-11-01): each recording is on disk while it's processed and deleted once the text is in. If speech-to-text or the AI fails, or the app closes first, it's kept (the latest failure only; a dictation that works doesn't replace it), the error says so, and **Retry last dictation** in the menu bar runs it again with its preset and copies the text. ⬜ A crash during the recording itself (before release) still loses it.
- ⬜ **A shareable Insights card.**

### 8. Later, to stand out
- ✅ **MCP voice input for coding agents:** Claude Code, Cursor or Codex can ask the user to answer by voice. `splashdown-mcp`, set up as in [AGENTS.md](AGENTS.md).
- ✅ **Whisper mode** for quiet speech: an opt-in switch (General, or by voice) that raises quiet recordings to a normal level before transcription (up to 25×, softly limited) and lowers hands-free's "still speaking" level. A recording at about −40 dB went from "no speech found" to a word-perfect transcript.
- 🟡 **Computer use (exploration, 2026-11-02):** `splashdown-cli computer "<task>"` on macOS has Claude do a task on screen through Anthropic's computer toolset, with Splashdown's own checks before every action: apps it never touches, an allow-list, a yes before clicking anything labelled Send, Delete, Buy or Pay, nothing typed into password fields, the mouse pausing it, and limits on steps, time and cost. `eval/computer-tasks.toml` is the 15-task spike set. `--driver cua` drives the screen with [Cua Driver](https://github.com/trycua/cua) (MIT, Rust, embedded in-process, clicks in the background without taking the pointer) instead of Splashdown's own CGEvent driver, to compare the two. ⬜ Run the spike on both drivers; decide whether it goes in the app, and with which models (any vision model plus a grounding model is the next step).
- ⬜ **Linux** (the architecture allows it).
- ⬜ Localize the app's own interface.

### Not planned for 1.0
Mobile apps, enterprise compliance (SOC 2/HIPAA/SSO), screen OCR context, our own speech model. Each is a separate product or needs servers we don't run; choosing among providers matters more than a model of our own.
