//! Hooks before a meeting ("10 minutes before a meeting on your
//! calendar"): when a calendar event is about to start, the hook gets the
//! event and the notes of the last meetings about it (ones with its title,
//! or with the same people), to write a brief to walk in with.
//!
//! The calendar is read on a Mac only, and only while a hook like this is
//! on; macOS asks for it the first time. Each event is seen to once. One
//! whose time passed while the computer slept still gets its hook for a
//! few minutes after it starts, and is let go after that. A hook that's
//! new, changed or turned back on starts with the events still ahead.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use splashdown_core::meeting::Turn;

use super::*;
use crate::notes::agents::{kind_of, made};
use crate::notes::store::Summary;

/// How often the calendar is looked at.
const EVERY: Duration = Duration::from_secs(30);
/// Which events each hook has been run for: `hook-calendar.json` in the
/// settings folder.
const FILE: &str = "hook-calendar.json";
/// How long after an event starts its hook still runs, in minutes: a brief
/// is some use to someone joining late, and none an hour in.
const LATE: i64 = 5;
/// How long an event that was seen to is remembered, in days.
const KEPT: i64 = 2;
/// The most earlier notes a hook gets: the last few say where things
/// stand, and a small model reads them at once.
const MOST: usize = 3;
/// How far ahead Test looks for a meeting to try a hook on, in days.
const TEST_AHEAD: i64 = 7;

/// A meeting and its earlier notes to try a hook on, for Test, when the
/// calendar has none.
pub(super) const SAMPLE: &str = "# Pricing review\n[The meeting about to start, Tuesday, 17 November 2026 at 14:00]\n\nWith Dana Lee, Sam Reyes.\n\n# Dana 1:1\n[An earlier meeting, Thursday, 12 November 2026 at 10:30]\n\nWith Dana Lee.\n\n## Summary\n\n### Decisions\n- Dana takes over the pricing page copy from Sam.\n\n### Action items\n- [ ] Send Dana the old copy doc today (You)\n- [ ] Share the hiring plan draft next week (Dana Lee)\n\n# Pricing review\n[An earlier meeting, Tuesday, 10 November 2026 at 14:00]\n\nWith Dana Lee, Sam Reyes.\n\n## Summary\n\n### Decisions\n- The pricing page goes live on Friday.\n\n### Action items\n- [ ] Send Dana the final numbers by Wednesday (You)\n- [ ] Write to support before the launch (You)\n- [ ] Check the EU prices with finance (Sam Reyes)\n";

/// A calendar event that starts soon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Event {
    /// The calendar's name for it: the same each time a repeating event
    /// comes round.
    pub id: String,
    pub starts: DateTime<Local>,
    pub all_day: bool,
    /// The user said no to it, or it was called off.
    pub declined: bool,
    pub title: String,
    /// The other people invited.
    pub people: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Seen {
    /// Each hook before a meeting that's on, by what it runs and when.
    #[serde(default)]
    hooks: BTreeMap<String, Briefed>,
}

/// What one hook has seen to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Briefed {
    /// When the hook was first seen: its events start after that.
    since: String,
    /// The events it was run for, each by a code made from the calendar's
    /// name for it and its day, with when it starts. No titles.
    #[serde(default)]
    events: BTreeMap<String, String>,
}

/// Starts watching the calendar, for as long as the app runs. It's only
/// looked at while a hook before a meeting is on.
pub fn watch(app: &AppHandle) {
    // The calendar is read on a Mac only.
    if !cfg!(target_os = "macos") {
        return;
    }
    let app = app.clone();
    let started = std::thread::Builder::new()
        .name("hook-calendar".into())
        .spawn(move || {
            loop {
                std::thread::sleep(EVERY);
                check(&app);
            }
        });
    if let Err(e) = started {
        tracing::error!("couldn't start watching the calendar for hooks: {e}");
    }
}

/// Whether the user has been told the calendar may not be read, so it's
/// said once and not every half minute.
static TOLD: AtomicBool = AtomicBool::new(false);

