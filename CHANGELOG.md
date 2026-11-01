# Changelog

What changed in each release of Splashdown. Downloads are on the [releases page](https://github.com/bhowerton/splashdown/releases).

## Unreleased

- **Today's note and smart views.** Today's note (⌘T) opens today's daily note in Notes, making it if it isn't there yet (the folder and name are the ones set for captures in Settings → Notes). Beside Pinned, the column has To do (notes with an unticked checkbox), Changed today and Untagged.

## 0.2.0 — 2026-11-01

- **A redesigned Notes window.** Record a meeting, Transcribe (a file or a link) and Ask sit in the toolbar instead of a menu, and ⌘K finds any note or anything the window can do. Notes show their formatting as you write: headings, bullets, and checkboxes you can click, while the file stays plain Markdown. The list shows what each note is (a meeting, a transcript, a dictation) and filters to pinned notes, meetings or transcripts; each note's ⋯ (or a right-click) pins, renames, shows or trashes it. A new, empty note shows the ways to fill it, and pasting a link into one offers to transcribe it. **Paste into {app}** (⌘↩) replaces Send to, and follows whichever app you used last. Notes is now an ordinary window; ⋯ → Keep on top floats it over other apps, as the scratchpad did.
- **Folders and tags in Notes.** A column beside the list has All notes, Pinned, Meetings & recordings and Transcripts, then your folders, nested as deep as you like, and your tags. Folders are real folders in your notes folder (Obsidian and Finder see the same ones): create, rename, nest and trash them from their ⋯ menu, and move a note by dragging it onto a folder or with Move to…. A new note goes into the folder you're looking at. Tags work the way Obsidian's do: write `#tag` anywhere in a note, or add one above it, nested with `/` (`#work/acme`); the tags column shows them as a tree with counts, and ⌘-click on a tag in a note shows its notes. ⌘K finds folders and tags too, and ⌘⇧E shows or hides the column.
- **Rewrites you can check before keeping.** Clean up, Email and your other transforms sit under the note. The result shows what changed, words taken out struck through and new ones highlighted, and you keep it, keep the original, or say what to change ("warmer", "in French") and try again. **Say how…** rewrites in your own words without a saved transform. Each try stays until you choose, and the note before is in Versions.
- **Ask your notes from Notes.** ⌘J opens Ask beside the note: ask about this note or all of them. Answers mark which notes they came from, and a click opens one. Follow-ups carry on the conversation; Add to this note puts an answer in.
- **Meeting notes in tabs: Summary, Transcript and My notes.** While a meeting records, Notes shows the transcript as it grows, with a place beside it to write your own notes; the summary covers what you wrote down first. **What did I miss?** answers from the last few minutes. Afterwards, a kept recording plays in the note: click a turn's time to hear it. Summarize again rewrites the summary.
- **Much faster local speech-to-text.** A new engine runs NVIDIA's Nemotron and Parakeet models as well as Whisper. The new default, Nemotron 3.5, transcribes while you speak in any of 32 languages, so your text is ready almost the moment you let go of the key, and it shows the words as you speak. Nemotron (English) is offered too, and Whisper large-v3-turbo is the most accurate for other languages. Models download when you choose them (Settings → Providers → Speech to text → On this computer), and a model is only loaded once you've chosen it; Splashdown says when it's still loading. The live words while you speak now need speech-to-text on your computer. A Whisper model downloaded before needs downloading again in the new format.
- **Text appears sooner with a cloud AI.** The AI's text is typed into the app as it's written, starting a moment after you let go of the key, instead of all at once at the end. If the final version differs (the AI went too far and your words are used instead), the end is corrected. Settings → General → Type the AI's text as it's written turns it off.
- **Ask a question and get an answer instead of typed text.** Hold the dictation key, press A and ask: "what's the German word for deadline?", or select text first and ask "what does this say?". The answer appears in a small panel with Copy and Insert. Answers can draw on your notes ("what did we decide about the launch?"); turn that off in Settings → Privacy. A question said after the instruction key, with nothing dictated, is answered the same way. Ask a follow-up by typing it into the panel, or by asking again while it's open. Also `splashdown-cli ask`.
- **Transcribe an app's audio.** Notes → Transcribe → An app's audio… lists the apps playing sound (a browser with a webinar, a podcast app, a video call you're only listening to) and transcribes what the chosen one plays into a note as it plays, with My notes beside it and a summary at the end. Only that app's sound is recorded, not the microphone or other apps, so headphones are fine. A level meter shows what it's getting, with "Hearing words", "Sound, no words yet" or "No sound from …" beside it. "All of this computer's sound" is there too. macOS 14.2 or later; on Windows, Windows 10 version 2004 or later.
- **Transcribe the microphone.** Notes → Transcribe → The microphone transcribes a lecture, a talk or a conversation in the room into a note as it's said, hands-free, as plain timed paragraphs (Record meeting would label everyone in the room "You"). Stop it from the note or the menu bar.
- **Transcribe a link.** Notes → Transcribe → A link… takes a link to an audio or video file, or a page such as YouTube. For pages, Splashdown offers to download a small helper the first time (yt-dlp and QuickJS, about 39 MB) and keeps it up to date; nothing else needs installing. The download is deleted once the note is written.
- **App icons in Settings on Windows**, next to each app in Presets, as on the Mac.
- **Share presets.** Share… on a preset saves it as a small file; anyone can add it with New preset → Import a preset file….
- **A second-choice microphone.** Settings → General: if your chosen microphone isn't connected, Splashdown uses the one you pick next (say, the laptop's own) before falling back to Automatic. Meetings follow the same choice.
- **Pin history entries** to keep them at the top and out of automatic deletion.
- **A dictation is never lost.** If speech-to-text or the AI fails (you're offline, a provider is down) or Splashdown closes before it's done, the recording is kept, and **Retry last dictation** in the menu bar turns it into text and copies it. Only that one recording is kept, even with history off.
- **Better at telling speech from everything else.** A small built-in speech detector (Silero) now decides where you're speaking, so quiet or whispered speech is no longer trimmed away, music and noise aren't turned into made-up words, and long recordings are cut between words. It works with every speech-to-text choice and runs on your computer.
- **Notes replace the scratchpad.** ⌃⌥N opens Notes. Each note is a plain Markdown file in `Documents/Splashdown Notes`, or any folder you choose, such as an Obsidian vault or iCloud Drive. Notes has search, pins, earlier versions you can restore, and Show in Finder. Deleted notes go to the Trash. Your scratchpad notes move in the first time you open it.
- **A recording bar for meetings.** While a meeting records, a small bar at the top of the screen shows the time and both sides' levels, with Stop. It's on by default on Windows; on a Mac the menu bar shows the time, and the bar can be turned on in Settings → Notes → Meetings (handy when the notch hides the menu bar icons).
- **Offers to record when a call starts.** When Zoom, Teams, Slack, FaceTime, Meet in a browser or another call app starts using the microphone, a small panel offers to record the meeting, and offers to stop when the call ends. Nothing is recorded until you click; Settings → Notes → Meetings turns it off.
- **Meeting notes.** Press ⌃⌥M (or Record a meeting in the menu bar or Notes) during a call. Splashdown records your microphone and what your computer plays, with no bot joining, and writes the conversation into a note in Meetings as it goes, each turn marked You or Them. When it ends, a summary of decisions, action items and open questions goes at the top (with a cloud AI, it asks first). If Splashdown closes mid-meeting, it offers to finish the note next time. Also `splashdown-cli meeting start`, `stop` and `status`.
- **Transcribe audio and video files.** Drop a voice memo, interview or video onto Notes (or use Transcribe → A file…) and it becomes a note, with the time in the file at the start of each paragraph. It uses the speech-to-text you dictate with, so with a model on your computer nothing leaves it. Reads MP3, M4A, WAV, FLAC, OGG, MP4 and MOV, among others. Also `splashdown-cli transcribe-file`.
- **Dictate straight into a note.** Hold the dictation key and press N: what you say is added to your Inbox (or today's daily note, or the last note opened) instead of typed. ⌃⌥Z takes it back. Also `splashdown-cli note`, `splashdown://note`, and **Save to notes** in History.
- **Notes for coding agents.** If you allow it (Settings → Privacy, off by default), agents set up with `splashdown-mcp` can search and read your notes, and add to them.
- **Transforms.** Saved instructions you run without speaking: Clean up, Summary, To-dos, Email, Prompt for an AI and Meeting summary, or your own (Settings → Words). In Notes they work on the note or the selected text, and ⌘Z or History undoes them. Give one a shortcut and it works on the selected text in any app.
- **A fuller command line.** `splashdown-cli status`, `settings get` and `set` (by key, like `general.language`), `models use` and `history` for scripts, with exit codes they can rely on.
- **Bring your words from another dictation app.** Settings → Words & language → Import… (or `splashdown-cli words import <file>`) adds the vocabulary, corrections and snippets from another app's export, a word list or a CSV file, without changing what you already have.
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
