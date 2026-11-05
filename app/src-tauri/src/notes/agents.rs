//! Coding agents' access to notes (through `splashdown-mcp`): list, search,
//! read and append, each only as far as Settings → Privacy allows. A note
//! with `agents: false` in its front matter is never reached, and meetings
//! and transcripts only with their own switch.

use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use serde_json::{Value, json};
use splashdown_control::{NoteFilter, ReadPart};
use splashdown_core::meeting::Turn;

use super::*;

/// Asked for while Settings doesn't allow it.
pub(super) const AGENTS_OFF: &str = "Splashdown's notes are off for coding agents. To allow it, turn on “Let coding agents read notes” in Splashdown → Settings → Privacy.";

pub(super) const AGENTS_NO_WRITE: &str = "Coding agents may read Splashdown's notes but not add to them. To allow it, turn on “…and add to them” in Splashdown → Settings → Privacy.";

const MEETINGS_OFF: &str = "Meetings and transcripts are off for coding agents. To allow it, turn on “…including meetings and transcripts” in Splashdown → Settings → Privacy.";

const HIDDEN: &str = "The user keeps this note from coding agents.";

/// Most notes one search returns.
pub(super) const SEARCH_LIMIT: usize = 50;

/// Most notes one page of a list returns, and with summaries.
const LIST_LIMIT: usize = 100;
const LIST_LIMIT_WITH_SUMMARY: usize = 25;

/// The most of a summary a list gives, in characters.
const LIST_SUMMARY_CHARS: usize = 2_000;

/// How much of a note one `read_note` returns, in characters.
const READ_CHARS: usize = 20_000;
const READ_CHARS_MAX: usize = 40_000;

/// A reply has to fit the control channel's line (256 KB) once escaped
/// again on the way; anything bigger is refused with what to narrow.
const REPLY_BYTES: usize = 200_000;

/// Transcript turns a search hit points at, at most.
const HITS_PER_NOTE: usize = 3;

// ------------------------------------------------------------ tools

/// `list_notes`: the notes `filter` lets through, newest first (by when
/// they were made), a page at a time.
pub fn agent_list(
    settings: &NotesSettings,
    filter: &NoteFilter,
    with_summary: bool,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    let filter = Filter::parse(filter, settings, Local::now())?;
    let (store, _) = store_for(settings.folder.clone())?;
    let mut notes: Vec<Summary> = store
        .list()
        .into_iter()
        .filter(|n| visible(settings, n) && filter.admits(n))
        .collect();
    notes.sort_by(|a, b| made(b).cmp(&made(a)).then_with(|| a.path.cmp(&b.path)));
    let most = if with_summary {
        LIST_LIMIT_WITH_SUMMARY
    } else {
        LIST_LIMIT
    };
    let limit = limit.unwrap_or(20).clamp(1, most);
    let from = parse_cursor(cursor)?;
    let total = notes.len();
    let page: Vec<Value> = notes
        .iter()
        .skip(from)
        .take(limit)
        .map(|n| listed(n, with_summary))
        .collect();
    let next = from + page.len();
    let mut out = json!({
        "notes": page,
        "returned": page.len(),
        "total": total,
        "next_cursor": (next < total).then(|| next.to_string()),
    });
    if total == 0 {
        out["hint"] = json!(nothing_found(&filter, settings));
    }
    reply(&out)
}

/// `search_notes`: notes whose title or text contains `query` (any case),
/// titles first, then newest, with a short excerpt each and, in a
/// transcript, when the first matching turns were said.
pub fn agent_search(
    settings: &NotesSettings,
    query: &str,
    limit: Option<usize>,
    filter: &NoteFilter,
) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    let filter = Filter::parse(filter, settings, Local::now())?;
    let (store, _) = store_for(settings.folder.clone())?;
    let needle = query.trim().to_lowercase();
    let limit = limit.unwrap_or(10).clamp(1, SEARCH_LIMIT);
    let mut hits: Vec<(bool, Summary)> = store
        .list()
        .into_iter()
        .filter(|n| visible(settings, n) && filter.admits(n))
        .filter_map(|n| {
            let in_title = n.title.to_lowercase().contains(&needle);
            (needle.is_empty() || in_title || n.text.to_lowercase().contains(&needle))
                .then_some((in_title, n))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.modified.cmp(&a.1.modified)));
    let results: Vec<Value> = hits
        .into_iter()
        .take(limit)
        .map(|(_, n)| {
            let mut hit = json!({
                "path": n.path,
                "title": n.title,
                "kind": kind_of(&n),
                "created": n.created,
                "modified": iso(n.modified),
                "excerpt": excerpt(&n.text, &needle),
                "link": link_to(&n.path),
            });
            let said = said_in(&n.text, &needle);
            if !said.is_empty() {
                hit["said"] = json!(said);
            }
            hit
        })
        .collect();
    reply(&json!(results))
}

