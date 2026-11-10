//! Hooks on a schedule ("on Fridays at 16:00"): when the time comes round,
//! the hook gets the notes made in the day or the week before, to write a
//! review of them, list what's still open, or send them on.
//!
//! The time is this computer's clock. A time that passed while the computer
//! slept or Splashdown wasn't running is caught up on when it next can be,
//! once, with the notes up to the time it was due. A hook that's new,
//! changed or turned back on starts from its next time, never with one
//! already past.

use std::collections::BTreeMap;

use chrono::{Datelike, TimeZone};
use serde::Deserialize;
use splashdown_config::{Day, Schedule};

use super::*;
use crate::notes::agents::{kind_of, made};
use crate::notes::markdown;
use crate::notes::store::Summary;

/// How often the time is looked at.
const EVERY: Duration = Duration::from_secs(30);
/// When each scheduled hook was last seen to: `hook-schedule.json` in the
/// settings folder.
const FILE: &str = "hook-schedule.json";
/// How much of a note with no dates in it is given: its end, where what's
/// new is. Notes that collect hooks' results grow for months, and only say
/// when they last changed; so does an Inbox from before its lines came
/// under their days.
const UNDATED_SHOWN: usize = 4000;

/// Notes to try a scheduled hook on, for Test, when there are none.
pub(super) const SAMPLE: &str = "# Pricing review\n[A meeting, Monday, 9 November 2026 at 14:00]\n\nWith Dana Lee.\n\n## Summary\n\nThe new pricing page goes live on Friday, once Dana has the final numbers. You send them by Wednesday and write to support first.\n\n# Hiring sync\n[A meeting, Wednesday, 11 November 2026 at 10:30]\n\nWith Sam Reyes.\n\n## Summary\n\nTwo candidates go to the last round. Sam books the interviews; you write the offer template by next Monday.\n\n# 2026-11-12\n[A note, last changed Thursday, 12 November 2026 at 17:10]\n\n**09:12** Idea: a weekly digest of what I agreed to in meetings.\n\n**17:10** Call the dentist tomorrow morning.\n";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Seen {
    /// Each scheduled hook that's on, by what it runs and when, with the
    /// time it was last run for or first seen.
    #[serde(default)]
    hooks: BTreeMap<String, String>,
}

/// Starts watching the time, for as long as the app runs.
pub fn watch(app: &AppHandle) {
    let app = app.clone();
    let started = std::thread::Builder::new()
        .name("hook-schedule".into())
        .spawn(move || {
            loop {
                std::thread::sleep(EVERY);
                check(&app);
            }
        });
    if let Err(e) = started {
        tracing::error!("couldn't start watching for scheduled hooks: {e}");
    }
}

/// Runs the scheduled hooks whose time has come.
fn check(app: &AppHandle) {
    let Some(settings) = app.try_state::<AppState>().map(|s| s.settings()) else {
        return;
    };
    let Ok(path) = crate::data::path(FILE) else {
        return;
    };
    let any = settings.hooks.iter().any(|h| h.when == HookEvent::Schedule);
    if !any && !path.exists() {
        return;
    }
    let mut seen: Seen = crate::data::read(&path);
    let before = seen.hooks.clone();
    let now = Local::now();
    let due = due(&mut seen.hooks, &settings.hooks, now);
    // Before they run: a run cut short isn't done twice.
    if seen.hooks != before
        && let Err(e) = crate::data::write(&path, &seen)
    {
        tracing::warn!("couldn't keep when scheduled hooks ran: {e}");
        // Not kept, so they'd run again every half minute.
        return;
    }
    for (hook, at) in due {
        run(app, &settings, hook, at);
    }
}

