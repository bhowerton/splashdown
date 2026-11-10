# Hooks

A hook does something when something happens in Splashdown: after a dictation, when a meeting or transcript is finished, when a note comes from your iPhone, or at a time you set, with the notes of the day or week. It can ask the AI to do something with the text in your own words ("draft the follow-up email"), run your own script or program, or send to a web address. One kind runs *before* a dictation goes in and changes its text.

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

A dictation into a note (the note key) doesn't run dictation hooks. Neither does a rewrite of selected text.

A hook for meetings or transcripts can be for some of them only: **Only when the title has** takes a word or two ("1:1", a client's name), matched in any capitals.

## On a schedule

A hook on a schedule runs at a time of day (this computer's clock), every day, on weekdays, or on one day of the week, and gets your notes from **the last day** (the 24 hours before) or **the last 7 days**. That makes a weekly review, a daily wrap-up, or a digest sent somewhere else.

What it gets, oldest first, each note under its title and a line saying what it is and when:

- **Meetings and transcripts** made in that time. One that has a summary comes with its summary and your own notes, without its transcript; one that has none comes whole.
- **Other notes that changed in that time**, such as the Inbox, a daily note, or a note that collects another hook's results. These don't record when each line was added, so a long one comes as its last few paragraphs, which can include older lines.
- **Left out:** notes hidden from coding agents (`agents: false`), and the note the hook's own results go into.

If the computer is asleep or Splashdown isn't running at the time, the hook runs when it next can, once, with the notes up to the time it was due. A hook you just added, changed or turned back on waits for its next time. With no notes in that time, nothing runs, and What ran says so.

**Test** tries what you wrote on the notes it would get right now (or on a sample, when there are none) and shows the result without putting it anywhere.

## Asking the AI

No code needed: choose **Ask the AI to do something with it**, say what you want in your own words, and choose where what it writes goes. Some are ready to use under **Start from**, and can be changed:

| Start from | When | What it writes | Where |
|---|---|---|---|
| Follow-up email | A meeting ends | A short email to the others: thanks, what was decided, who does what | The meeting's note |
| What I said I'd do | A meeting ends | Your own commitments, as to-dos | A note called Commitments |
| Decisions | A meeting ends | What was decided, a line each | A note called Decisions |
| Key points | A transcript is finished | The main points, with their times | The transcript's note |
| Study cards | A transcript is finished | Questions and answers to study from | The transcript's note |
| To-dos from my phone | A note comes from your iPhone | The things to do in it, if any | A note called To-dos |
| Weekly review | Fridays at 16:00, the last 7 days | What got done or decided, what you said you'd do that's still open, what's waiting on others | A note called Weekly reviews |
| Daily wrap-up | Weekdays at 17:30, the last day | Your to-dos from the day, then what was decided | A note called Daily wrap-ups |

Where what it writes can go:

| Put what it writes | What happens |
|---|---|
| **In the note** (meetings and transcripts) | A section named after the hook, above the note's summary and transcript. Running it again for the same note replaces that section |
| **At the end of another note** | Added to a note in your notes folder (made if it isn't there), under a heading with the meeting's title and the day, or the days a scheduled hook covers. Results collect there, meeting after meeting |
| **At the end of a file** | The same, in any file you give the full path of |
| **On the clipboard** | Replaces what's on the clipboard, ready to paste |
| **To a web address** | Sent as a web address hook is (below), with `text` holding what the AI wrote instead of the note, and `ask` holding the hook's name |

If the AI finds nothing to do (no to-dos in a note), nothing is put anywhere, and What ran says so. **Test** asks about a short sample meeting and shows what it would write, without putting it anywhere.

### Which AI, and long meetings

The text goes to the AI that follows your instructions (Settings → AI Models), and the editor says which one that is: the one on this computer, or a cloud provider. In local-only mode it's always the one on this computer.

The AI on this computer reads about ten minutes of talk at a time. A longer meeting is read a part at a time, with notes taken from each part for what you asked, and your instruction is then run on the notes. That works, and takes a request per part: about a quarter of a minute for an hour-long meeting on a recent Mac, longer on a computer without a graphics chip the model can use. The meeting summary is written the same way.

What to expect from the AI on this computer (a small model):

- **Good at finding things that were said:** to-dos, decisions, who agreed to what. It sometimes credits a task to the wrong person, so read the list.
- **Weaker at writing from a whole long meeting:** a follow-up email for an hour-long meeting can leave things out.
- **A weekly review needs exact wording:** told plainly which headings to write and what goes under each (as the ready-made one is), it finds most decisions and to-dos in a week of notes and names the note each came from. It can miss one or put it under the wrong heading. Asked loosely ("review my week"), it makes up dates and mixes up who does what.

A cloud model reads the whole meeting at once and does both well. Either way, check what it wrote before you send it on.

### What it can't do

The AI only writes. It can't send, open, delete or run anything, and it can't choose where its text goes: you did. A meeting is mostly other people's words, and those words can't give it orders; the worst a transcript can do is change what gets written. It doesn't run before a dictation goes in (a preset's own instructions do that).

In `config.toml`:

```toml
[[hooks]]
when = "meeting"
kind = "ask"
name = "My commitments"
instruction = "List everything I said I would do, as to-dos."
put = "other-note"   # note, other-note, file, clipboard or web
to = "Commitments"   # the note, the file's full path, or the web address
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
```

## A script or program

Splashdown starts it directly, never through a shell, with the arguments you gave it, one per line. It gets:

- **The text on its standard input:** the dictation, the note from your iPhone, the whole note (Markdown) for a meeting or transcript, or the notes of the day or week for a hook on a schedule.
- **These environment variables:**

| Variable | Holds |
|---|---|
| `SPLASHDOWN_EVENT` | `dictation`, `before-insert`, `meeting`, `transcript`, `phone-note` or `schedule` |
| `SPLASHDOWN_AT` | When it happened, as `2026-11-09T14:03:22-05:00` (for a note from the iPhone, when you said it; for a schedule, the time it was due) |
| `SPLASHDOWN_SINCE` | Where the notes start, in the same form (schedules only): they run from then until `SPLASHDOWN_AT` |
| `SPLASHDOWN_APP` | The app dictated into (dictations only) |
| `SPLASHDOWN_PRESET` | The preset's name (dictations only) |
| `SPLASHDOWN_TITLE` | The note's title (meetings and transcripts) |
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

For a dictation, `app` and `preset` are filled and `note` is `null`. A hook on a schedule has the notes in `text`, the time it was due in `at`, and adds `"since"` with where the notes start. Test adds `"test": true`. A hook that asks the AI and sends what it wrote here adds `"ask"` with the hook's name, and its `text` is what the AI wrote.

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
- Programs never run through a shell, so nothing in what you said can become a command.
- A hook gets what you dictated or recorded. A web address sends it off your computer, to whoever runs that address, and asking a cloud AI sends it to that provider: see [PRIVACY.md](PRIVACY.md).