/// `read_note`: a note, or one part of it, a page at a time.
pub fn agent_read(settings: &NotesSettings, path: &str, part: &ReadPart) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    let (store, _) = store_for(settings.folder.clone())?;
    let summary = reachable(settings, &store, path)?;
    let note = store
        .read(path)
        .map_err(|e| format!("Couldn't read {path}: {e}"))?;
    let max_chars = part
        .max_chars
        .unwrap_or(READ_CHARS)
        .clamp(500, READ_CHARS_MAX);
    let section = part.section.as_deref().map(str::trim).unwrap_or("all");
    let mut out = json!({
        "path": note.path,
        "title": markdown::title_of(&note.body),
        "kind": kind_of(&summary),
        "section": section,
        "link": link_to(&note.path),
    });
    let ranged = part.start.is_some() || part.end.is_some() || part.speaker.is_some();
    if ranged && section != "transcript" {
        return Err("`start`, `end` and `speaker` only go with `section: \"transcript\"`.".into());
    }
    let parts = Parts::of(&note.body);
    let (text, next) = match section {
        "all" | "" => page_of(&note.body, part.cursor.as_deref(), max_chars)?,
        "summary" | "my_notes" => {
            let heading = if section == "summary" {
                "Summary"
            } else {
                crate::meeting::MY_NOTES
            };
            match markdown::section(&note.body, heading).filter(|t| !t.is_empty()) {
                Some(text) => page_of(&text, part.cursor.as_deref(), max_chars)?,
                None => {
                    out["missing"] = json!(if parts.turns.is_empty() {
                        format!(
                            "This note has no {} section. Read it with `section: \"all\"`.",
                            heading
                        )
                    } else {
                        format!(
                            "This note has no {} section yet. Read `section: \"transcript\"`.",
                            heading
                        )
                    });
                    (String::new(), None)
                }
            }
        }
        "transcript" => {
            if parts.turns.is_empty() {
                out["missing"] =
                    json!("This note has no transcript. Read it with `section: \"all\"`.");
                (String::new(), None)
            } else {
                parts.transcript(part, max_chars)?
            }
        }
        other => {
            return Err(format!(
                "`section` is one of all, summary, my_notes or transcript, not “{other}”."
            ));
        }
    };
    out["text"] = json!(text);
    out["next_cursor"] = json!(next);
    // Older agents looked for this.
    out["truncated"] = json!(out["next_cursor"].is_string());
    reply(&out)
}

/// `append_note`: adds `text` to the capture note, or the note at `to`.
pub fn agent_append(
    settings: &NotesSettings,
    text: &str,
    to: Option<&str>,
) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    if !settings.agents_write {
        return Err(AGENTS_NO_WRITE.into());
    }
    if text.trim().is_empty() {
        return Err("There's no text to add.".into());
    }
    if let Some(to) = to {
        let (store, _) = store_for(settings.folder.clone())?;
        reachable(settings, &store, to)?;
    }
    let captured = capture(settings, to, text, None)?;
    Ok(format!(
        "Added to {} ({})",
        captured.label, captured.appended.path
    ))
}

// ------------------------------------------------------------ who sees what

/// What a note is, for agents: `meeting`, `transcript` (an app's audio,
/// the microphone, a file or a link), `capture` or `note`.
fn kind_of(n: &Summary) -> &'static str {
    match n.source.as_str() {
        "meeting" => "meeting",
        crate::meeting::APP_AUDIO | crate::meeting::RECORDING | "file" => "transcript",
        "capture" => "capture",
        _ => "note",
    }
}

/// Its source in more detail than its kind: a file transcript of a page
/// link is a `link`.
fn source_of(n: &Summary) -> &str {
    if n.source == "file" && n.text.contains("Transcribed from <http") {
        "link"
    } else {
        &n.source
    }
}

fn visible(settings: &NotesSettings, n: &Summary) -> bool {
    !n.hidden_from_agents
        && (settings.agents_meetings() || !matches!(kind_of(n), "meeting" | "transcript"))
}

/// The note at `path` if agents may reach it, or why not.
fn reachable(settings: &NotesSettings, store: &Store, path: &str) -> Result<Summary, String> {
    let wanted = path.trim().trim_start_matches('/');
    let note = store
        .list()
        .into_iter()
        .find(|n| n.path == wanted)
        .ok_or_else(|| {
            format!("There's no note at {path}. Paths come from list_notes or search_notes.")
        })?;
    if note.hidden_from_agents {
        return Err(HIDDEN.into());
    }
    if !visible(settings, &note) {
        return Err(MEETINGS_OFF.into());
    }
    Ok(note)
}