/// Runs the hooks whose meeting is about to start.
fn check(app: &AppHandle) {
    let Some(settings) = app.try_state::<AppState>().map(|s| s.settings()) else {
        return;
    };
    let Ok(path) = crate::data::path(FILE) else {
        return;
    };
    let on: Vec<&Hook> = settings
        .hooks
        .iter()
        .filter(|h| h.on && h.when == HookEvent::BeforeMeeting)
        .collect();
    if on.is_empty() && !path.exists() {
        return;
    }
    let now = Local::now();
    // Without a hook that's on the calendar isn't touched, nor asked for.
    let events = match on.iter().map(|h| h.minutes_before()).max() {
        None => Vec::new(),
        Some(most) => match events(
            now - chrono::Duration::minutes(LATE),
            now + chrono::Duration::minutes(i64::from(most) + 1),
        ) {
            Some(events) => {
                TOLD.store(false, Ordering::Relaxed);
                events
            }
            None => {
                if !TOLD.swap(true, Ordering::Relaxed) {
                    tracing::warn!("a hook before a meeting can't read the calendar");
                    runs::record(
                        app,
                        runs::Run {
                            at: now.to_rfc3339(),
                            event: HookEvent::BeforeMeeting.name().to_string(),
                            hook: name(on[0]),
                            about: None,
                            ok: false,
                            said: NO_ACCESS.into(),
                            took: 0.0,
                        },
                    );
                }
                return;
            }
        },
    };
    let mut seen: Seen = crate::data::read(&path);
    let before = seen.hooks.clone();
    let due = due(&mut seen.hooks, &settings.hooks, &events, now);
    // Before they run: a run cut short isn't done twice.
    if seen.hooks != before
        && let Err(e) = crate::data::write(&path, &seen)
    {
        tracing::warn!("couldn't keep which meetings hooks ran for: {e}");
        // Not kept, so they'd run again every half minute.
        return;
    }
    for (hook, event) in due {
        run(app, &settings, hook, &event);
    }
}

/// What's said when macOS doesn't let Splashdown read the calendar.
const NO_ACCESS: &str = "Splashdown may not read your calendar. Allow it in System Settings → Privacy & Security → Calendars.";

/// The events that start from `from` until `to`, in the order they start;
/// `None` when the calendar may not be read. macOS asks the first time.
#[cfg(target_os = "macos")]
fn events(from: DateTime<Local>, to: DateTime<Local>) -> Option<Vec<Event>> {
    use chrono::TimeZone;
    let now = Local::now();
    let seconds = |t: DateTime<Local>| (t - now).num_seconds() as f64;
    let found = tauri::async_runtime::block_on(async {
        if !splashdown_apple_shim::calendar_access().await {
            return None;
        }
        splashdown_apple_shim::calendar_upcoming(seconds(from), seconds(to)).await
    })?;
    Some(
        found
            .into_iter()
            .filter_map(|e| {
                Some(Event {
                    id: e.id,
                    starts: Local.timestamp_opt(e.starts, 0).single()?,
                    all_day: e.all_day,
                    declined: e.declined,
                    title: e.title,
                    people: e.people,
                })
            })
            .collect(),
    )
}

/// The calendar is read on a Mac only.
#[cfg(not(target_os = "macos"))]
fn events(_from: DateTime<Local>, _to: DateTime<Local>) -> Option<Vec<Event>> {
    None
}

/// The hooks whose event is about to start, each with its event; `seen` is
/// brought up to `now`. An event is a hook's once: from so many minutes
/// before it starts until a few after, when it's not all day or declined,
/// its title is one the hook is for, and it starts after the hook was
/// first seen.
fn due(
    seen: &mut BTreeMap<String, Briefed>,
    hooks: &[Hook],
    events: &[Event],
    now: DateTime<Local>,
) -> Vec<(Hook, Event)> {
    let time = |t: &str| {
        DateTime::parse_from_rfc3339(t)
            .ok()
            .map(|t| t.with_timezone(&Local))
    };
    let mut out = Vec::new();
    let mut kept = BTreeMap::new();
    for hook in hooks
        .iter()
        .filter(|h| h.on && h.when == HookEvent::BeforeMeeting)
    {
        let key = key(hook);
        let mut briefed = seen.get(&key).cloned().unwrap_or_default();
        // New, changed or turned on again: from the events still ahead. A
        // clock that was set back mustn't hold it up.
        let since = time(&briefed.since).map_or(now, |t| t.min(now));
        briefed.since = since.to_rfc3339();
        // Old ones are forgotten.
        briefed
            .events
            .retain(|_, starts| time(starts).is_some_and(|t| (now - t).num_days().abs() < KEPT));
        let ahead = chrono::Duration::minutes(i64::from(hook.minutes_before()));
        let mut ran: Vec<(String, DateTime<Local>)> = Vec::new();
        for event in events {
            let its_time = now >= event.starts - ahead
                && now < event.starts + chrono::Duration::minutes(LATE)
                && event.starts > since;
            if !its_time
                || event.all_day
                || event.declined
                || !hook.applies_to(HookEvent::BeforeMeeting, None, Some(&event.title))
            {
                continue;
            }
            let code = code(event);
            if briefed.events.contains_key(&code) {
                continue;
            }
            briefed.events.insert(code, event.starts.to_rfc3339());
            // The same meeting in two calendars is one meeting.
            let same = (event.title.trim().to_lowercase(), event.starts);
            if ran.contains(&same) {
                continue;
            }
            ran.push(same);
            out.push((hook.clone(), event.clone()));
        }
        kept.insert(key, briefed);
    }
    // Hooks that are gone or off are forgotten.
    *seen = kept;
    out
}

