# Hooks

A hook runs your own script or program, or sends to a web address, when something happens in Splashdown: after a dictation, when a meeting or transcript is finished, or when a note comes from your iPhone. One kind runs *before* a dictation goes in and changes its text.

Add them in **Settings → Actions → When something happens**. **Test** runs a hook once with sample text before you save it.

## When a hook runs

| When | Gets | Choose a preset? |
|---|---|---|
| **After a dictation** | The text that went into the field | Every preset, or one |
| **Before a dictation goes in** | The text about to go in; what the program writes back goes in instead | One preset (required) |
| **When a meeting ends** | The meeting's note (Markdown), its title, summary and file | No |
| **When a transcript is finished** | The transcript's note: a file, a link, a watched folder, an app's audio or the microphone | No |
| **When a note comes from your iPhone** | What you said, and when | No |

A dictation into a note (the note key) doesn't run dictation hooks. Neither does a rewrite of selected text.

## A script or program

Splashdown starts it directly, never through a shell, with the arguments you gave it, one per line. It gets:

- **The text on its standard input:** the dictation, the note from your iPhone, or the whole note (Markdown) for a meeting or transcript.
- **These environment variables:**

| Variable | Holds |
|---|---|
| `SPLASHDOWN_EVENT` | `dictation`, `before-insert`, `meeting`, `transcript` or `phone-note` |
| `SPLASHDOWN_AT` | When it happened, as `2026-11-09T14:03:22-05:00` (for a note from the iPhone, when you said it) |
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

For a dictation, `app` and `preset` are filled and `note` is `null`. Test adds `"test": true`.

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

**Zapier, Make or n8n:** start a Zap (or scenario, or workflow) with a "Catch Hook" or "Webhook" trigger, copy its address into a hook, and press Test: the sample shows up with every field, ready to map to Notion, Google Sheets, email or anything else.

## Safety

- Hooks are only added in Settings, which says exactly what each one will run. A hook found in `config.toml` that wasn't saved there (written by hand, or by another program) asks before its first run.
- Voice commands, coding agents and notes can't add or change hooks.
- Programs never run through a shell, so nothing in what you said can become a command.
- A hook gets what you dictated or recorded. A web address sends it off your computer, to whoever runs that address: see [PRIVACY.md](PRIVACY.md).