/// When a note was made: its front matter's `created`, or its last change.
fn made(n: &Summary) -> i64 {
    n.created
        .as_deref()
        .and_then(|c| DateTime::parse_from_rfc3339(c).ok())
        .map_or(n.modified as i64, |t| t.timestamp_millis())
}

fn iso(ms: u64) -> Option<String> {
    DateTime::from_timestamp_millis(ms as i64).map(|t| t.with_timezone(&Local).to_rfc3339())
}

/// Opens the note in Splashdown's Notes window (macOS).
fn link_to(path: &str) -> String {
    format!("splashdown://open?note={}", crate::triggers::encode(path))
}

/// One note in a list.
fn listed(n: &Summary, with_summary: bool) -> Value {
    let mut out = json!({
        "path": n.path,
        "title": n.title,
        "kind": kind_of(n),
        "source": source_of(n),
        "created": n.created,
        "modified": iso(n.modified),
        "folder": n.folder,
        "tags": n.tags,
        "pinned": n.pinned,
        "link": link_to(&n.path),
    });
    if matches!(kind_of(n), "meeting" | "transcript") {
        let parts = Parts::of(&n.text);
        let summary = markdown::section(&n.text, "Summary").filter(|s| !s.is_empty());
        let mine = markdown::section(&n.text, crate::meeting::MY_NOTES).filter(|s| !s.is_empty());
        out["speakers"] = json!(parts.speakers());
        out["duration"] = json!(parts.duration());
        out["sections"] = json!({
            "summary": summary.is_some(),
            "my_notes": mine.is_some(),
            "transcript": !parts.turns.is_empty(),
            "recording": parts.recording,
        });
        if with_summary && let Some(summary) = summary {
            let cut: String = summary.chars().take(LIST_SUMMARY_CHARS).collect();
            out["summary_truncated"] = json!(cut.len() < summary.len());
            out["summary"] = json!(cut);
        }
    }
    out
}

/// Why a list came back empty, and what to try.
fn nothing_found(filter: &Filter, settings: &NotesSettings) -> String {
    let mut hint = String::from("No notes match.");
    if filter.since.is_some() || filter.until.is_some() {
        hint.push_str(" Try a wider `when`, or leave out `since` and `until`.");
    }
    if filter.kind.is_some() || filter.folder.is_some() || filter.tag.is_some() {
        hint.push_str(" Leaving out `kind`, `folder` or `tag` widens it.");
    }
    if !settings.agents_meetings() {
        hint.push(' ');
        hint.push_str(MEETINGS_OFF);
    }
    hint
}

fn reply(value: &Value) -> Result<String, String> {
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if text.len() > REPLY_BYTES {
        return Err("That's too much to return at once. Ask for fewer notes (`limit`), leave out `with_summary`, or read with a smaller `max_chars`.".into());
    }
    Ok(text)
}

// ------------------------------------------------------------ filters

/// A [`NoteFilter`] checked and made concrete.
#[derive(Debug, Default)]
struct Filter {
    since: Option<DateTime<Local>>,
    until: Option<DateTime<Local>>,
    kind: Option<&'static str>,
    /// Lowercase, no slashes at the ends.
    folder: Option<String>,
    tag: Option<String>,
}

impl Filter {
    fn parse(
        f: &NoteFilter,
        settings: &NotesSettings,
        now: DateTime<Local>,
    ) -> Result<Self, String> {
        fn text(v: &Option<String>) -> Option<&str> {
            v.as_deref().map(str::trim).filter(|v| !v.is_empty())
        }
        let mut out = Filter::default();
        if let Some(when) = text(&f.when) {
            let (since, until) = named_range(when, now)?;
            out.since = Some(since);
            out.until = Some(until);
        }
        if let Some(since) = text(&f.since) {
            out.since = Some(point(since, now, "since")?);
        }
        if let Some(until) = text(&f.until) {
            out.until = Some(point(until, now, "until")?);
        }
        if let Some(kind) = text(&f.kind) {
            out.kind = Some(match kind.to_lowercase().as_str() {
                "meeting" | "meetings" => "meeting",
                "transcript" | "transcripts" => "transcript",
                "capture" | "captures" => "capture",
                "note" | "notes" => "note",
                _ => {
                    return Err(format!(
                        "`kind` is one of meeting, transcript, capture or note, not “{kind}”."
                    ));
                }
            });
            if matches!(out.kind, Some("meeting" | "transcript")) && !settings.agents_meetings() {
                return Err(MEETINGS_OFF.into());
            }
        }
        out.folder = text(&f.folder).map(|v| v.trim_matches('/').to_lowercase());
        out.tag = text(&f.tag).map(|v| v.trim_start_matches('#').trim_matches('/').to_lowercase());
        Ok(out)
    }