/// A hook before a meeting by what it runs, how long before, and for which
/// titles: changing any makes it new.
fn key(hook: &Hook) -> String {
    use sha2::Digest;
    let what = serde_json::to_string(&(&hook.run, hook.minutes_before(), &hook.title_has))
        .unwrap_or_default();
    hex(&sha2::Sha256::digest(what.as_bytes())[..8])
}

/// An event by the calendar's name for it and the day it's on, so one
/// that's moved to later that day isn't a new one, and each time a
/// repeating event comes round is. Not its title: this is kept in a file.
fn code(event: &Event) -> String {
    use sha2::Digest;
    let day = event.starts.format("%Y-%m-%d");
    let what = if event.id.is_empty() {
        format!("{}\n{}", event.title, event.starts.to_rfc3339())
    } else {
        format!("{}\n{day}", event.id)
    };
    hex(&sha2::Sha256::digest(what.as_bytes())[..8])
}

/// When a hook before a meeting runs, to end a sentence: "10 minutes
/// before a meeting on your calendar".
pub(super) fn words(hook: &Hook) -> String {
    match hook.minutes_before() {
        1 => "a minute before a meeting on your calendar".into(),
        minutes => format!("{minutes} minutes before a meeting on your calendar"),
    }
}

/// Runs `hook` for `event`, on the earlier notes about it.
fn run(app: &AppHandle, settings: &Settings, hook: Hook, event: &Event) {
    let nothing = |ok: bool, said: String| {
        runs::record(
            app,
            runs::Run {
                at: Local::now().to_rfc3339(),
                event: HookEvent::BeforeMeeting.name().to_string(),
                hook: name(&hook),
                about: Some(event.title.trim().to_string()),
                ok,
                said,
                took: 0.0,
            },
        );
    };
    match gathered(settings, &hook, event) {
        Ok(Some((happened, notes))) => {
            tracing::info!("a meeting is about to start, with {notes} earlier note(s)");
            run_later(app, hook, happened);
        }
        // Nothing to read: said in what ran, so it isn't a mystery.
        Ok(None) => nothing(true, "No earlier notes about this meeting".into()),
        Err(e) => {
            tracing::warn!("a hook before a meeting couldn't read the notes: {e}");
            nothing(false, e);
        }
    }
}

/// What a hook before `event` gets, and how many earlier notes that is;
/// `None` when there are none.
fn gathered(
    settings: &Settings,
    hook: &Hook,
    event: &Event,
) -> Result<Option<(Happened, usize)>, String> {
    let (store, _) = crate::notes::store_for(settings.notes.folder.clone())?;
    // Only notes made before the hook's time: not this meeting's own,
    // started early.
    let before = event.starts - chrono::Duration::minutes(i64::from(hook.minutes_before()));
    let (text, notes) = brief(&store.list(), event, before);
    Ok((notes > 0).then(|| (Happened::coming(text, event, Local::now()), notes)))
}

