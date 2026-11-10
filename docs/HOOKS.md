# Hooks

A hook does something when something happens in Splashdown: after a dictation, when a meeting or transcript is finished, when a note comes from your iPhone, at a time you set, with the notes of the day or week, or (on a Mac) a few minutes before a meeting on your calendar, with your notes from the last time. It can ask the AI to do something with the text in your own words ("draft the follow-up email"), run your own script or program, or send to a web address. One kind runs *before* a dictation goes in and changes its text.

Add them in **Settings → Actions → When something happens**. **Test** runs a hook once with sample text before you save it. **What ran** lists the last 50 runs: when, which hook, for which meeting, and what it did or what went wrong (never the text itself).

## When a hook runs

| When | Gets | Choose a preset? |
|---|---|---|
| **After a dictation** | The text that went into the field | Every preset, or one |
| **Before a dictation goes in** | The text about to go in; what the program writes back goes in instead | One preset (required) |
| **When a meeting ends** | The meeting's note (Markdown), its title, summary and file | No |
| **When a transcript is finished** | The transcript's note: a file, a link, a watched folder, an app's audio or the microphone | No |
| **When a note comes from your iPhone** | What you said, and when | No |
| **On a schedule** | The notes you made in the last day or the last 7 days, as one text | No |
| **Before a meeting on your calendar** (Mac) | The meeting about to start and your notes of the last meetings about it, as one text | No |

A dictation into a note (the note key) doesn't run dictation hooks. Neither does a rewrite of selected text.