    fn admits(&self, n: &Summary) -> bool {
        let at = made(n);
        self.since.is_none_or(|s| at >= s.timestamp_millis())
            && self.until.is_none_or(|u| at < u.timestamp_millis())
            && self.kind.is_none_or(|k| kind_of(n) == k)
            && self.folder.as_ref().is_none_or(|want| {
                let folder = n.folder.to_lowercase();
                want.is_empty() || folder == *want || folder.starts_with(&format!("{want}/"))
            })
            && self.tag.as_ref().is_none_or(|want| {
                n.tags.iter().any(|t| {
                    let t = t.to_lowercase();
                    t == *want || t.starts_with(&format!("{want}/"))
                })
            })
    }
}

fn midnight(day: NaiveDate) -> DateTime<Local> {
    let naive = day.and_hms_opt(0, 0, 0).unwrap_or_default();
    Local
        .from_local_datetime(&naive)
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&naive))
}

/// `when`: a named stretch of time, as [since, until).
fn named_range(
    when: &str,
    now: DateTime<Local>,
) -> Result<(DateTime<Local>, DateTime<Local>), String> {
    let today = now.date_naive();
    let monday = today - chrono::Days::new(u64::from(today.weekday().num_days_from_monday()));
    let first = today.with_day(1).unwrap_or(today);
    let day = chrono::Days::new(1);
    let week = chrono::Days::new(7);
    Ok(
        match when.to_lowercase().replace([' ', '-'], "_").as_str() {
            "today" => (midnight(today), midnight(today + day)),
            "yesterday" => (midnight(today - day), midnight(today)),
            "this_week" => (midnight(monday), midnight(monday + week)),
            "last_week" => (midnight(monday - week), midnight(monday)),
            "this_month" => (
                midnight(first),
                midnight(
                    first
                        .checked_add_months(chrono::Months::new(1))
                        .unwrap_or(today + day),
                ),
            ),
            "last_month" => (
                midnight(
                    first
                        .checked_sub_months(chrono::Months::new(1))
                        .unwrap_or(first),
                ),
                midnight(first),
            ),
            "last_7_days" => (
                midnight(today - chrono::Days::new(6)),
                midnight(today + day),
            ),
            "last_30_days" => (
                midnight(today - chrono::Days::new(29)),
                midnight(today + day),
            ),
            _ => {
                return Err(format!(
                    "`when` is one of today, yesterday, this_week, last_week, this_month, last_month, last_7_days or last_30_days, not “{when}”. For other dates use `since` and `until`."
                ));
            }
        },
    )
}

/// `since` or `until`: a date (`2026-11-02`, its midnight), a date and
/// time (RFC 3339), `today`, `yesterday`, or a while ago (`7d`, `36h`,
/// `90m`, `2w`).
fn point(value: &str, now: DateTime<Local>, name: &str) -> Result<DateTime<Local>, String> {
    let lower = value.to_lowercase();
    let today = now.date_naive();
    match lower.as_str() {
        "now" => return Ok(now),
        "today" => return Ok(midnight(today)),
        "yesterday" => return Ok(midnight(today - chrono::Days::new(1))),
        _ => {}
    }
    if let Ok(day) = NaiveDate::parse_from_str(&lower, "%Y-%m-%d") {
        return Ok(midnight(day));
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(value) {
        return Ok(t.with_timezone(&Local));
    }
    let digits: String = lower.chars().take_while(char::is_ascii_digit).collect();
    let unit = &lower[digits.len()..];
    if let Ok(n) = digits.parse::<i64>() {
        let ago = match unit {
            "m" | "min" | "mins" => chrono::Duration::try_minutes(n),
            "h" | "hr" | "hrs" | "hours" => chrono::Duration::try_hours(n),
            "d" | "day" | "days" => chrono::Duration::try_days(n),
            "w" | "wk" | "weeks" => chrono::Duration::try_weeks(n),
            _ => None,
        };
        if let Some(ago) = ago {
            return Ok(now - ago);
        }
    }
    Err(format!(
        "`{name}` takes a date (2026-11-02), a date and time (2026-11-02T14:00:00-05:00), today, yesterday, or a while ago (7d, 36h), not “{value}”."
    ))
}

fn parse_cursor(cursor: Option<&str>) -> Result<usize, String> {
    match cursor.map(str::trim).filter(|c| !c.is_empty()) {
        None => Ok(0),
        Some(c) => c
            .parse()
            .map_err(|_| format!("“{c}” isn't a cursor from this tool's `next_cursor`.")),
    }
}

// ------------------------------------------------------------ a note's parts

/// What a meeting or transcript note holds beyond its sections: its turns
/// (`**[m:ss] Who:** said`), how long it ran and whether a recording is
/// kept.
struct Parts<'a> {
    turns: Vec<Turn<'a>>,
    ended: Option<&'a str>,
    recording: bool,
}