/// The scheduled hooks whose time has come since they were last seen to,
/// each with the time it was due; `seen` is brought up to `now`.
fn due(
    seen: &mut BTreeMap<String, String>,
    hooks: &[Hook],
    now: DateTime<Local>,
) -> Vec<(Hook, DateTime<Local>)> {
    let mut out = Vec::new();
    let mut kept = BTreeMap::new();
    for hook in hooks
        .iter()
        .filter(|h| h.on && h.when == HookEvent::Schedule)
    {
        let Some(schedule) = &hook.schedule else {
            continue;
        };
        let key = key(hook);
        let since = seen
            .get(&key)
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            // A clock that was set back mustn't hold it up.
            .map(|t| t.with_timezone(&Local).min(now));
        let seen_to = match (since, last_due(schedule, now)) {
            (Some(since), Some(due)) if due > since => {
                out.push((hook.clone(), due));
                now
            }
            (Some(since), _) => since,
            // New, changed or turned on again: from its next time.
            (None, _) => now,
        };
        kept.insert(key, seen_to.to_rfc3339());
    }
    // Hooks that are gone or off are forgotten.
    *seen = kept;
    out
}

/// A scheduled hook by what it runs and when: changing either makes it
/// new.
fn key(hook: &Hook) -> String {
    use sha2::Digest;
    let what = serde_json::to_string(&(&hook.run, &hook.schedule)).unwrap_or_default();
    hex(&sha2::Sha256::digest(what.as_bytes())[..8])
}

/// The last time on or before `now` that the schedule came round.
fn last_due(schedule: &Schedule, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let (hour, minute) = schedule.time()?;
    (0..=7).find_map(|back| {
        let day = now.date_naive() - chrono::Days::new(back);
        if !schedule.runs_on(day.weekday().num_days_from_monday()) {
            return None;
        }
        // A time the clocks skip that day never comes.
        let at = Local
            .from_local_datetime(&day.and_hms_opt(hour, minute, 0)?)
            .earliest()?;
        (at <= now).then_some(at)
    })
}

/// When a schedule runs, to end a sentence: "on Fridays at 16:00".
pub(super) fn words(schedule: &Schedule) -> String {
    const ALL: [Day; 7] = [
        Day::Mon,
        Day::Tue,
        Day::Wed,
        Day::Thu,
        Day::Fri,
        Day::Sat,
        Day::Sun,
    ];
    let days: Vec<Day> = ALL
        .into_iter()
        .filter(|d| schedule.days.contains(d))
        .collect();
    let at = schedule.at.trim();
    let weekend = days.contains(&Day::Sat) || days.contains(&Day::Sun);
    match days.len() {
        0 | 7 => format!("every day at {at}"),
        5 if !weekend => format!("on weekdays at {at}"),
        _ => {
            let names: Vec<String> = days.iter().map(|d| format!("{}s", d.name())).collect();
            let (last, rest) = names.split_last().expect("at least one day");
            if rest.is_empty() {
                format!("on {last} at {at}")
            } else {
                format!("on {} and {last} at {at}", rest.join(", "))
            }
        }
    }
}

/// Runs `hook`, due at `due`, on the notes made before then.
fn run(app: &AppHandle, settings: &Settings, hook: Hook, due: DateTime<Local>) {
    let Some(schedule) = hook.schedule.clone() else {
        return;
    };
    let nothing = |ok: bool, said: String| {
        runs::record(
            app,
            runs::Run {
                at: Local::now().to_rfc3339(),
                event: HookEvent::Schedule.name().to_string(),
                hook: name(&hook),
                about: Happened::scheduled(String::new(), since(&schedule, due), due).period(),
                ok,
                said,
                took: 0.0,
            },
        );
    };
    match gathered(settings, &hook, &schedule, due) {
        Ok(Some((happened, notes))) => {
            tracing::info!("a scheduled hook is due, with {notes} note(s)");
            run_later(app, hook, happened);
        }
        // Nothing to read: said in what ran, so it isn't a mystery.
        Ok(None) => nothing(true, format!("No notes from {}", schedule.covers.words())),
        Err(e) => {
            tracing::warn!("a scheduled hook couldn't read the notes: {e}");
            nothing(false, e);
        }
    }
}

