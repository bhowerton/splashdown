# Privacy

Splashdown has no servers, no accounts and no analytics. This page lists everything that leaves your computer and everything it keeps on it. Settings → Privacy shows the same thing for your current settings.

## What leaves your computer

Only these, and only when you can see why:

| What | Where it goes | When |
|---|---|---|
| Your recording | The speech-to-text provider you chose (OpenAI or OpenRouter) | Each dictation, if speech-to-text is set to a cloud provider. With Apple's speech model or local Whisper it stays on your computer |
| A meeting you record: your microphone and what your computer plays | The same speech-to-text provider | Only while you record a meeting (⌃⌥M), and only if speech-to-text is set to a cloud provider. On macOS, recording what your computer plays needs your permission the first time |
| A recorded meeting's transcript, and what you wrote under My notes during it | The AI provider that runs transforms | Only for its summary: with a cloud AI, Splashdown asks first (Settings → Notes → Meetings), and when you click Summarize again |
| What an app plays, when you transcribe its audio in Notes | The same speech-to-text provider | Only while you transcribe it, and only if speech-to-text is set to a cloud provider. Only that app's sound is recorded, never the microphone |
| Your microphone, when you transcribe it in Notes (Transcribe → The microphone) | The same speech-to-text provider | Only while that recording runs, and only if speech-to-text is set to a cloud provider |
| The sound of an audio or video file you transcribe in Notes | The same speech-to-text provider | Only when you drop or pick a file to transcribe, and only if speech-to-text is set to a cloud provider. The file itself isn't copied or moved |
| The transcript, plus context (the app's name unless you turn that off in Settings → Privacy, selected text you're rewriting or running a transform on, what you dictated into the same app in the last few minutes unless you turn that off, and, only if the preset turns them on, the text around the cursor or your clipboard) | The AI provider you chose (OpenAI, OpenRouter, Anthropic, or a server you run) | Each dictation, if AI cleanup is on and set to a cloud provider. With Apple's model or the built-in local model it stays on your computer |
| A question you ask (dictation key + A), plus selected text if any, parts of up to three of your notes that match it unless you turn that off in Settings → Privacy, the app's name unless you turn that off, and, only if the preset turns them on, the text around the cursor or your clipboard | The AI provider you chose | Only when you ask a question. With Apple's model or the built-in local model it stays on your computer |
| A question you ask in Notes (⌘J), with the note you're asking about, or parts of up to five of your notes that match the question, and the conversation's earlier questions and answers | The AI provider you chose | Only when you ask in Notes. With Apple's model or the built-in local model it stays on your computer |
| A note, or the selected part of it, that you rewrite in Notes, and what you asked to change | The AI provider that runs transforms | Only when you click a rewrite or Try again |
| A check for a newer version | GitHub Releases | At launch and once a day. Turn it off in Settings → General. Nothing about you or your dictation is sent |
| Model downloads | Hugging Face (speech models such as Nemotron, Parakeet and Whisper, and the built-in model) or Apple | Only when you choose to download one |
| A link you transcribe in Notes | The site it points to (YouTube, a podcast host), which sends the audio | Only when you transcribe a link. For a page such as YouTube, the link helper (yt-dlp and QuickJS) is downloaded from GitHub the first time, after asking, and yt-dlp checks GitHub for an update about once a week when it's used |
| Feedback | A GitHub issue page in your browser | Only when you choose "Send feedback". It's pre-filled with versions and provider names; you see it before submitting |

If you turn on Find notes by meaning (Settings → Notes), the Qwen3 Embedding model is downloaded from Hugging Face once, and your notes are read by it on your computer. What it makes of them (a list of numbers per paragraph) is kept in Splashdown's cache folder, not beside your notes, and never leaves your computer.

If you turn on Name meetings from my calendar (Settings → Notes → Meetings, macOS only), Splashdown reads the calendar event on when a meeting starts, its title and who's invited, to name the note. It's read on your Mac and only goes where the meeting's transcript goes.

To offer to record a call, Splashdown checks every two seconds which apps are using the microphone (Settings → Notes → Meetings). Only the apps' names are read, nothing leaves your computer, and nothing is recorded until you click Record.

If you allow coding agents to read your notes (Settings → Privacy, off by default), what an agent reads goes to that agent's own AI provider.

Each provider handles what it receives under its own privacy policy. Local-only mode (Settings → Privacy) refuses every cloud provider, even when a key is saved.

## What stays on your computer

- **Settings:** `config.toml` in Splashdown's settings folder.
- **API keys:** the macOS Keychain or Windows Credential Manager, under "Splashdown". Never in files, never in exported settings.
- **History:** off by default. When on, the last 300 dictations (and, optionally, their recordings), deleted after the period you choose.
- **The recording being processed:** written to Splashdown's settings folder while it's turned into text, and deleted as soon as the text is in. If that fails (offline, a provider outage) or the app closes first, the one recording is kept for Retry (menu bar → Last dictation) until a retry works or a later dictation fails, whatever the history setting.
- **Insights:** counts only (words per day and per app), never what you said.
- **Spending:** cost totals per month, for the spending limit.
- **Notes:** one Markdown file per note in `Documents/Splashdown Notes` (or the folder you choose), with earlier versions in a hidden `.splashdown` folder beside them. Nothing is sent anywhere unless you send or copy a note. If that folder is in iCloud Drive, Dropbox or OneDrive (Windows often backs up Documents to OneDrive), that service keeps a copy; Settings → Privacy says so.
- **Logs:** timings, provider names and error messages. Never what you said or typed.
- **Models:** downloaded speech and AI models.

## Permissions

- **Mac:** Microphone (to hear you) and Accessibility (to read the focused field, see selected text and type). Splashdown only reads a field while you're dictating into it, and never reads password fields.
- **Windows:** no permission prompts. Splashdown can't type into apps running as administrator; it copies the text instead.

## Deleting everything

Uninstall the app, then remove these:

**Mac**
- `~/Library/Application Support/Splashdown` (settings, history, Insights)
- Your notes folder (`~/Documents/Splashdown Notes` unless you chose another), if you want your notes gone too. Uninstalling never touches it.
- `~/Library/Caches/Splashdown` (models) and `~/Library/Caches/app.splashdown`
- `~/Library/Logs/Splashdown`
- `~/Library/WebKit/app.splashdown`
- Keychain Access: items named "Splashdown"

With Homebrew, `brew uninstall --zap --cask splashdown` removes the folders (not the keychain items).

**Windows**
- `%APPDATA%\Splashdown` (settings, history, Insights)
- Your notes folder (`Documents\Splashdown Notes` unless you chose another), if you want your notes gone too. Uninstalling never touches it.
- `%LOCALAPPDATA%\Splashdown` (models and logs)
- Credential Manager → Windows Credentials: entries named "Splashdown"