impl<'a> Parts<'a> {
    fn of(body: &'a str) -> Self {
        let mut parts = Parts {
            turns: Vec::new(),
            ended: None,
            recording: false,
        };
        for line in body.lines() {
            let line = line.trim_end();
            if let Some(turn) = Turn::parse(line) {
                parts.turns.push(turn);
            } else if let Some(at) = line
                .strip_prefix("*Ended at ")
                .and_then(|r| r.strip_suffix(".*"))
            {
                parts.ended = Some(at);
            } else if line.starts_with("[Recording](") {
                parts.recording = true;
            }
        }
        parts
    }

    /// Everyone who speaks, in order of first speaking.
    fn speakers(&self) -> Vec<&'a str> {
        let mut out: Vec<&str> = Vec::new();
        for t in &self.turns {
            if !t.who.is_empty() && !out.contains(&t.who) {
                out.push(t.who);
            }
        }
        out
    }

    fn duration(&self) -> Option<&'a str> {
        self.ended.or_else(|| self.turns.last().map(|t| t.stamp))
    }

    /// The turns `part` asks for, from its cursor (a turn's index), as
    /// `[m:ss] Who: said` lines, up to `max_chars`; and where the next
    /// page starts.
    fn transcript(
        &self,
        part: &ReadPart,
        max_chars: usize,
    ) -> Result<(String, Option<String>), String> {
        let start = part
            .start
            .as_deref()
            .map(|s| time(s, "start"))
            .transpose()?;
        let end = part.end.as_deref().map(|s| time(s, "end")).transpose()?;
        let speaker = part
            .speaker
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase);
        let from = parse_cursor(part.cursor.as_deref())?;
        let wanted = |t: &Turn| {
            let at = t.at();
            start.is_none_or(|s| at >= s)
                && end.is_none_or(|e| at <= e)
                && speaker.as_ref().is_none_or(|s| t.who.to_lowercase() == *s)
        };
        let mut text = String::new();
        let mut chars = 0;
        for (i, turn) in self.turns.iter().enumerate().skip(from) {
            if !wanted(turn) {
                continue;
            }
            let line = if turn.who.is_empty() {
                format!("[{}] {}\n", turn.stamp, turn.said)
            } else {
                format!("[{}] {}: {}\n", turn.stamp, turn.who, turn.said)
            };
            let n = line.chars().count();
            if chars + n > max_chars && !text.is_empty() {
                return Ok((text, Some(i.to_string())));
            }
            chars += n;
            text.push_str(&line);
        }
        if text.is_empty() {
            let who = self.speakers().join(", ");
            return Err(format!(
                "No turns match. This transcript runs to {} and its speakers are {}.",
                self.duration().unwrap_or("0:00"),
                if who.is_empty() { "unnamed" } else { &who }
            ));
        }
        Ok((text, None))
    }
}

/// `m:ss`, `h:mm:ss` or seconds.
fn time(value: &str, name: &str) -> Result<Duration, String> {
    let value = value.trim();
    let parts: Vec<&str> = value.split(':').collect();
    let numbers: Option<Vec<u64>> = parts.iter().map(|p| p.parse().ok()).collect();
    match numbers.as_deref() {
        Some([s]) => Ok(Duration::from_secs(*s)),
        Some([m, s]) if *s < 60 => Ok(Duration::from_secs(m * 60 + s)),
        Some([h, m, s]) if *m < 60 && *s < 60 => Ok(Duration::from_secs(h * 3600 + m * 60 + s)),
        _ => Err(format!(
            "`{name}` is a time in the recording (1:05, 1:02:30 or 65), not “{value}”."
        )),
    }
}

/// A page of `text` from the character offset in `cursor`.
fn page_of(
    text: &str,
    cursor: Option<&str>,
    max_chars: usize,
) -> Result<(String, Option<String>), String> {
    let from = parse_cursor(cursor)?;
    let total = text.chars().count();
    if from > total {
        return Err("That cursor is past the end of the note.".into());
    }
    let page: String = text.chars().skip(from).take(max_chars).collect();
    let next = from + page.chars().count();
    Ok((page, (next < total).then(|| next.to_string())))
}

