# Privacy

Splashdown has no servers, no accounts and no analytics. This page lists everything that leaves your computer and everything it keeps on it. Settings → Privacy shows the same thing for your current settings.

## What leaves your computer

Only these, and only when you can see why:

| What | Where it goes | When |
|---|---|---|
| Your recording | The speech-to-text provider you chose (OpenAI or OpenRouter) | Each dictation, if speech-to-text is set to a cloud provider. With Apple's speech model or local Whisper it stays on your computer |
| A meeting you record: your microphone and what your computer plays | The same speech-to-text provider | Only while you record a meeting (⌃⌥M), and only if speech-to-text is set to a cloud provider. On macOS, recording what your computer plays needs your permission the first time |
| A recorded meeting's transcript, and what you wrote under My notes during it | The AI provider that runs transforms | Only for its summary: with a cloud AI, Splashdown asks first (Settings → Meetings), and when you click Summarize again |
| What an app plays, when you transcribe its audio in Notes | The same speech-to-text provider | Only while you transcribe it, and only if speech-to-text is set to a cloud provider. Only that app's sound is recorded, never the microphone |
| Your microphone, when you transcribe it in Notes (Transcribe → The microphone) | The same speech-to-text provider | Only while that recording runs, and only if speech-to-text is set to a cloud provider |
| The sound of an audio or video file you transcribe in Notes | The same speech-to-text provider | Only when you drop or pick a file to transcribe, and only if speech-to-text is set to a cloud provider. The file itself isn't copied or moved |
| The sound of new audio and video files in a folder you watch (Settings → Notes → Transcribe new files in a folder) | The same speech-to-text provider | Only for files that arrive after you add the folder (or the ones already there, if you click to transcribe them), and only if speech-to-text is set to a cloud provider. Subtitles you ask for are written beside the file, nowhere else |
| The transcript, plus context (the app's name unless you turn that off in Settings → Privacy, selected text you're rewriting or running a transform on, what you dictated into the same app in the last few minutes unless you turn that off, up to 30 names and terms from the window you're typing in (read on your computer; only those words are sent) unless you turn that off, and, only if the preset turns them on, the text around the cursor or your clipboard) | The AI provider you chose (OpenAI, OpenRouter, Anthropic, or a server you run) | Each dictation, if AI cleanup is on and set to a cloud provider. With Apple's model or the built-in local model it stays on your computer |
| A question you ask (dictation key + A), plus selected text if any, parts of up to three of your notes that match it unless you turn that off in Settings → Privacy, the app's name unless you turn that off, and, only if the preset turns them on, the text around the cursor or your clipboard | The AI provider you chose | Only when you ask a question. With Apple's model or the built-in local model it stays on your computer |
| What you asked to have written (after the instruction key), plus the text you selected, the text of the field you're typing in, the text around it in the window (the conversation you're answering, read with nothing selected, unless you turn that off in Settings → Privacy), parts of up to three of your notes that match, the description of your writing style for that kind of app if one has been learned, the app's name unless you turn that off, your name, and your clipboard only if the preset turns it on. Sentences in that text that give an AI instructions are left out first | The AI provider you chose | Only when you ask for a draft, and for each change to it. With Apple's model or the built-in local model it stays on your computer |
| A question you ask in Notes (⌘J), with the note you're asking about, or parts of up to five of your notes that match the question, and the conversation's earlier questions and answers | The AI provider you chose | Only when you ask in Notes. With Apple's model or the built-in local model it stays on your computer |
| A note, or the selected part of it, that you rewrite in Notes, and what you asked to change | The AI provider that runs transforms | Only when you click a rewrite or Try again |
| A check for a newer version | GitHub Releases | At launch and once a day. Turn it off in Settings → General. Nothing about you or your dictation is sent. The new version itself is downloaded when you click Install, or by itself if you chose that in Settings → General → Updates |
| Model downloads | Hugging Face (speech models such as Nemotron, Parakeet and Whisper, and the built-in model), GitHub (the voice model that recognizes people you've named, from the sherpa-onnx project) or Apple | Only when you choose to download one |
| A link you transcribe in Notes | The site it points to (YouTube, a podcast host), which sends the audio | Only when you transcribe a link. For a page such as YouTube, the link helper (yt-dlp and QuickJS) is downloaded from GitHub the first time, after asking, and yt-dlp checks GitHub for an update about once a week when it's used |
| What a hook you added gets: a dictation's text, the app and preset, a finished meeting's or transcript's note, a note from your iPhone, for a hook on a schedule, the notes of the last day or 7 days except ones hidden from coding agents, or, for a hook before a meeting, the calendar event about to start (its title, time and who's invited) and your notes of the last meetings with that title or those people, except ones hidden from coding agents (Settings → Actions → When something happens) | The web address you gave it, signed with your webhook secret; the AI that follows your instructions, for a hook that asks the AI (your cloud AI provider if you chose one, otherwise nothing leaves this computer; Settings says which when you add the hook, and each hook asks once before its text goes to a cloud provider, and again if the provider changes); or nothing, for a hook that runs a script or program on your computer (what that program does with it is up to it) | Only for hooks you added, when what they're for happens. None by default |
| Feedback | A GitHub issue page in your browser | Only when you choose "Send feedback". It's pre-filled with versions and provider names; you see it before submitting |