/// Where a schedule's notes start, for one due at `due`.
fn since(schedule: &Schedule, due: DateTime<Local>) -> DateTime<Local> {
    due - chrono::Duration::days(i64::from(schedule.covers.days()))
}

/// What a scheduled hook due at `due` gets, and how many notes that is;
/// `None` when there are none.
fn gathered(
    settings: &Settings,
    hook: &Hook,
    schedule: &Schedule,
    due: DateTime<Local>,
) -> Result<Option<(Happened, usize)>, String> {
    let since = since(schedule, due);
    let (store, _) = crate::notes::store_for(settings.notes.folder.clone())?;
    // Not the note its own results go in: last week's review isn't this
    // week's news.
    let own = match &hook.run {
        HookTarget::Ask {
            put: PutIn::OtherNote,
            to: Some(to),
            ..
        } => Some(ask::other_note(to).0),
        _ => None,
    };
    let (text, notes) = notes_between(&store.list(), since, due, own.as_deref());
    Ok((notes > 0).then(|| (Happened::scheduled(text, since, due), notes)))
}

/// The notes a scheduled hook would get now, for Test, and how many in
/// words ("5 from the last 7 days"); `None` when there are none, or it
/// isn't on a schedule.
pub(super) fn now(app: &AppHandle, hook: &Hook) -> Option<(Happened, String)> {
    if hook.when != HookEvent::Schedule {
        return None;
    }
    let schedule = hook.schedule.as_ref()?;
    let settings = app.try_state::<AppState>()?.settings();
    let (happened, notes) = gathered(&settings, hook, schedule, Local::now()).ok()??;
    Some((
        happened,
        format!("{notes} from {}", schedule.covers.words()),
    ))
}

/// The notes made from `since` until `until`, oldest first, as one text,
/// and how many. Each starts with its title and a line saying what it is
/// and when. A meeting or transcript that has a summary comes without its
/// transcript. A note that doesn't say when it was made (the Inbox, a
/// daily note, one that collects hooks' results) counts when it changed
/// in that time: the lines added then when it says which those are (the
/// Inbox's day headings), or else all of it, or its end when it's long.
/// Notes hidden from agents, and the note at `skip`, are left out.
fn notes_between(
    notes: &[Summary],
    since: DateTime<Local>,
    until: DateTime<Local>,
    skip: Option<&str>,
) -> (String, usize) {
    let (from, to) = (since.timestamp_millis(), until.timestamp_millis());
    let mut picked: Vec<&Summary> = notes
        .iter()
        .filter(|n| {
            let at = made(n);
            at >= from
                && at <= to
                && !n.hidden_from_agents
                && !skip.is_some_and(|s| s.eq_ignore_ascii_case(&n.path))
        })
        .collect();
    picked.sort_by_key(|n| made(n));
    let mut out = String::new();
    let mut count = 0;
    for n in picked {
        let dated = n.created.is_some();
        let text = shown(&n.text);
        // How much of it is given: all of a note made in that time; of
        // one that only changed in it, the lines added then when it says
        // which those are, or else its end.
        let (text, part) = if dated {
            (text.to_string(), Part::Whole)
        } else if let Some(lines) = lines_between(text, since, until) {
            (lines, Part::Lines)
        } else {
            match end_of(text, UNDATED_SHOWN) {
                (end, true) => (end.to_string(), Part::End),
                (all, false) => (all.to_string(), Part::Whole),
            }
        };
        if text.is_empty() {
            continue;
        }
        let Some(at) = DateTime::from_timestamp_millis(made(n)).map(|t| t.with_timezone(&Local))
        else {
            continue;
        };
        let what = match kind_of(n) {
            "meeting" => "A meeting",
            "transcript" => "A transcript",
            "capture" => "Dictated notes",
            _ => "A note",
        };
        let when = format!(
            "{} at {}",
            splashdown_core::prompt::today(at.date_naive()),
            at.format("%H:%M")
        );
        let label = match (dated, part) {
            (true, _) => format!("{what}, {when}"),
            (false, Part::Whole) => format!("{what}, last changed {when}"),
            (false, Part::Lines) => format!("{what}: the lines added in this time"),
            (false, Part::End) => format!("{what}, last changed {when}; only its end is here"),
        };
        out.push_str(&format!("# {}\n[{label}]\n\n{text}\n\n", n.title.trim()));
        count += 1;
    }
    (format!("{}\n", out.trim_end()), count)
}