/// When the first few transcript turns containing `needle` were said, and
/// by whom, to read from with `read_note`.
fn said_in(body: &str, needle: &str) -> Vec<Value> {
    if needle.is_empty() {
        return Vec::new();
    }
    body.lines()
        .filter_map(|l| Turn::parse(l.trim_end()))
        .filter(|t| t.said.to_lowercase().contains(needle))
        .take(HITS_PER_NOTE)
        .map(|t| {
            if t.who.is_empty() {
                json!({ "at": t.stamp })
            } else {
                json!({ "at": t.stamp, "speaker": t.who })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEETING: &str = "# Weekly sync\n\nWith Dana Lee.\n\n## My notes\n\nBudget!\n\n## Summary\n\nWe keep the price.\n\n**[0:01] You:** hello\n\n**[0:05] Dana Lee:** the price stays\n\n**[1:08] Them:** bye\n\n*Ended at 1:09.*\n\n[Recording](Attachments/Weekly%20sync.wav)\n";

    fn note(path: &str, source: &str, created: &str, text: &str) -> Summary {
        Summary {
            path: path.into(),
            title: markdown::title_of(text),
            text: text.into(),
            modified: 0,
            pinned: false,
            folder: path
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_default(),
            source: source.into(),
            tags: vec!["work/acme".into()],
            created: Some(created.into()),
            hidden_from_agents: false,
        }
    }

    fn settings() -> NotesSettings {
        NotesSettings {
            agents_read: true,
            ..NotesSettings::default()
        }
    }

    fn at(s: &str) -> DateTime<Local> {
        DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&Local)
    }

    #[test]
    fn meetings_come_apart() {
        let parts = Parts::of(MEETING);
        assert_eq!(parts.speakers(), ["You", "Dana Lee", "Them"]);
        assert_eq!(parts.duration(), Some("1:09"));
        assert!(parts.recording);
        assert_eq!(
            markdown::section(MEETING, "Summary").as_deref(),
            Some("We keep the price.")
        );
        assert_eq!(
            markdown::section(MEETING, crate::meeting::MY_NOTES).as_deref(),
            Some("Budget!")
        );

        let whole = |p: ReadPart| parts.transcript(&p, 10_000).unwrap();
        let (text, next) = whole(ReadPart::default());
        assert_eq!(
            text,
            "[0:01] You: hello\n[0:05] Dana Lee: the price stays\n[1:08] Them: bye\n"
        );
        assert_eq!(next, None);
        let (text, _) = whole(ReadPart {
            speaker: Some("dana lee".into()),
            ..ReadPart::default()
        });
        assert_eq!(text, "[0:05] Dana Lee: the price stays\n");
        let (text, _) = whole(ReadPart {
            start: Some("0:05".into()),
            end: Some("65".into()),
            ..ReadPart::default()
        });
        assert_eq!(text, "[0:05] Dana Lee: the price stays\n");
        assert!(
            parts
                .transcript(
                    &ReadPart {
                        speaker: Some("Sam".into()),
                        ..ReadPart::default()
                    },
                    10_000
                )
                .unwrap_err()
                .contains("You, Dana Lee, Them")
        );
    }

    #[test]
    fn long_transcripts_come_in_pages() {
        let parts = Parts::of(MEETING);
        let (first, next) = parts.transcript(&ReadPart::default(), 20).unwrap();
        assert_eq!(first, "[0:01] You: hello\n");
        assert_eq!(next.as_deref(), Some("1"));
        let (second, next) = parts
            .transcript(
                &ReadPart {
                    cursor: next,
                    ..ReadPart::default()
                },
                40,
            )
            .unwrap();
        assert_eq!(second, "[0:05] Dana Lee: the price stays\n");
        assert_eq!(next.as_deref(), Some("2"));

        let (page, next) = page_of("abcdef", None, 4).unwrap();
        assert_eq!((page.as_str(), next.as_deref()), ("abcd", Some("4")));
        let (page, next) = page_of("abcdef", Some("4"), 4).unwrap();
        assert_eq!((page.as_str(), next), ("ef", None));
        assert!(page_of("abc", Some("x"), 4).is_err());
    }

    #[test]
    fn times_read_every_way() {
        assert_eq!(time("65", "start"), Ok(Duration::from_secs(65)));
        assert_eq!(time("1:05", "start"), Ok(Duration::from_secs(65)));
        assert_eq!(time("1:02:30", "start"), Ok(Duration::from_secs(3750)));
        assert!(time("1:75", "start").is_err());
        assert!(time("soon", "start").is_err());
    }

    #[test]
    fn named_ranges_follow_the_calendar() {
        // Wednesday 2026-11-04, 15:00 local.
        let now = Local.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap();
        let day = |y, m, d| Local.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap();
        assert_eq!(
            named_range("this_week", now).unwrap(),
            (day(2026, 11, 2), day(2026, 11, 9))
        );
        assert_eq!(
            named_range("last week", now).unwrap(),
            (day(2026, 10, 26), day(2026, 11, 2))
        );
        assert_eq!(
            named_range("today", now).unwrap(),
            (day(2026, 11, 4), day(2026, 11, 5))
        );
        assert_eq!(
            named_range("last_month", now).unwrap(),
            (day(2026, 10, 1), day(2026, 11, 1))
        );
        assert_eq!(
            named_range("last_7_days", now).unwrap().0,
            day(2026, 10, 29)
        );
        assert!(named_range("someday", now).is_err());
        // A Monday is in its own week.
        let monday = Local.with_ymd_and_hms(2026, 11, 2, 9, 0, 0).unwrap();
        assert_eq!(
            named_range("this_week", monday).unwrap().0,
            day(2026, 11, 2)
        );

        assert_eq!(point("2026-11-01", now, "since").unwrap(), day(2026, 11, 1));
        assert_eq!(
            point("7d", now, "since").unwrap(),
            now - chrono::Duration::days(7)
        );
        assert_eq!(
            point("36h", now, "since").unwrap(),
            now - chrono::Duration::hours(36)
        );
        assert_eq!(point("yesterday", now, "since").unwrap(), day(2026, 11, 3));
        assert!(point("2026-11-02T10:00:00Z", now, "since").is_ok());
        assert!(
            point("a while", now, "since")
                .unwrap_err()
                .contains("`since`")
        );
    }

    #[test]
    fn filters_pick_notes() {
        let now = at("2026-11-04T15:00:00Z");
        let s = settings();
        let parse = |f: NoteFilter| Filter::parse(&f, &s, now).unwrap();
        let sync = note(
            "Meetings/Sync.md",
            "meeting",
            "2026-11-03T10:00:00Z",
            MEETING,
        );
        let old = note("Old.md", "", "2026-09-01T10:00:00Z", "# Old\n\nText");
        let memo = note(
            "Work/Acme/Memo.md",
            crate::meeting::RECORDING,
            "2026-11-04T09:00:00Z",
            "# Memo",
        );

        let since = parse(NoteFilter {
            since: Some("2026-11-01".into()),
            ..NoteFilter::default()
        });
        assert!(since.admits(&sync) && !since.admits(&old));

        let kind = parse(NoteFilter {
            kind: Some("Transcripts".into()),
            ..NoteFilter::default()
        });
        assert!(kind.admits(&memo) && !kind.admits(&sync));

        let folder = parse(NoteFilter {
            folder: Some("work/".into()),
            ..NoteFilter::default()
        });
        assert!(folder.admits(&memo) && !folder.admits(&sync));

        let tag = parse(NoteFilter {
            tag: Some("#work".into()),
            ..NoteFilter::default()
        });
        assert!(tag.admits(&memo));
        let tag = parse(NoteFilter {
            tag: Some("wor".into()),
            ..NoteFilter::default()
        });
        assert!(!tag.admits(&memo), "a tag isn't matched by its start");

        assert!(
            Filter::parse(
                &NoteFilter {
                    kind: Some("email".into()),
                    ..NoteFilter::default()
                },
                &s,
                now
            )
            .is_err()
        );
    }

    #[test]
    fn hidden_notes_and_meetings_stay_out() {
        let mut s = settings();
        let sync = note(
            "Meetings/Sync.md",
            "meeting",
            "2026-11-03T10:00:00Z",
            MEETING,
        );
        let plan = note("Plan.md", "", "2026-11-03T10:00:00Z", "# Plan");
        assert!(visible(&s, &sync) && visible(&s, &plan));
        s.agents_meetings = Some(false);
        assert!(!visible(&s, &sync) && visible(&s, &plan));
        let meetings_only = NoteFilter {
            kind: Some("meeting".into()),
            ..NoteFilter::default()
        };
        assert_eq!(
            Filter::parse(&meetings_only, &s, Local::now()).unwrap_err(),
            MEETINGS_OFF
        );
        let hidden = Summary {
            hidden_from_agents: true,
            ..plan
        };
        assert!(!visible(&settings(), &hidden));
    }

    #[test]
    fn lists_describe_meetings() {
        let sync = note(
            "Meetings/Sync.md",
            "meeting",
            "2026-11-03T10:00:00Z",
            MEETING,
        );
        let item = listed(&sync, true);
        assert_eq!(item["kind"], "meeting");
        assert_eq!(item["duration"], "1:09");
        assert_eq!(item["sections"]["summary"], true);
        assert_eq!(item["sections"]["transcript"], true);
        assert_eq!(item["summary"], "We keep the price.");
        assert_eq!(item["link"], "splashdown://open?note=Meetings%2FSync.md");
        assert!(listed(&sync, false).get("summary").is_none());
        let plain = listed(&note("Plan.md", "", "2026-11-03T10:00:00Z", "# Plan"), true);
        assert!(plain.get("sections").is_none());

        let link = note(
            "Talk.md",
            "file",
            "2026-11-03T10:00:00Z",
            "# Talk\n\nTranscribed from <https://example.com/v> (4 min) on this computer.\n",
        );
        assert_eq!(listed(&link, false)["source"], "link");
        assert_eq!(listed(&link, false)["kind"], "transcript");
    }

    #[test]
    fn front_matter_values_are_set_and_cleared() {
        let front = "---\nsplashdown-id: 1-ab\ncreated: x\n---\n";
        let hidden = markdown::with_front_value(front, "agents", Some("false"));
        assert_eq!(
            hidden,
            "---\nsplashdown-id: 1-ab\ncreated: x\nagents: false\n---\n"
        );
        assert!(markdown::hidden_from_agents(&hidden));
        assert_eq!(markdown::with_front_value(&hidden, "agents", None), front);
        // Replaced in place, not added twice.
        let again = markdown::with_front_value(&hidden, "agents", Some("true"));
        assert_eq!(again.matches("agents:").count(), 1);
        assert!(!markdown::hidden_from_agents(&again));
        // A key that only starts the same is left alone.
        let other = "---\nagents_note: x\n---\n";
        assert_eq!(markdown::with_front_value(other, "agents", None), other);
        assert_eq!(
            markdown::with_front_value("", "agents", Some("false")),
            "---\nagents: false\n---\n"
        );
        assert_eq!(
            markdown::with_front_value("---\nagents: false\n---\n", "agents", None),
            ""
        );
    }

    #[test]
    fn agents_reach_a_real_notes_folder() {
        let dir = std::env::temp_dir().join(format!("splashdown-agents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Meetings")).unwrap();
        let now = Local::now().to_rfc3339();
        std::fs::write(
            dir.join("Meetings/Sync.md"),
            format!("---\nsplashdown-id: 1-ab\ncreated: {now}\nsource: meeting\n---\n{MEETING}"),
        )
        .unwrap();
        std::fs::write(
            dir.join("Secret.md"),
            format!("---\ncreated: {now}\nagents: false\n---\n# Secret\n\nThe price is a secret."),
        )
        .unwrap();
        std::fs::write(dir.join("Plan.md"), "# Plan\n\nThe price goes up.").unwrap();
        let mut s = NotesSettings {
            folder: Some(dir.clone()),
            ..settings()
        };
        let parse = |t: String| serde_json::from_str::<Value>(&t).unwrap();

        let meetings = NoteFilter {
            when: Some("today".into()),
            kind: Some("meeting".into()),
            ..NoteFilter::default()
        };
        let list = parse(agent_list(&s, &meetings, true, None, None).unwrap());
        assert_eq!(list["total"], 1);
        assert_eq!(list["notes"][0]["path"], "Meetings/Sync.md");
        assert_eq!(list["notes"][0]["summary"], "We keep the price.");

        let all = parse(agent_list(&s, &NoteFilter::default(), false, Some(1), None).unwrap());
        assert_eq!(all["total"], 2, "the hidden note isn't listed");
        assert_eq!(all["next_cursor"], "1");

        let found = parse(agent_search(&s, "price", None, &NoteFilter::default()).unwrap());
        let paths: Vec<&str> = found
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths.len(), 2);
        assert!(!paths.contains(&"Secret.md"));

        let summary = ReadPart {
            section: Some("summary".into()),
            ..ReadPart::default()
        };
        let read = parse(agent_read(&s, "Meetings/Sync.md", &summary).unwrap());
        assert_eq!(read["text"], "We keep the price.");
        assert_eq!(read["truncated"], false);
        assert_eq!(
            agent_read(&s, "Secret.md", &ReadPart::default()).unwrap_err(),
            HIDDEN
        );
        assert!(agent_read(&s, "Nowhere.md", &ReadPart::default()).is_err());
        let ranged = ReadPart {
            start: Some("0:05".into()),
            ..ReadPart::default()
        };
        assert!(
            agent_read(&s, "Meetings/Sync.md", &ranged).is_err(),
            "a range needs the transcript"
        );
        let missing = parse(agent_read(&s, "Plan.md", &summary).unwrap());
        assert!(missing["missing"].is_string());

        s.agents_meetings = Some(false);
        assert_eq!(
            agent_read(&s, "Meetings/Sync.md", &ReadPart::default()).unwrap_err(),
            MEETINGS_OFF
        );
        let all = parse(agent_list(&s, &NoteFilter::default(), false, None, None).unwrap());
        assert_eq!(all["total"], 1);
        s.agents_read = false;
        assert_eq!(
            agent_list(&s, &NoteFilter::default(), false, None, None).unwrap_err(),
            AGENTS_OFF
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_points_into_transcripts() {
        let said = said_in(MEETING, "price");
        assert_eq!(said, [json!({ "at": "0:05", "speaker": "Dana Lee" })]);
        assert!(said_in(MEETING, "").is_empty());
        assert!(said_in("# Plan\n\nThe price.", "price").is_empty());
        // A file's transcript names nobody: no empty speaker.
        assert_eq!(
            said_in("**[0:20]** And the price page.", "price"),
            [json!({ "at": "0:20" })]
        );
    }
}