If you turn on Find notes by meaning (Settings → Notes), the Qwen3 Embedding model is downloaded from Hugging Face once, and your notes are read by it on your computer. What it makes of them (a list of numbers per paragraph) is kept in Splashdown's cache folder, not beside your notes, and never leaves your computer.

If you turn on Reply in the conversation's language (Settings → Words & language), the language of the conversation on screen is worked out on your computer from the same window text read for names on screen. Nothing more is read or sent: your dictation goes to the AI as always, asked to write it in that language.

If you turn on Recognize people you've named (Settings → Meetings, macOS only), a voiceprint is made on your computer for each speaker in a meeting once speakers are told apart: a list of numbers from their voice, not a recording. A voiceprint is kept only for people you name. Voiceprints are other people's biometric data, which some places regulate (Illinois's BIPA, the EU's GDPR), so ask people first if you're unsure. They never leave your computer and are never written into notes.

If you turn on Name meetings from my calendar (Settings → Meetings, macOS only), Splashdown reads the calendar event on when a meeting starts, its title and who's invited, to name the note. It's read on your Mac and only goes where the meeting's transcript goes.

If you turn on Offer to join and record meetings on my calendar (Settings → Meetings, macOS only, off by default), Splashdown looks at your calendar every half minute while it's on, for the events that start in the next minutes: their titles, times, who's invited, whether you declined, and their link, location and notes, which are only searched for a link to a video call. macOS asks you first. It's read on your Mac, and nothing about it is sent anywhere. Join opens the event's own link in your browser or call app, and only an `https` link to a video-call service Splashdown knows (Zoom, Google Meet, Teams, Webex, GoTo Meeting, Whereby, Jitsi Meet): an invitation's text is written by whoever sent it, so no other link in it is ever opened. If you click Record, the event's title and who's invited name the note, and go only where the meeting's transcript goes. With the switch off the calendar isn't looked at for this.

If a hook puts what the AI wrote in Reminders (macOS only) or in an email draft, that text goes to your Reminders or your mail app, and from there wherever those sync or send: Splashdown adds the reminders and opens the draft, and sends nothing itself. macOS asks before Splashdown may add to Reminders. A draft is only to addresses you typed in the hook; none is taken from a meeting.

If you add a hook before a meeting (Settings → Actions → When something happens, macOS only), Splashdown looks at your calendar every half minute while that hook is on, for the events that start in the next minutes: their titles, times, who's invited and whether you declined. macOS asks you first. It's read on your Mac, and an event's title, time and people go only where that hook sends them (the table above). Without such a hook the calendar isn't looked at for this.

To offer to record a call, Splashdown checks every two seconds which apps are using the microphone (Settings → Meetings). Only the apps' names are read, nothing leaves your computer, and nothing is recorded until you click Record.

If you allow coding agents to read your notes (Settings → Privacy, off by default), what an agent reads goes to that agent's own AI provider. Meetings and transcripts, which hold other people's words, have a switch of their own, and a note marked Hide from coding agents (its ⋯ menu) is never reached.