/// How much of a note a scheduled hook is given.
#[derive(Clone, Copy)]
enum Part {
    Whole,
    /// The lines added in the time it covers.
    Lines,
    /// Its last paragraphs.
    End,
}

/// The paragraphs of a note that were added from `since` until `until`,
/// under their days, for a note that says which day each was added on:
/// the Inbox, with a heading above each day's lines (`## 2026-11-10`) and
/// each line starting with its time (`**09:15** …`). A paragraph with no
/// time goes by its day. What comes before the first day's heading has no
/// day, so is left out. `None` when the note has no such headings.
fn lines_between(text: &str, since: DateTime<Local>, until: DateTime<Local>) -> Option<String> {
    let mut day = None;
    let mut kept: Vec<(chrono::NaiveDate, String)> = Vec::new();
    let mut headings = false;
    // Whether the paragraph being read is kept, and whether a new one starts.
    let mut keep = false;
    let mut starts = true;
    for line in text.lines() {
        if line.trim().is_empty() {
            starts = true;
            continue;
        }
        if let Some(named) = markdown::day_heading(line) {
            day = Some(named);
            headings = true;
            starts = true;
            continue;
        }
        let Some(on) = day else {
            continue;
        };
        if starts {
            keep = match markdown::timed(line) {
                Some((time, _)) => Local
                    .from_local_datetime(&on.and_time(time))
                    .earliest()
                    .is_some_and(|at| at > since && at <= until),
                None => on >= since.date_naive() && on <= until.date_naive(),
            };
            if keep {
                kept.push((on, String::new()));
            }
            starts = false;
        }
        if keep && let Some((_, paragraph)) = kept.last_mut() {
            if !paragraph.is_empty() {
                paragraph.push('\n');
            }
            paragraph.push_str(line.trim_end());
        }
    }
    if !headings {
        return None;
    }
    let mut out = String::new();
    let mut under = None;
    for (on, paragraph) in kept {
        if under != Some(on) {
            out.push_str(&format!("## {}\n\n", on.format("%Y-%m-%d")));
            under = Some(on);
        }
        out.push_str(&paragraph);
        out.push_str("\n\n");
    }
    Some(out.trim_end().to_string())
}

/// A note's text as a scheduled hook gets it: without its title line
/// (given before it), and without its transcript when it has a summary.
fn shown(body: &str) -> &str {
    let body = match markdown::transcript_start(body) {
        Some(at) if markdown::section(body, "Summary").is_some_and(|s| !s.trim().is_empty()) => {
            &body[..at]
        }
        _ => body,
    };
    let body = body.trim();
    match body.split_once('\n') {
        Some((first, rest)) if first.starts_with("# ") => rest.trim(),
        None if body.starts_with("# ") => "",
        _ => body,
    }
}