/// The next meeting on the calendar that a hook would run for and that has
/// earlier notes, as the hook would get it, for Test; with what that is in
/// words. `None` when there's none in the days ahead, the calendar may not
/// be read, or the hook isn't one before a meeting.
pub(super) fn next(app: &AppHandle, hook: &Hook) -> Option<(Happened, String)> {
    if hook.when != HookEvent::BeforeMeeting {
        return None;
    }
    let settings = app.try_state::<AppState>()?.settings();
    let now = Local::now();
    events(now, now + chrono::Duration::days(TEST_AHEAD))?
        .iter()
        .filter(|e| {
            !e.all_day
                && !e.declined
                && hook.applies_to(HookEvent::BeforeMeeting, None, Some(&e.title))
        })
        .find_map(|event| {
            let (happened, notes) = gathered(&settings, hook, event).ok()??;
            let earlier = if notes == 1 {
                "the last note".to_string()
            } else {
                format!("the last {notes} notes")
            };
            Some((
                happened,
                format!(
                    "your next meeting with earlier notes, {}, and {earlier} about it",
                    event.title.trim()
                ),
            ))
        })
}

/// What a hook before `event` gets, as one text, and how many earlier
/// notes are in it. The event comes first: its title, when it starts and
/// who's invited. Then the notes of the last meetings about it that were
/// made before `before`, newest first, each under its title and a line
/// saying what it is and when. A meeting that has a summary comes without
/// its transcript. Notes hidden from agents are left out.
fn brief(notes: &[Summary], event: &Event, before: DateTime<Local>) -> (String, usize) {
    let when = |at: DateTime<Local>| {
        format!(
            "{} at {}",
            splashdown_core::prompt::today(at.date_naive()),
            at.format("%H:%M")
        )
    };
    let mut out = format!(
        "# {}\n[The meeting about to start, {}]\n\n",
        event.title.trim(),
        when(event.starts)
    );
    if !event.people.is_empty() {
        out.push_str(&format!("With {}.\n\n", event.people.join(", ")));
    }
    let mut count = 0;
    for n in earlier(notes, event, before) {
        let Some(at) = DateTime::from_timestamp_millis(made(n)).map(|t| t.with_timezone(&Local))
        else {
            continue;
        };
        out.push_str(&format!(
            "# {}\n[An earlier meeting, {}]\n\n{}\n\n",
            n.title.trim(),
            when(at),
            schedule::shown(&n.text)
        ));
        count += 1;
    }
    (format!("{}\n", out.trim_end()), count)
}

/// The notes of the last meetings about `event`, newest first: meetings
/// made before `before` that have its title or mostly the same people,
/// that say something, and that aren't hidden from agents.
fn earlier<'a>(notes: &'a [Summary], event: &Event, before: DateTime<Local>) -> Vec<&'a Summary> {
    let until = before.timestamp_millis();
    let mut picked: Vec<&Summary> = notes
        .iter()
        .filter(|n| {
            kind_of(n) == "meeting"
                && !n.hidden_from_agents
                && made(n) < until
                && (same_title(&n.title, &event.title)
                    || same_people(&people_in(&n.text), &event.people))
                && !schedule::shown(&n.text).is_empty()
        })
        .collect();
    picked.sort_by_key(|n| std::cmp::Reverse(made(n)));
    picked.truncate(MOST);
    picked
}

/// A title as it's compared: its letters and digits in small letters, a
/// space between words.
fn plain(title: &str) -> String {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a note's title is the event's, whatever the capitals and
/// punctuation: a repeating meeting, named from the calendar each time.
/// "Meeting" alone is what a note is called when nothing names it, and
/// says nothing.
fn same_title(note: &str, event: &str) -> bool {
    let (note, event) = (plain(note), plain(event));
    !note.is_empty() && note != "meeting" && note == event
}

/// The people a meeting's note names: those its "With …" line says were
/// invited, and those who speak in its transcript by name.
fn people_in(body: &str) -> Vec<String> {
    let mut people = crate::meeting::voices::invited(body);
    for who in body
        .lines()
        .filter_map(Turn::parse)
        .map(|turn| turn.who.trim())
    {
        let numbered = who
            .strip_prefix("Speaker ")
            .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()));
        if !who.is_empty()
            && !numbered
            && !matches!(who, "You" | "Them")
            && !people.iter().any(|p| p.eq_ignore_ascii_case(who))
        {
            people.push(who.to_string());
        }
    }
    people
}