If you allow coding agents to search your dictation history (Settings → Privacy, off by default, and a switch apart from the notes one), the dictations an agent finds go to that agent's own AI provider: for each, when it was, the app, the preset and the text that was inserted. Never what was heard before cleanup, a spoken instruction or a recording. Only when an agent you set up searches it (agents are told to do that only when you ask about something you dictated), and only while history is kept; with history off there is nothing to search. History holds everything you dictated anywhere, so think of what you've said into messages and forms before turning this on.

Each provider handles what it receives under its own privacy policy. Local-only mode (Settings → Privacy) refuses every cloud provider, even when a key is saved.

## What stays on your computer

- **Settings:** `config.toml` in Splashdown's settings folder.
- **API keys:** the macOS Keychain or Windows Credential Manager, under "Splashdown". Never in files, never in exported settings.
- **History:** off by default. When on, the last 300 dictations (and, optionally, their recordings), deleted after the period you choose. It stays on your computer unless you let coding agents search it (above).
- **The recording being processed:** written to Splashdown's settings folder while it's turned into text, and deleted as soon as the text is in. If that fails (offline, a provider outage) or the app closes first, the one recording is kept for Retry (menu bar → Last dictation) until a retry works or a later dictation fails, whatever the history setting.
- **Insights:** counts only (words per day and per app), never what you said.
- **Spending:** cost totals per month, for the spending limit.
- **Writing style:** when Compose writes a draft into a reply and you change it before sending, the draft and what you sent (the last 20 for each kind of app: email, work chat, texts, everything else) in `style.json`, and the short description of your style written from them. Only the description is ever sent, with a draft. Settings → Words & language shows it, lets you edit it or forget it all, and turns learning off.
- **Learning your style, sent to the AI:** every few edited drafts, those drafts and what you sent go to the AI provider you chose, once, to write the description again (with a model on this computer, nothing leaves it).
- **Notes:** one Markdown file per note in `Documents/Splashdown Notes` (or the folder you choose), with earlier versions in a hidden `.splashdown` folder beside them. Nothing is sent anywhere unless you send or copy a note. If that folder is in iCloud Drive, Dropbox or OneDrive (Windows often backs up Documents to OneDrive), that service keeps a copy; Settings → Privacy says so. Settings → Notes → Keep them in moves them to a sync service only when you click it.
- **Notes from your iPhone:** the Splashdown Note shortcut turns speech into text with Apple's dictation on the phone and saves only the text, to your iCloud Drive (in the Shortcuts folder). Splashdown reads those files from iCloud Drive on your computer, adds them to your notes and deletes them. It never gets the recording, and sends nothing.
- **Voiceprints:** with Recognize people you've named on, `voices.json` (up to five voiceprints per person you named) and a `speakers` folder (each meeting's voiceprints, so naming someone later can learn from that meeting), in Splashdown's settings folder. Settings → Meetings → People known by voice forgets one person or everyone, with every meeting's voiceprints.
- **Watched folders:** `watched.json` in the settings folder lists the files already transcribed or left alone (their paths, sizes and times), so nothing is done twice.
- **Subtitle timings:** for a file transcribed on your computer, when each word was said, in a `subtitles` folder in the settings folder, so its subtitles can be timed to the words. Not beside your notes.
- **What hooks did:** `hook-runs.json` in the settings folder holds the last 50 runs: when, which hook, the meeting's title or the app, and what it did or what went wrong. Never the text. Settings → Actions clears it.
- **When scheduled hooks ran:** `hook-schedule.json` in the settings folder holds, for each hook on a schedule, the time it was last run for. Nothing else.
- **Which meetings hooks ran for:** `hook-calendar.json` in the settings folder holds, for each hook before a meeting, when it was added and a code and start time for each event of the last two days it ran for. No titles or names.
- **Which meetings were offered:** `calendar-offers.json` in the settings folder holds a code and start time for each calendar event of the last two days that the offer to join was shown for, so none is offered twice. No titles, names or links.
- **Webhook secret:** `webhook-secret` in the settings folder, made the first time a hook sends to a web address. Settings → Actions copies it or makes a new one.
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