/// The last `most` characters or so of `text`, from the start of a
/// paragraph where there is one, and whether anything was left off.
fn end_of(text: &str, most: usize) -> (&str, bool) {
    if text.len() <= most {
        return (text, false);
    }
    let mut start = text.len() - most;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let tail = &text[start..];
    let tail = tail
        .find("\n\n")
        .or_else(|| tail.find('\n'))
        .map_or(tail, |at| &tail[at..]);
    (tail.trim(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use splashdown_config::Covers;

    fn at(s: &str) -> DateTime<Local> {
        let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    fn schedule(time: &str, days: &[Day], covers: Covers) -> Schedule {
        Schedule {
            at: time.into(),
            days: days.to_vec(),
            covers,
        }
    }

    fn hook(name: &str, schedule: Schedule) -> Hook {
        Hook {
            when: HookEvent::Schedule,
            run: HookTarget::Ask {
                name: name.into(),
                instruction: "Write a review of my week.".into(),
                put: PutIn::OtherNote,
                to: Some("Weekly reviews".into()),
            },
            preset: None,
            title_has: None,
            on: true,
            trusted: true,
            schedule: Some(schedule),
        }
    }

    fn note(path: &str, source: &str, created: Option<&str>, changed: &str, text: &str) -> Summary {
        Summary {
            path: path.into(),
            title: markdown::title_of(text),
            text: text.into(),
            modified: at(changed).timestamp_millis() as u64,
            pinned: false,
            folder: String::new(),
            source: source.into(),
            tags: Vec::new(),
            created: created.map(|c| at(c).to_rfc3339()),
            hidden_from_agents: false,
        }
    }

    #[test]
    fn a_schedule_s_last_time_is_found() {
        // 2026-11-13 is a Friday.
        let fridays = schedule("16:00", &[Day::Fri], Covers::Week);
        assert_eq!(
            last_due(&fridays, at("2026-11-13 16:00")),
            Some(at("2026-11-13 16:00"))
        );
        assert_eq!(
            last_due(&fridays, at("2026-11-13 15:59")),
            Some(at("2026-11-06 16:00"))
        );
        assert_eq!(
            last_due(&fridays, at("2026-11-16 09:00")),
            Some(at("2026-11-13 16:00"))
        );
        let daily = schedule("8:30", &[], Covers::Day);
        assert_eq!(
            last_due(&daily, at("2026-11-13 08:00")),
            Some(at("2026-11-12 08:30"))
        );
        assert_eq!(
            last_due(&schedule("late", &[], Covers::Day), at("2026-11-13 08:00")),
            None
        );
    }

    #[test]
    fn schedules_are_said_in_words() {
        let on = |days: &[Day]| words(&schedule("16:00", days, Covers::Day));
        assert_eq!(on(&[]), "every day at 16:00");
        assert_eq!(on(&[Day::Fri]), "on Fridays at 16:00");
        assert_eq!(on(&[Day::Fri, Day::Mon]), "on Mondays and Fridays at 16:00");
        assert_eq!(
            on(&[Day::Mon, Day::Wed, Day::Sat]),
            "on Mondays, Wednesdays and Saturdays at 16:00"
        );
        assert_eq!(
            on(&[Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri]),
            "on weekdays at 16:00"
        );
    }

    #[test]
    fn a_hook_runs_once_when_its_time_comes_and_catches_up_once() {
        let weekly = hook(
            "Weekly review",
            schedule("16:00", &[Day::Fri], Covers::Week),
        );
        let hooks = vec![weekly.clone()];
        let mut seen = BTreeMap::new();
        // Added on Friday evening: not for the time that's just past.
        assert!(due(&mut seen, &hooks, at("2026-11-06 18:00")).is_empty());
        assert_eq!(seen.len(), 1);
        assert!(due(&mut seen, &hooks, at("2026-11-13 15:59")).is_empty());
        // The time comes.
        let ran = due(&mut seen, &hooks, at("2026-11-13 16:00"));
        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].1, at("2026-11-13 16:00"));
        assert!(due(&mut seen, &hooks, at("2026-11-13 16:01")).is_empty());
        // Asleep for two Fridays: one run, for the last of them.
        let ran = due(&mut seen, &hooks, at("2026-11-30 09:00"));
        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].1, at("2026-11-27 16:00"));
        assert!(due(&mut seen, &hooks, at("2026-11-30 09:01")).is_empty());

        // Off, it's forgotten; on again, it starts from its next time.
        let off = vec![Hook {
            on: false,
            ..weekly.clone()
        }];
        assert!(due(&mut seen, &off, at("2026-12-01 09:00")).is_empty());
        assert!(seen.is_empty());
        assert!(due(&mut seen, &hooks, at("2026-12-07 09:00")).is_empty());
        assert_eq!(due(&mut seen, &hooks, at("2026-12-11 16:00")).len(), 1);

        // A new time makes it new; trusting it doesn't.
        let moved = hook(
            "Weekly review",
            schedule("17:00", &[Day::Fri], Covers::Week),
        );
        assert_ne!(key(&moved), key(&weekly));
        assert_eq!(
            key(&Hook {
                trusted: false,
                ..weekly.clone()
            }),
            key(&weekly)
        );
        // A clock set back doesn't hold it up for good.
        let mut ahead = BTreeMap::from([(key(&weekly), at("2027-06-01 00:00").to_rfc3339())]);
        assert!(due(&mut ahead, &hooks, at("2026-12-12 10:00")).is_empty());
        assert_eq!(due(&mut ahead, &hooks, at("2026-12-18 16:00")).len(), 1);
    }

    #[test]
    fn a_scheduled_hook_gets_the_notes_made_since() {
        const MEETING: &str = "# Weekly sync\n\nWith Dana Lee.\n\n## Summary\n\nWe keep the price.\n\n**[0:01] You:** hello\n\n**[0:05] Dana Lee:** the price stays\n";
        const NO_SUMMARY: &str = "# Standup\n\n**[0:01] You:** I'll fix the build today.\n";
        let mut inbox = String::from("# Inbox\n\n");
        for n in 0..400 {
            inbox.push_str(&format!("**10:{:02}** Old thought number {n}\n\n", n % 60));
        }
        inbox.push_str("**11:38** Call the dentist tomorrow morning.\n");
        let hidden = Summary {
            hidden_from_agents: true,
            ..note(
                "Secret.md",
                "",
                Some("2026-11-11 09:00"),
                "2026-11-11 09:00",
                "# Secret\n\nThe price.",
            )
        };
        let notes = vec![
            note("Inbox.md", "", None, "2026-11-12 11:38", &inbox),
            note(
                "Meetings/Standup.md",
                "meeting",
                Some("2026-11-12 09:00"),
                "2026-11-12 09:05",
                NO_SUMMARY,
            ),
            note(
                "Meetings/Weekly sync.md",
                "meeting",
                Some("2026-11-09 14:03"),
                "2026-11-13 10:00",
                MEETING,
            ),
            note(
                "Meetings/Old.md",
                "meeting",
                Some("2026-11-02 14:03"),
                "2026-11-12 10:00",
                MEETING,
            ),
            note(
                "Weekly reviews.md",
                "",
                None,
                "2026-11-10 16:00",
                "# Weekly reviews\n\nLast week's.",
            ),
            note(
                "Daily/2026-11-10.md",
                "",
                None,
                "2026-11-10 12:00",
                "# 2026-11-10",
            ),
            hidden,
        ];
        let (text, count) = notes_between(
            &notes,
            at("2026-11-06 16:00"),
            at("2026-11-13 16:00"),
            Some("weekly reviews.md"),
        );
        assert_eq!(count, 3, "{text}");
        // Oldest first, each under its title and what it is.
        let sync = text
            .find("# Weekly sync\n[A meeting, Monday, 9 November 2026 at 14:03]")
            .unwrap();
        let standup = text
            .find("# Standup\n[A meeting, Thursday, 12 November 2026 at 09:00]")
            .unwrap();
        let inbox_at = text
            .find("# Inbox\n[A note, last changed Thursday, 12 November 2026 at 11:38; only its end is here]")
            .unwrap();
        assert!(sync < standup && standup < inbox_at, "{text}");
        // A summary stands in for the transcript; without one, the transcript.
        assert!(text.contains("We keep the price.") && !text.contains("the price stays"));
        assert!(text.contains("I'll fix the build today."));
        // The end of a long note with no date, from a whole line.
        assert!(text.contains("Call the dentist") && !text.contains("number 0\n"));
        assert!(text.len() < UNDATED_SHOWN + 1000);
        // Made before, hidden, empty, or where the results go: left out.
        assert!(
            !text.contains("Secret")
                && !text.contains("Last week's")
                && !text.contains("2026-11-10\n[")
        );
        assert_eq!(text.matches("# Weekly sync").count(), 1);

        assert_eq!(
            notes_between(&notes, at("2026-10-01 00:00"), at("2026-10-02 00:00"), None).1,
            0
        );

        // An Inbox that says which day each line is from: only the lines
        // from that time, to the minute.
        let dated = "# Inbox\n\n**08:00** From before the days were kept\n\n## 2026-11-05\n\n**10:00** Too old\n\n## 2026-11-06\n\n**15:59** Just before\n\n**16:01** Just after\nwith a second line\n\n## 2026-11-12\n\nTyped by hand, no time\n\n**11:38** Call the dentist\n\n## 2026-11-13\n\n**16:00** On the dot\n\n**16:30** After it ran\n";
        let inbox = vec![note("Inbox.md", "", None, "2026-11-13 16:30", dated)];
        let (text, count) =
            notes_between(&inbox, at("2026-11-06 16:00"), at("2026-11-13 16:30"), None);
        assert_eq!(count, 1);
        assert_eq!(
            text,
            "# Inbox\n[A note: the lines added in this time]\n\n## 2026-11-06\n\n**16:01** Just after\nwith a second line\n\n## 2026-11-12\n\nTyped by hand, no time\n\n**11:38** Call the dentist\n\n## 2026-11-13\n\n**16:00** On the dot\n\n**16:30** After it ran\n"
        );
        let week = |until: &str| lines_between(dated, at("2026-11-06 16:00"), at(until)).unwrap();
        assert!(week("2026-11-13 16:00").ends_with("**16:00** On the dot"));
        // The next run starts where this one ended: no line twice.
        let next = lines_between(dated, at("2026-11-13 16:00"), at("2026-11-20 16:00")).unwrap();
        assert_eq!(next, "## 2026-11-13\n\n**16:30** After it ran");
        // Changed, but nothing added in that time: left out.
        let none = lines_between(dated, at("2026-11-13 16:30"), at("2026-11-13 16:40")).unwrap();
        assert_eq!(none, "");
        // No day headings: it can't say.
        let plain = "# Inbox\n\n**10:00** A line";
        assert!(lines_between(plain, at("2026-11-06 16:00"), at("2026-11-13 16:00")).is_none());
        assert_eq!(shown("# Only a title"), "");
        assert_eq!(shown("No title\n\nJust text"), "No title\n\nJust text");
        assert_eq!(end_of("short", 100), ("short", false));
    }

    #[test]
    fn a_scheduled_hook_s_results_go_under_the_days_they_cover() {
        let week = Happened::scheduled(
            String::new(),
            at("2026-11-06 16:00"),
            at("2026-11-13 16:00"),
        );
        assert_eq!(week.period().as_deref(), Some("2026-11-06 to 2026-11-13"));
        let day = Happened::scheduled(
            String::new(),
            at("2026-11-12 17:30"),
            at("2026-11-13 17:30"),
        );
        assert_eq!(day.period().as_deref(), Some("2026-11-13"));
        assert!(Happened::phone("x", Local::now()).period().is_none());
        let json = serde_json::to_value(&week).unwrap();
        assert_eq!(json["event"], "schedule");
        assert!(
            json["since"]
                .as_str()
                .unwrap()
                .starts_with("2026-11-06T16:00")
        );
        assert!(week.env().iter().any(|(k, _)| *k == "SPLASHDOWN_SINCE"));
        // The sample for Test reads as the real thing does.
        assert!(SAMPLE.starts_with("# Pricing review\n[A meeting, "));
    }
}