/// Whether a note's people are mostly the event's: at least half of the
/// longer list is in both. So a 1:1 finds the last 1:1 and a meeting of
/// three with that person, and not every all-hands they were at.
fn same_people(note: &[String], event: &[String]) -> bool {
    let shared = event
        .iter()
        .filter(|person| {
            note.iter()
                .any(|n| n.trim().eq_ignore_ascii_case(person.trim()))
        })
        .count();
    shared > 0 && shared * 2 >= note.len().max(event.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use splashdown_config::Before;

    fn at(s: &str) -> DateTime<Local> {
        let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    fn hook(name: &str, minutes: Option<u32>) -> Hook {
        Hook {
            when: HookEvent::BeforeMeeting,
            run: HookTarget::Ask {
                name: name.into(),
                instruction: "Write a brief for this meeting.".into(),
                put: PutIn::OtherNote,
                to: Some("Briefs".into()),
            },
            preset: None,
            title_has: None,
            on: true,
            trusted: true,
            notify: false,
            cloud_ok: None,
            schedule: None,
            before: minutes.map(|minutes| Before { minutes }),
        }
    }

    fn event(id: &str, title: &str, starts: &str, people: &[&str]) -> Event {
        Event {
            id: id.into(),
            starts: at(starts),
            all_day: false,
            declined: false,
            title: title.into(),
            people: people.iter().map(|p| p.to_string()).collect(),
        }
    }

    fn meeting(path: &str, created: &str, text: &str) -> Summary {
        Summary {
            path: path.into(),
            title: crate::notes::markdown::title_of(text),
            text: text.into(),
            modified: at(created).timestamp_millis() as u64,
            pinned: false,
            folder: "Meetings".into(),
            source: "meeting".into(),
            tags: Vec::new(),
            created: Some(at(created).to_rfc3339()),
            hidden_from_agents: false,
        }
    }

    #[test]
    fn a_hook_runs_once_when_its_meeting_is_about_to_start() {
        let brief = hook("Brief", None);
        let hooks = vec![brief.clone()];
        let review = event("A1", "Pricing review", "2026-11-17 14:00", &["Dana Lee"]);
        let events = vec![review.clone()];
        let mut seen = BTreeMap::new();
        // Added in the morning: nothing is due yet.
        assert!(due(&mut seen, &hooks, &events, at("2026-11-17 09:00")).is_empty());
        assert_eq!(seen.len(), 1);
        assert!(due(&mut seen, &hooks, &events, at("2026-11-17 13:49")).is_empty());
        // Ten minutes before, by default.
        let ran = due(&mut seen, &hooks, &events, at("2026-11-17 13:50"));
        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].1, review);
        // Never twice, not even once it's moved to later that day.
        assert!(due(&mut seen, &hooks, &events, at("2026-11-17 13:51")).is_empty());
        let moved = vec![event("A1", "Pricing review", "2026-11-17 16:00", &[])];
        assert!(due(&mut seen, &hooks, &moved, at("2026-11-17 15:55")).is_empty());
        // The next time it comes round, it runs again.
        let next_week = vec![event("A1", "Pricing review", "2026-11-24 14:00", &[])];
        assert!(due(&mut seen, &hooks, &next_week, at("2026-11-24 13:40")).is_empty());
        assert_eq!(
            due(&mut seen, &hooks, &next_week, at("2026-11-24 13:55")).len(),
            1
        );
        // And last week's is forgotten.
        assert_eq!(seen[&key(&brief)].events.len(), 1);

        // How long before is the hook's own.
        let early = vec![hook("Brief", Some(30))];
        let mut seen = BTreeMap::new();
        assert!(due(&mut seen, &early, &events, at("2026-11-17 13:00")).is_empty());
        assert!(due(&mut seen, &early, &events, at("2026-11-17 13:29")).is_empty());
        assert_eq!(
            due(&mut seen, &early, &events, at("2026-11-17 13:30")).len(),
            1
        );
        assert_eq!(
            words(&early[0]),
            "30 minutes before a meeting on your calendar"
        );
        assert_eq!(
            words(&hook("Brief", Some(1))),
            "a minute before a meeting on your calendar"
        );
        assert_ne!(key(&early[0]), key(&brief));
        assert_eq!(
            key(&Hook {
                trusted: false,
                ..brief.clone()
            }),
            key(&brief)
        );
    }

    #[test]
    fn a_meeting_that_started_is_let_go_and_a_new_hook_starts_ahead() {
        let brief = hook("Brief", None);
        let hooks = vec![brief.clone()];
        let events = vec![
            event("A1", "Standup", "2026-11-17 09:00", &[]),
            event("B2", "Pricing review", "2026-11-17 14:00", &[]),
        ];
        // Asleep until two minutes into the meeting: still worth having.
        let mut seen = BTreeMap::new();
        assert!(due(&mut seen, &hooks, &events, at("2026-11-17 08:00")).is_empty());
        let ran = due(&mut seen, &hooks, &events, at("2026-11-17 09:02"));
        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].1.title, "Standup");
        // Asleep until well into it: let go, for good.
        let mut seen = BTreeMap::new();
        assert!(due(&mut seen, &hooks, &events, at("2026-11-17 08:00")).is_empty());
        assert!(due(&mut seen, &hooks, &events, at("2026-11-17 09:05")).is_empty());

        // A hook added two minutes into a meeting isn't for that one, but
        // is for one still ahead, even inside its ten minutes.
        let mut seen = BTreeMap::new();
        assert!(due(&mut seen, &hooks, &events, at("2026-11-17 09:02")).is_empty());
        let mut seen = BTreeMap::new();
        let ran = due(&mut seen, &hooks, &events, at("2026-11-17 13:57"));
        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].1.title, "Pricing review");

        // Off, it's forgotten; on again, it starts with what's ahead.
        let off = vec![Hook {
            on: false,
            ..brief.clone()
        }];
        assert!(due(&mut seen, &off, &events, at("2026-11-17 13:58")).is_empty());
        assert!(seen.is_empty());
        assert_eq!(
            due(&mut seen, &hooks, &events, at("2026-11-17 13:59")).len(),
            1
        );
        // A clock set back doesn't hold it up for good.
        let mut ahead = BTreeMap::from([(
            key(&brief),
            Briefed {
                since: at("2027-06-01 00:00").to_rfc3339(),
                events: BTreeMap::from([("old".to_string(), at("2027-05-31 10:00").to_rfc3339())]),
            },
        )]);
        assert!(due(&mut ahead, &hooks, &events, at("2026-11-17 12:00")).is_empty());
        assert!(ahead[&key(&brief)].events.is_empty());
        assert_eq!(
            due(&mut ahead, &hooks, &events, at("2026-11-17 13:50")).len(),
            1
        );
    }

    #[test]
    fn some_events_never_run_a_hook() {
        let all = hook("Brief", None);
        let one_on_ones = Hook {
            title_has: Some("1:1".into()),
            ..hook("1:1 brief", None)
        };
        let hooks = vec![all, one_on_ones];
        let events = vec![
            Event {
                all_day: true,
                ..event("A", "Offsite", "2026-11-17 14:00", &[])
            },
            Event {
                declined: true,
                ..event("B", "All hands", "2026-11-17 14:00", &[])
            },
            event("C", "Dana 1:1", "2026-11-17 14:00", &["Dana Lee"]),
            // The same meeting again, from a second calendar.
            event("D", "dana 1:1", "2026-11-17 14:00", &["Dana Lee"]),
            event("E", "Pricing review", "2026-11-17 14:05", &[]),
        ];
        let mut seen = BTreeMap::new();
        assert!(due(&mut seen, &hooks, &events, at("2026-11-17 12:00")).is_empty());
        let ran = due(&mut seen, &hooks, &events, at("2026-11-17 13:56"));
        let got: Vec<(String, &str)> = ran
            .iter()
            .map(|(hook, event)| (name(hook), event.title.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("Brief".to_string(), "Dana 1:1"),
                ("Brief".to_string(), "Pricing review"),
                ("1:1 brief".to_string(), "Dana 1:1"),
            ]
        );
        assert!(due(&mut seen, &hooks, &events, at("2026-11-17 13:57")).is_empty());
        // What's kept names no meeting.
        let kept = serde_json::to_string(&seen).unwrap();
        assert!(
            !kept.contains("Dana") && !kept.contains("Pricing"),
            "{kept}"
        );
        // No hook before a meeting: nothing, and nothing kept.
        let mut seen = BTreeMap::new();
        let weekly = Hook {
            when: HookEvent::Schedule,
            ..hook("Weekly review", None)
        };
        assert!(due(&mut seen, &[weekly], &events, at("2026-11-17 13:56")).is_empty());
        assert!(seen.is_empty());
    }

    #[test]
    fn a_hook_gets_the_meeting_and_the_last_notes_about_it() {
        const LAST: &str = "# Pricing review\n\nWith Dana Lee, Sam Reyes.\n\n## My notes\n\nAsk Sam about the EU prices.\n\n## Summary\n\nThe page goes live on Friday.\n\n**[0:01] You:** hello\n\n**[0:05] Dana Lee:** Friday works\n";
        const ONE_ON_ONE: &str = "# Dana 1:1\n\nWith Dana Lee.\n\n**[0:01] You:** I'll send you the old copy doc today.\n";
        const NAMED: &str = "# Zoom meeting\n\n## Summary\n\nSam checks the EU prices.\n\n**[0:01] Sam Reyes:** I'll check\n\n**[0:04] Dana Lee:** thanks\n\n**[0:06] Speaker 3:** bye\n";
        const ALL_HANDS: &str = "# All hands\n\nWith Dana Lee, Sam Reyes, Kim Park, Lee Wong, Ana Diaz.\n\n## Summary\n\nThe year in review.\n";
        let old = |day: u32| {
            meeting(
                &format!("Meetings/2026-10-{day} Pricing review.md"),
                &format!("2026-10-{day} 14:00"),
                &format!("# Pricing review\n\n## Summary\n\nFrom October {day}.\n"),
            )
        };
        let hidden = Summary {
            hidden_from_agents: true,
            ..meeting(
                "Meetings/Secret.md",
                "2026-11-16 09:00",
                "# Pricing review\n\n## Summary\n\nThe secret price.\n",
            )
        };
        let a_note = Summary {
            source: String::new(),
            ..meeting(
                "Pricing review.md",
                "2026-11-15 09:00",
                "# Pricing review\n\nJust thoughts.\n",
            )
        };
        let notes = vec![
            old(13),
            old(20),
            meeting("Meetings/Last.md", "2026-11-10 14:03", LAST),
            meeting("Meetings/Dana.md", "2026-11-12 10:31", ONE_ON_ONE),
            meeting("Meetings/Zoom.md", "2026-11-05 11:00", NAMED),
            meeting("Meetings/All hands.md", "2026-11-11 16:00", ALL_HANDS),
            // This meeting's own note, started early.
            meeting(
                "Meetings/Now.md",
                "2026-11-17 13:55",
                "# Pricing review\n\nWith Dana Lee, Sam Reyes.\n\n**[0:01] You:** testing\n",
            ),
            meeting("Meetings/Empty.md", "2026-11-14 09:00", "# Pricing review"),
            hidden,
            a_note,
        ];
        let coming = event(
            "A1",
            "Pricing Review",
            "2026-11-17 14:00",
            &["Dana Lee", "Sam Reyes"],
        );
        let (text, count) = brief(&notes, &coming, at("2026-11-17 13:50"));
        assert_eq!(count, 3, "{text}");
        // The meeting first, then the last three about it, newest first.
        assert!(
            text.starts_with(
                "# Pricing Review\n[The meeting about to start, Tuesday, 17 November 2026 at 14:00]\n\nWith Dana Lee, Sam Reyes.\n\n# Dana 1:1\n[An earlier meeting, Thursday, 12 November 2026 at 10:31]\n\n"
            ),
            "{text}"
        );
        let last = text
            .find("# Pricing review\n[An earlier meeting, Tuesday, 10 November 2026 at 14:03]")
            .unwrap();
        let named = text
            .find("# Zoom meeting\n[An earlier meeting, Thursday, 5 November 2026 at 11:00]")
            .unwrap();
        assert!(last < named, "{text}");
        // A summary and the user's own notes stand in for the transcript;
        // without a summary, the transcript.
        assert!(
            text.contains("Ask Sam about the EU prices.") && text.contains("goes live on Friday")
        );
        assert!(!text.contains("Friday works"));
        assert!(text.contains("I'll send you the old copy doc today."));
        // Older ones, another kind of note, a hidden one, an empty one,
        // this meeting's own and one with too many other people: left out.
        for out in [
            "October",
            "Just thoughts",
            "secret",
            "testing",
            "year in review",
        ] {
            assert!(!text.contains(out), "{out}: {text}");
        }
        assert!(text.ends_with("Sam checks the EU prices.\n"), "{text}");

        // No one invited: the title alone finds them. Nothing found: none.
        let alone = event("B2", "pricing review!", "2026-11-17 14:00", &[]);
        let (text, count) = brief(&notes, &alone, at("2026-11-17 13:50"));
        assert_eq!(count, 3);
        assert!(text.contains(
            "[The meeting about to start, Tuesday, 17 November 2026 at 14:00]\n\n# Pricing review\n"
        ));
        assert!(text.contains("From October 20.") && text.contains("From October 13."));
        let new = event("C3", "Kickoff", "2026-11-17 14:00", &["Pat Moss"]);
        let (text, count) = brief(&notes, &new, at("2026-11-17 13:50"));
        assert_eq!(count, 0);
        assert_eq!(
            text,
            "# Kickoff\n[The meeting about to start, Tuesday, 17 November 2026 at 14:00]\n\nWith Pat Moss.\n"
        );
    }

    #[test]
    fn notes_are_about_a_meeting_by_title_or_people() {
        assert!(same_title("Pricing review", " pricing  Review "));
        assert!(same_title("Dana 1:1", "Dana 1-1"));
        assert!(!same_title("Pricing review", "Pricing"));
        assert!(!same_title("Meeting", "Meeting") && !same_title("", ""));
        let names = |list: &[&str]| list.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        let dana = names(&["Dana Lee"]);
        assert!(same_people(&names(&["dana lee"]), &dana));
        assert!(same_people(&names(&["Dana Lee", "Sam Reyes"]), &dana));
        assert!(!same_people(
            &names(&["Dana Lee", "Sam Reyes", "Kim Park"]),
            &dana
        ));
        assert!(!same_people(&names(&["Sam Reyes"]), &dana));
        assert!(!same_people(&[], &dana) && !same_people(&dana, &[]));
        assert_eq!(
            people_in(
                "# Sync\n\nWith Dana Lee.\n\n**[0:01] You:** hi\n\n**[0:02] Them:** hi\n\n**[0:03] Sam Reyes:** hi\n\n**[0:04] Dana Lee:** hi\n\n**[0:05] Speaker 2:** hi\n"
            ),
            ["Dana Lee", "Sam Reyes"]
        );
    }

    #[test]
    fn a_hook_before_a_meeting_gets_the_event_s_details() {
        let coming = event(
            "A1",
            " Pricing review ",
            "2026-11-17 14:00",
            &["Dana Lee", "Sam Reyes"],
        );
        let happened =
            Happened::coming("# Pricing review\n".into(), &coming, at("2026-11-17 13:50"));
        let json = serde_json::to_value(&happened).unwrap();
        assert_eq!(json["event"], "before-meeting");
        assert_eq!(json["meeting"]["title"], "Pricing review");
        assert!(
            json["meeting"]["starts"]
                .as_str()
                .unwrap()
                .starts_with("2026-11-17T14:00")
        );
        assert_eq!(json["meeting"]["people"][1], "Sam Reyes");
        assert!(json["at"].as_str().unwrap().starts_with("2026-11-17T13:50"));
        assert!(json.get("since").is_none() && json["note"].is_null());
        let env = happened.env();
        assert!(env.contains(&("SPLASHDOWN_TITLE", "Pricing review".into())));
        assert!(env.contains(&("SPLASHDOWN_PEOPLE", "Dana Lee, Sam Reyes".into())));
        assert!(
            env.iter()
                .any(|(k, v)| *k == "SPLASHDOWN_STARTS" && v.starts_with("2026-11-17T14:00"))
        );
        // Other hooks say nothing of a meeting to come.
        let phone = Happened::phone("x", Local::now());
        assert!(
            serde_json::to_value(&phone)
                .unwrap()
                .get("meeting")
                .is_none()
        );
        assert!(!phone.env().iter().any(|(k, _)| *k == "SPLASHDOWN_STARTS"));
        // The sample for Test reads as the real thing does.
        assert!(SAMPLE.starts_with("# Pricing review\n[The meeting about to start, "));
        assert!(SAMPLE.contains("\n[An earlier meeting, "));
    }
}