A hook for meetings, transcripts or meetings on your calendar can be for some of them only: **Only when the title has** takes a word or two ("1:1", a client's name), matched in any capitals.

## On a schedule

A hook on a schedule runs at a time of day (this computer's clock), every day, on weekdays, or on one day of the week, and gets your notes from **the last day** (the 24 hours before) or **the last 7 days**. That makes a weekly review, a daily wrap-up, or a digest sent somewhere else.

What it gets, oldest first, each note under its title and a line saying what it is and when:

- **Meetings and transcripts** made in that time. One that has a summary comes with its summary and your own notes, without its transcript; one that has none comes whole.
- **The Inbox:** the lines added in that time, to the minute. Each day's lines sit under a heading for the day (`## 2026-11-10`), which Splashdown adds with the day's first line. Lines from before it did that have no day, so they're left out once the Inbox has any heading; an Inbox with none comes as its last few paragraphs.
- **Other notes that changed in that time**, such as a daily note or a note that collects another hook's results. These don't record when each line was added, so a long one comes as its last few paragraphs, which can include older lines.
- **Left out:** notes hidden from coding agents (`agents: false`), and the note the hook's own results go into.

If the computer is asleep or Splashdown isn't running at the time, the hook runs when it next can, once, with the notes up to the time it was due. A hook you just added, changed or turned back on waits for its next time. With no notes in that time, nothing runs, and What ran says so.

**Test** tries what you wrote on the notes it would get right now (or on a sample, when there are none) and shows the result without putting it anywhere.

## Before a meeting

A hook before a meeting runs a few minutes before an event on your calendar starts (10, unless you choose another number from 1 to 120), with what you need to walk in prepared: your notes from the last time you met. The ready-made **Brief** asks the AI what was decided, what you said you'd do and what others owe you, and collects the answers in a note called Briefs.

This needs a Mac: Splashdown reads the calendar through macOS, which asks you first, and only while a hook like this is on. On Windows the hook can't be added.

What it gets, as one text, each part under its title and a line saying what it is and when:

- **The meeting about to start:** its title, when it starts, and who else is invited.
- **Your notes of the last meetings about it,** newest first, three at most: meeting notes with the same title (a repeating meeting, whatever the capitals and punctuation), or with mostly the same people (at least half of those invited to either are in both, so a 1:1 finds your last 1:1 with that person and not every large meeting they were at). A note's people are the ones on its "With …" line and the ones named in its transcript. A note that has a summary comes with its summary and your own notes, without its transcript; one that has none comes whole.
- **Left out:** notes hidden from coding agents (`agents: false`), notes that aren't meetings, and this meeting's own note if you started recording early.

Meeting notes are found by title most easily when they're named from the calendar: turn on **Name meetings from my calendar** in Settings → General → Meetings, which also writes the "With …" line.

Each event runs a hook once. All-day events and ones you declined (or that were called off) are skipped. If the computer was asleep, the hook still runs up to 5 minutes after the meeting starts, and not after that. An event moved to later the same day isn't briefed again. A hook you just added, changed or turned back on starts with the meetings still ahead. With no earlier notes about a meeting, nothing runs, and What ran says so.

**Test** tries what you wrote on your next meeting in the coming 7 days that has earlier notes (or on a sample, when there is none) and shows the result without putting it anywhere.

## Asking the AI

No code needed: choose **Ask the AI to do something with it**, say what you want in your own words, and choose where what it writes goes. Some are ready to use under **Start from**, and can be changed:

| Start from | When | What it writes | Where |
|---|---|---|---|
| Follow-up email | A meeting ends | A short email to the others: thanks, what was decided, who does what | The meeting's note |
| Follow-up email, as a draft | A meeting ends | The same | A draft in your mail app, to no one until you add addresses |
| What I said I'd do | A meeting ends | Your own commitments, as to-dos | A note called Commitments |
| What I said I'd do, as reminders (Mac) | A meeting ends | The same, each line written to make sense on its own | Your Reminders |
| Decisions | A meeting ends | What was decided, a line each | A note called Decisions |
| Key points | A transcript is finished | The main points, with their times | The transcript's note |
| Study cards | A transcript is finished | Questions and answers to study from | The transcript's note |
| To-dos from my phone | A note comes from your iPhone | The things to do in it, if any | A note called To-dos |
| Weekly review | Fridays at 16:00, the last 7 days | What got done or decided, what you said you'd do that's still open, what's waiting on others | A note called Weekly reviews |
| Daily wrap-up | Weekdays at 17:30, the last day | Your to-dos from the day, then what was decided | A note called Daily wrap-ups |
| Brief | 10 minutes before a meeting on your calendar (Mac) | What was decided the last times, what you said you'd do, what others said they'd do | A note called Briefs |

Where what it writes can go:

| Put what it writes | What happens |
|---|---|
| **In the note** (meetings and transcripts) | A section named after the hook, above the note's summary and transcript. Running it again for the same note replaces that section |
| **At the end of another note** | Added to a note in your notes folder (made if it isn't there), under a heading with the meeting's title and the day (for a hook before a meeting, the meeting about to start and its day), or the days a scheduled hook covers. Results collect there, meeting after meeting |
| **At the end of a file** | The same, in any file you give the full path of |
| **On the clipboard** | Replaces what's on the clipboard, ready to paste |
| **To a web address** | Sent as a web address hook is (below), with `text` holding what the AI wrote instead of the note, and `ask` holding the hook's name |
| **In Reminders** (Mac) | A reminder for each to-do it wrote, in a list you name or your default list. See below |
| **In an email draft** | Opens as a new email in your mail app, for you to check and send. It is never sent for you. See below |

If the AI finds nothing to do (no to-dos in a note), nothing is put anywhere, and What ran says so. **Test** asks about a short sample meeting and shows what it would write, without putting it anywhere. For Reminders it also lists the reminders it would add, and for an email draft who it would be to and its subject; it adds and opens nothing.

### Reminders

Each to-do line in what the AI wrote (`- [ ] …`) becomes one reminder. If it wrote no to-do lines at all, each plain list line (`- …`, `1. …`) does. Everything else is left out: headings, sentences, and to-dos already ticked off (`- [x]`). With nothing to add, What ran says so.

- **Which list:** the one you name under **List** (in any capitals), or your default list in Reminders when you leave it empty. A list you name has to be there already: Splashdown doesn't make one, so a typing slip is caught rather than starting a new list. Settings checks when you save the hook, and each run checks again before the AI is asked. In `config.toml` it's `to = "Work"`.
- **Where it came from** is in the reminder's notes: the meeting's or note's title and the day, and the hook's name.
- **Due dates:** a reminder is due only if its line has one date written as `2026-11-12`, today or later. With a time on the 24-hour clock right after it (`2026-11-12 15:00`) it's due then, with an alert; without, that day, with no alert. A day in words ("by Wednesday", "tomorrow") stays in the reminder's text and sets no date: working out which day was meant could get it wrong. The ready-made hooks keep days as they were said, so their reminders have none.
- **Asking first:** if **Ask before adding events and reminders** is on (Settings → Actions → Choices, shown when voice actions are on), Splashdown shows the to-dos and asks before adding them, as it does for a spoken "remind me to…". Otherwise they're added at once. To take one back, delete it in Reminders.
- **Limits:** at most 25 reminders a run (What ran says when there were more), and a long line is cut at 200 characters.

This needs a Mac, where Splashdown can add to Reminders. macOS asks you to allow it when you save the hook; if you say no, the hook fails and tells you where to allow it (System Settings → Privacy & Security → Reminders). On Windows the choice can't be saved, and a hook that has it in `config.toml` doesn't run or ask the AI.

### An email draft

What the AI wrote opens as a new email in your mail app (the one your computer opens email links with), with the hook's name and the meeting's or note's title as its subject ("Follow-up email: Pricing review"). You read it, change it and send it, or close it. Splashdown never sends it. Marks for headings, bold and to-do boxes are taken out, since mail would show them as they are.

- **Who it's to:** only the addresses you type in the hook's **To** field (`to` in `config.toml`), separated by commas. Left empty, the draft is to no one and you fill it in. No address is ever taken from what was said in the meeting or from what the AI wrote: those only ever become the email's text.
- **Long results:** a draft is opened through a link, and a link can only be so long: about 8,000 characters on a Mac and 2,000 on Windows once the text is encoded, which is roughly 1,000 words and 250 words of English, and fewer in other alphabets. A result longer than that goes on the clipboard instead, the draft opens saying to paste it, and What ran says so.
- The draft opens in front of what you're doing, when the hook runs.

### Being told, and running one now

A hook that asks the AI works in the background and puts what it writes in a note, where you might not look. With **Tell me when it's done** on, a small panel at the top right says the hook has written something, shows how it starts, and has **Open** when it went into a note. The panel stays for five minutes unless you answer it, and makes way at once for a question Splashdown needs answered. The ready-made weekly review, daily wrap-up and brief have it on. In `config.toml` it's `notify = true`.

A hook on a schedule or before a meeting needn't wait for its time:

- **Run now**, next to it in Settings, runs it on the notes it would get at this moment: the last day or 7 days up to now, or your next meeting on the calendar that has earlier notes.
- **By voice**, the way your own actions are run: hold the dictation key, tap the instruction key and say "run my weekly review". It goes by the hook's name, for hooks that ask the AI. One of your own actions with the same name wins.

Either way the result goes where the hook says, and you're told when it's done. A scheduled hook that adds to another note and is run again for the same days writes over what it wrote for them before, so the note says it once.

### Which AI, and long meetings

The text goes to the AI that follows your instructions (Settings → AI Models), and the editor says which one that is: the one on this computer, or a cloud provider. In local-only mode it's always the one on this computer.

A cloud AI is agreed to once for each hook. Saving a hook while a cloud AI follows your instructions asks first, saying what will go there each time it runs. A hook saved with the AI on this computer asks at its next run if you later choose a cloud AI, and again if you change to another provider. Say no and the hook doesn't run. In Settings, Test on a scheduled hook or one before a meeting uses your own notes, so it asks before sending those too; the others test on a made-up sample. In `config.toml` the agreement is `cloud_ok = "OpenRouter"`, the provider's name.

The AI on this computer reads about ten minutes of talk at a time. A longer meeting is read a part at a time, with notes taken from each part for what you asked, and your instruction is then run on the notes. That works, and takes a request per part: about a quarter of a minute for an hour-long meeting on a recent Mac, longer on a computer without a graphics chip the model can use. The meeting summary is written the same way.

What to expect from the AI on this computer (a small model):

- **Good at finding things that were said:** to-dos, decisions, who agreed to what. It sometimes credits a task to the wrong person, so read the list.
- **Weaker at writing from a whole long meeting:** a follow-up email for an hour-long meeting can leave things out.
- **A weekly review needs exact wording:** told plainly which headings to write and what goes under each (as the ready-made one is), it finds most decisions and to-dos in a week of notes and names the note each came from. It can miss one or put it under the wrong heading. Asked loosely ("review my week"), it makes up dates and mixes up who does what.

- **A brief before a meeting needs exact wording too:** with the ready-made one it lists the decisions and to-dos of the last meetings and says which meeting each is from. From notes with summaries it finds nearly all of them. From a transcript alone it can call something decided that was only asked about, or mix up who something is for. It lists everything the notes say is to do, including what you have done since without saying so in a meeting.

A cloud model reads the whole meeting at once and answers in about a second. A small one, like the default, is about as accurate as the AI on this computer, with different slips; a larger one does noticeably better. The scores are in [eval/README.md](../eval/README.md). Either way, check what it wrote before you send it on.

### What it can't do

The AI only writes. It can't send, open, delete or run anything, and it can't choose where its text goes: you did. A meeting is mostly other people's words, and those words can't give it orders; the worst a transcript can do is change what gets written. It doesn't run before a dictation goes in (a preset's own instructions do that).

Reminders and an email draft keep to that. Splashdown does one thing with the text, the one you chose in Settings: a reminder for each to-do line, or a draft that isn't sent. Words in a meeting can change what a reminder or a draft says. They can't add a recipient, send the email, set anything but a reminder's text and day, or make anything else happen.

In `config.toml`:

```toml
[[hooks]]
when = "meeting"
kind = "ask"
name = "My commitments"
instruction = "List everything I said I would do, as to-dos."
put = "other-note"   # note, other-note, file, clipboard, web, reminders (a Mac only) or email-draft
to = "Commitments"   # the note, the file's full path, the web address, or an email draft's addresses (optional)
title_has = "1:1"    # optional

[[hooks]]
when = "schedule"
kind = "ask"
name = "Weekly review"
instruction = "Write a review of my week from these notes."
put = "other-note"
to = "Weekly reviews"
# days: mon tue wed thu fri sat sun (none is every day); covers: day or week
schedule = { at = "16:00", days = ["fri"], covers = "week" }

[[hooks]]
when = "before-meeting"   # a Mac only
kind = "ask"
name = "Brief"
instruction = "Write a brief for the meeting that is about to start, from the notes of the earlier meetings."
put = "other-note"
to = "Briefs"
title_has = "1:1"         # optional: only events with this in their title
before = { minutes = 10 } # optional: 10 unless you say, from 1 to 120
```

## A script or program

Splashdown starts it directly, never through a shell, with the arguments you gave it, one per line. It gets:

- **The text on its standard input:** the dictation, the note from your iPhone, the whole note (Markdown) for a meeting or transcript, the notes of the day or week for a hook on a schedule, or the meeting about to start and your earlier notes about it for a hook before a meeting.
- **These environment variables:**

| Variable | Holds |
|---|---|
| `SPLASHDOWN_EVENT` | `dictation`, `before-insert`, `meeting`, `transcript`, `phone-note`, `schedule` or `before-meeting` |
| `SPLASHDOWN_AT` | When it happened, as `2026-11-09T14:03:22-05:00` (for a note from the iPhone, when you said it; for a schedule, the time it was due; before a meeting, when the hook ran) |
| `SPLASHDOWN_SINCE` | Where the notes start, in the same form (schedules only): they run from then until `SPLASHDOWN_AT` |
| `SPLASHDOWN_APP` | The app dictated into (dictations only) |
| `SPLASHDOWN_PRESET` | The preset's name (dictations only) |
| `SPLASHDOWN_TITLE` | The note's title (meetings and transcripts), or the title of the meeting about to start |
| `SPLASHDOWN_STARTS` | When the meeting starts, in the same form as `SPLASHDOWN_AT` (before a meeting only) |
| `SPLASHDOWN_PEOPLE` | The other people invited, by name or address, separated by `, ` (before a meeting only; empty when there are none) |
| `SPLASHDOWN_NOTE` | The note's file (meetings and transcripts) |
| `SPLASHDOWN_TEST` | `1` when run by Test |

Hooks run one at a time, in the background: nothing waits for them. One that runs longer than 60 seconds is stopped. If it fails, Splashdown shows the first line it wrote to standard error.

A script is run the way your system runs it: on a Mac, an executable file with a `#!` line (or a `.sh` file through `/bin/sh`); on Windows, `.ps1` through PowerShell, `.bat` and `.cmd` through cmd, and programs directly.

### Changing a dictation before it goes in

For **Before a dictation goes in**, what the program writes to standard output is inserted instead of the dictation (line breaks at the end are dropped). If it takes longer than **2 seconds**, fails, or writes nothing, the dictation goes in as you said it and Splashdown tells you why. With a hook like this, that preset's text isn't typed as it streams in, since it has to wait for the hook.

```sh
#!/bin/sh
# Spell the product names right, whatever was heard.
perl -pe 's/\bsplash down\b/Splashdown/gi; s/\breact\b/React/g'
```

## A web address

Splashdown sends a `POST` with this JSON:

```json
{
  "event": "meeting",
  "at": "2026-11-09T14:03:22-05:00",
  "text": "# Weekly sync\n\n## Summary\n\n…",
  "app": null,
  "preset": null,
  "note": {
    "title": "Weekly sync",
    "path": "/Users/you/Documents/Splashdown Notes/Meetings/2026-11-09 Weekly sync.md",
    "summary": "…"
  }
}
```

For a dictation, `app` and `preset` are filled and `note` is `null`. A hook on a schedule has the notes in `text`, the time it was due in `at`, and adds `"since"` with where the notes start. A hook before a meeting has the meeting and your earlier notes in `text`, and adds `"meeting"`: `{"title": "Pricing review", "starts": "2026-11-17T14:00:00-05:00", "people": ["Dana Lee", "Sam Reyes"]}`. Test adds `"test": true`. A hook that asks the AI and sends what it wrote here adds `"ask"` with the hook's name, and its `text` is what the AI wrote.

If the server can't be reached, doesn't answer within 15 seconds, or answers with a 5xx error, it's sent once more, 2 seconds later.

### Checking it came from you

Each request has these headers:

| Header | Holds |
|---|---|
| `X-Splashdown-Event` | The event, as in the JSON |
| `X-Splashdown-Timestamp` | When it was sent, in seconds since 1970 |
| `X-Splashdown-Signature` | `sha256=` and the HMAC-SHA256, in hex, of the timestamp, a `.`, and the body, made with your webhook secret |

Copy the secret from Settings → Actions → Webhook secret. To check a request, in Python:

```python
import hashlib, hmac, time

def from_splashdown(secret: str, headers, body: bytes) -> bool:
    at = headers["X-Splashdown-Timestamp"]
    if abs(time.time() - int(at)) > 300:
        return False  # too old: someone may be replaying it
    expected = "sha256=" + hmac.new(secret.encode(), at.encode() + b"." + body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, headers["X-Splashdown-Signature"])
```

Services like Zapier don't check signatures; anyone who knows the address can send to it, so keep the address private too.

## Examples

**A log of everything you dictate** (Mac or Linux):

```sh
#!/bin/sh
{ printf '%s  %s\n' "$SPLASHDOWN_AT" "$SPLASHDOWN_APP"; cat; printf '\n\n'; } >> "$HOME/dictation-log.txt"
```

**A meeting's summary to a Slack channel.** In Slack, make an incoming webhook for the channel, then use a script, since Slack expects its own JSON:

```sh
#!/bin/sh
# Needs jq (brew install jq). SLACK_URL is your incoming webhook's address.
SLACK_URL="https://hooks.slack.com/services/…"
summary=$(sed -n '/^## Summary/,/^## /p' | sed '1d;$d')
jq -n --arg t "*$SPLASHDOWN_TITLE*
$summary" '{text: $t}' | curl -s -X POST -H 'Content-Type: application/json' -d @- "$SLACK_URL"
```

**The same on Windows** (PowerShell, a `.ps1` file):

```powershell
$SlackUrl = "https://hooks.slack.com/services/…"
$note = [Console]::In.ReadToEnd()
$summary = if ($note -match '(?ms)^## Summary\s*(.*?)(?=^## |\z)') { $Matches[1].Trim() } else { "" }
$text = "*$env:SPLASHDOWN_TITLE*`n$summary"
Invoke-RestMethod -Method Post -Uri $SlackUrl -ContentType "application/json" -Body (@{ text = $text } | ConvertTo-Json)
```

**A meeting to your coding agent.** An agent with tools of its own (your issue tracker, your calendar) can act on a meeting where Splashdown's own AI only writes. Hooks are stopped after 60 seconds, so the script hands the note over and returns:

```sh
#!/bin/sh
# Gives each meeting's note to a coding agent's command-line mode, here Claude Code's.
# Use the agent's full path (`which claude` prints it): hooks don't get your shell's PATH.
note=$(mktemp) && cat > "$note"
(
  /opt/homebrew/bin/claude -p "This is the note of a meeting I was in. File an issue for each action item that is mine." < "$note" >> "$HOME/meeting-agent.log" 2>&1
  rm -f "$note"
) &
```

The note holds what other people said, and an agent that can act may be talked into acting by it. Give the agent only the tools the job needs, and prefer ones that make drafts for you to check.

**Zapier, Make or n8n:** start a Zap (or scenario, or workflow) with a "Catch Hook" or "Webhook" trigger, copy its address into a hook, and press Test: the sample shows up with every field, ready to map to Notion, Google Sheets, email or anything else.

## Safety

- Hooks are only added in Settings, which says exactly what each one will run. A hook found in `config.toml` that wasn't saved there (written by hand, or by another program) asks before its first run. So does every hook in a settings file you import: the import lists what the file would run or send to before you agree, and nothing in it runs unasked.
- Voice commands, coding agents and notes can't add or change hooks.
- A hook that asks the AI gives it no tools: it writes text, and that text goes only where the hook says. Nothing in a meeting or a note can make it do anything else.
- An email draft is never sent by Splashdown, and is only to addresses you typed in Settings. Reminders are only added by a hook you set to add them, and each holds one line of what the AI wrote.
- Programs never run through a shell, so nothing in what you said can become a command.
- A hook gets what you dictated or recorded. A web address sends it off your computer, to whoever runs that address, and asking a cloud AI sends it to that provider: see [PRIVACY.md](PRIVACY.md).
