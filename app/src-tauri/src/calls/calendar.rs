//! The offer for a meeting on the calendar: when an event that has a link
//! to a video call is about to start, the offer panel shows its title with
//! Join, which opens the link, and Record. One place to join and start
//! recording, before any call app has touched the microphone.
//!
//! Off unless it's turned on (Settings → Notes → Meetings), and on a Mac
//! only, where the calendar is read; macOS asks for it the first time.
//! While it's on, the events that start soon are looked at every half
//! minute. Each event is offered once: which ones were is kept in a file,
//! so not again after a restart either. An event without a link gets no
//! offer (the offer for a call still covers it), and nothing is offered
//! while something records.
//!
//! The link is only ever one to a known video-call service
//! (`splashdown_core::call_link`): an event's notes are written by whoever
//! sent the invitation.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Local};
use serde::Deserialize;
use splashdown_core::call_link;

use super::*;
use crate::hooks::calendar::{Event, code, events};

/// How often the calendar is looked at.
const EVERY: Duration = Duration::from_secs(30);
/// Which events were offered: `calendar-offers.json` in the settings
/// folder.
const FILE: &str = "calendar-offers.json";
/// How long before an event starts it's offered, in minutes.
const AHEAD: i64 = 2;
/// How long after an event starts it's still offered, in minutes: the
/// computer was asleep, or a recording has just ended.
const LATE: i64 = 5;
/// How long an event that was offered is remembered, in days.
const KEPT: i64 = 2;
/// The panel stays up this long: from two minutes before the meeting
/// until a minute into it.
const SHOW_FOR: Duration = Duration::from_secs(3 * 60);
/// ...and this long after Join, for Record once the call is open.
const AFTER_JOIN: Duration = Duration::from_secs(90);
/// After the panel was answered, a call app that takes the microphone
/// within this long is that meeting, and gets no offer of its own.
const HELD: Duration = Duration::from_secs(10 * 60);
/// A call that was offered at most this long ago is taken to be the
/// meeting that's starting, joined early.
const EARLY: Duration = Duration::from_secs(10 * 60);

/// The events that were offered, each by a code made from the calendar's
/// name for it and its day, with when it starts. No titles or links.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Seen {
    #[serde(default)]
    events: BTreeMap<String, String>,
}

/// When the calendar's offer was last answered: Join, Record or Not now.
static ANSWERED: Mutex<Option<Instant>> = Mutex::new(None);

/// Whether the user has been told the calendar may not be read, so it's
/// said once and not every half minute.
static TOLD: AtomicBool = AtomicBool::new(false);

/// Starts watching the calendar, for as long as the app runs. It's only
/// looked at while the offer is turned on.
pub fn watch(app: &AppHandle) {
    // The calendar is read on a Mac only.
    if !cfg!(target_os = "macos") {
        return;
    }
    let app = app.clone();
    let started = std::thread::Builder::new()
        .name("calendar-offer".into())
        .spawn(move || {
            loop {
                std::thread::sleep(EVERY);
                check(&app);
            }
        });
    if let Err(e) = started {
        tracing::warn!("couldn't watch the calendar for meetings to join: {e}");
    }
}

/// Offers the meeting that's about to start, if there's one.
fn check(app: &AppHandle) {
    let Some(settings) = app.try_state::<AppState>().map(|s| s.settings()) else {
        return;
    };
    // Turned off, the calendar isn't touched, nor asked for.
    if !settings.notes.offer_on_calendar {
        return;
    }
    // Not while something records, and one offer at a time: the event
    // waits, and is offered if it's still starting when that's over.
    if crate::meeting::is_running() || offered_now() {
        return;
    }
    let Ok(path) = crate::data::path(FILE) else {
        return;
    };
    let now = Local::now();
    let Some(events) = events(
        now - chrono::Duration::minutes(LATE),
        now + chrono::Duration::minutes(AHEAD + 1),
    ) else {
        if !TOLD.swap(true, Ordering::Relaxed) {
            tracing::warn!("the offer to join meetings can't read the calendar");
        }
        return;
    };
    TOLD.store(false, Ordering::Relaxed);
    let in_call = CALL_OFFERED.locked().is_some_and(|at| at.elapsed() < EARLY);
    let mut seen: Seen = crate::data::read(&path);
    let before = seen.events.clone();
    let event = due(&mut seen.events, &events, now, in_call);
    // Before it's shown: an offer cut short isn't made twice.
    if seen.events != before
        && let Err(e) = crate::data::write(&path, &seen)
    {
        tracing::warn!("couldn't keep which meetings were offered: {e}");
        // Not kept, so it'd be offered again every half minute.
        return;
    }
    if let Some(event) = event {
        offer(app, &event);
    }
}

/// The event to offer now, if any; `seen` is brought up to `now`. An event
/// is offered once: from two minutes before it starts until a few after,
/// when it has a link to a video call and isn't all day or declined. Of
/// two meetings that start together the second waits its turn. `in_call`:
/// a call was just offered to be recorded, so the meeting that's starting
/// has had its offer, and is let go without another.
fn due(
    seen: &mut BTreeMap<String, String>,
    events: &[Event],
    now: DateTime<Local>,
    in_call: bool,
) -> Option<Event> {
    // Old ones are forgotten.
    seen.retain(|_, starts| {
        DateTime::parse_from_rfc3339(starts)
            .is_ok_and(|t| (now - t.with_timezone(&Local)).num_days().abs() < KEPT)
    });
    let mut first: Option<&Event> = None;
    for event in events {
        let its_time = now >= event.starts - chrono::Duration::minutes(AHEAD)
            && now < event.starts + chrono::Duration::minutes(LATE);
        if !its_time || event.all_day || event.declined || event.link.is_none() {
            continue;
        }
        let code = code(event);
        if seen.contains_key(&code) {
            continue;
        }
        // The same meeting in two calendars is one meeting; another
        // meeting is offered once this one's panel is gone.
        if first.is_some_and(|first| !same_meeting(first, event)) {
            continue;
        }
        seen.insert(code, event.starts.to_rfc3339());
        if !in_call && first.is_none() {
            first = Some(event);
        }
    }
    first.cloned()
}

/// Whether two events are one meeting: the same link, or the same title
/// at the same time.
fn same_meeting(a: &Event, b: &Event) -> bool {
    a.link == b.link
        || (a.starts == b.starts && a.title.trim().to_lowercase() == b.title.trim().to_lowercase())
}

/// Puts the offer for `event` on screen.
fn offer(app: &AppHandle, event: &Event) {
    let Some(link) = &event.link else {
        return;
    };
    // (Not its title or link: this goes in the log.)
    tracing::info!(
        "a meeting on the calendar is about to start, with a {} link",
        link.service
    );
    let panel = Panel {
        event: Some(crate::meeting::Event {
            title: event.title.trim().to_string(),
            people: event.people.clone(),
        }),
        starts: Some(event.starts.timestamp_millis()),
        link: Some(link.url.clone()),
        seconds: SHOW_FOR.as_secs(),
        ..panel(
            app,
            Offer::Join {
                service: link.service,
            },
        )
    };
    show(app, panel);
}

/// Whether the offer to join is on screen.
pub(super) fn offered_now() -> bool {
    matches!(shown().as_ref(), Some((panel, _)) if matches!(panel.offer, Offer::Join { .. }))
}

/// Whether the calendar's offer has the meeting that's starting: it's on
/// screen, or was answered a short while ago. A call app that takes the
/// microphone meanwhile is that meeting, and isn't offered again.
pub(super) fn has_the_call() -> bool {
    offered_now() || ANSWERED.locked().is_some_and(|at| at.elapsed() < HELD)
}

/// The buttons of the offer to join. "join" opens the event's link and
/// leaves the panel up a while longer, for Record; "record" starts the
/// meeting, named from the event; anything else dismisses it.
pub(super) fn answer(app: &AppHandle, panel: Panel, answer: &str) -> Result<(), String> {
    *ANSWERED.locked() = Some(Instant::now());
    match answer {
        "join" => {
            // Checked again where it's opened: only ever a link to a
            // known video-call service.
            let link = panel
                .link
                .as_deref()
                .and_then(call_link::known)
                .ok_or("This meeting has no link to join.")?;
            splashdown_platform::open_url(&link.url).map_err(|e| e.to_string())?;
            show(
                app,
                Panel {
                    joined: true,
                    seconds: AFTER_JOIN.as_secs(),
                    ..panel
                },
            );
            Ok(())
        }
        "record" => {
            hide(app);
            match panel.event {
                Some(event) => crate::meeting::start_for(app, event),
                None => crate::meeting::start(app),
            }
            .map(|_| ())
        }
        _ => {
            hide(app);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use splashdown_core::call_link::CallLink;

    fn at(s: &str) -> DateTime<Local> {
        let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    /// An event, with what its URL, location and notes say.
    fn event(id: &str, title: &str, starts: &str, place: &str) -> Event {
        Event {
            id: id.into(),
            starts: at(starts),
            all_day: false,
            declined: false,
            title: title.into(),
            link: call_link::find(place),
            people: vec!["Dana Lee".into()],
        }
    }

    const ZOOM: &str = "Join Zoom Meeting https://us02web.zoom.us/j/5551234567?pwd=abc";

    #[test]
    fn a_meeting_with_a_link_is_offered_once_as_it_starts() {
        let review = event("A1", "Pricing review", "2026-11-17 14:00", ZOOM);
        let events = vec![review.clone()];
        let mut seen = BTreeMap::new();
        // Not in the morning, nor three minutes before.
        assert_eq!(due(&mut seen, &events, at("2026-11-17 09:00"), false), None);
        assert_eq!(due(&mut seen, &events, at("2026-11-17 13:57"), false), None);
        assert!(seen.is_empty());
        // Two minutes before, with its link.
        let offered = due(&mut seen, &events, at("2026-11-17 13:58"), false);
        assert_eq!(offered, Some(review));
        assert_eq!(
            offered.unwrap().link,
            Some(CallLink {
                url: "https://us02web.zoom.us/j/5551234567?pwd=abc".into(),
                service: "Zoom",
            })
        );
        // Never twice: not after Not now, Join or Record, which all leave
        // it seen to.
        assert_eq!(due(&mut seen, &events, at("2026-11-17 13:59"), false), None);
        assert_eq!(due(&mut seen, &events, at("2026-11-17 14:03"), false), None);
        // Not after the app starts again in the middle of the meeting,
        // with what was kept in the file.
        let kept = serde_json::to_string(&Seen {
            events: seen.clone(),
        })
        .unwrap();
        let mut again: Seen = serde_json::from_str(&kept).unwrap();
        assert_eq!(
            due(&mut again.events, &events, at("2026-11-17 14:02"), false),
            None
        );
        // What's kept names no meeting and has no link.
        assert!(
            !kept.contains("Pricing") && !kept.contains("zoom") && !kept.contains("Dana"),
            "{kept}"
        );
        // Not when it's moved to later that day.
        let moved = vec![event("A1", "Pricing review", "2026-11-17 16:00", ZOOM)];
        assert_eq!(due(&mut seen, &moved, at("2026-11-17 15:59"), false), None);
        // The next time it comes round, it's offered again, and last
        // week's is forgotten.
        let next_week = vec![event("A1", "Pricing review", "2026-11-24 14:00", ZOOM)];
        assert_eq!(
            due(&mut seen, &next_week, at("2026-11-24 13:50"), false),
            None
        );
        assert!(due(&mut seen, &next_week, at("2026-11-24 13:59"), false).is_some());
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn a_meeting_that_started_is_offered_for_a_few_minutes() {
        let events = vec![event("A1", "Standup", "2026-11-17 09:00", ZOOM)];
        // Asleep, or recording the meeting before, until four minutes in.
        let mut seen = BTreeMap::new();
        assert!(due(&mut seen, &events, at("2026-11-17 09:04"), false).is_some());
        assert_eq!(due(&mut seen, &events, at("2026-11-17 09:04"), false), None);
        // Five minutes in, it's let go.
        let mut seen = BTreeMap::new();
        assert_eq!(due(&mut seen, &events, at("2026-11-17 09:05"), false), None);
        assert_eq!(due(&mut seen, &events, at("2026-11-17 10:00"), false), None);
        assert!(seen.is_empty());
    }

    #[test]
    fn some_events_are_never_offered() {
        let events = vec![
            // No link, or none that may be opened.
            event("A", "Lunch", "2026-11-17 14:00", "The place on the corner"),
            event(
                "B",
                "Webinar",
                "2026-11-17 14:00",
                "http://zoom.us/j/5551234567",
            ),
            event(
                "C",
                "Prize draw",
                "2026-11-17 14:00",
                "Join: https://zoom.us.example.com/j/1 or https://evilzoom.us/j/1 or https://example.com/?to=https://zoom.us/j/1",
            ),
            Event {
                all_day: true,
                ..event("D", "Offsite", "2026-11-17 14:00", ZOOM)
            },
            Event {
                declined: true,
                ..event("E", "All hands", "2026-11-17 14:00", ZOOM)
            },
        ];
        let mut seen = BTreeMap::new();
        for minute in ["13:58", "14:00", "14:04"] {
            let now = at(&format!("2026-11-17 {minute}"));
            assert_eq!(due(&mut seen, &events, now, false), None, "{minute}");
        }
        // Nothing is kept about them.
        assert!(seen.is_empty());
    }

    #[test]
    fn two_meetings_at_once_take_turns_and_a_copy_is_the_same_meeting() {
        let events = vec![
            event("A", "Pricing review", "2026-11-17 14:00", ZOOM),
            // Another meeting at the same time.
            event(
                "B",
                "Dana 1:1",
                "2026-11-17 14:00",
                "https://meet.google.com/abc-defg-hij",
            ),
            // The first again, from a second calendar: by its link, and
            // by its title and time.
            event("C", "Fwd: Pricing review", "2026-11-17 14:00", ZOOM),
            event(
                "D",
                "pricing review ",
                "2026-11-17 14:00",
                "https://zoom.us/j/5551234567",
            ),
        ];
        let mut seen = BTreeMap::new();
        let first = due(&mut seen, &events, at("2026-11-17 13:58"), false).unwrap();
        assert_eq!(first.title, "Pricing review");
        assert_eq!(seen.len(), 3);
        // Once the panel is free, the other one.
        let second = due(&mut seen, &events, at("2026-11-17 14:01"), false).unwrap();
        assert_eq!(second.title, "Dana 1:1");
        assert_eq!(second.link.unwrap().service, "Google Meet");
        assert_eq!(due(&mut seen, &events, at("2026-11-17 14:02"), false), None);
    }

    #[test]
    fn a_meeting_whose_call_was_just_offered_is_let_go() {
        let events = vec![
            event("A", "Pricing review", "2026-11-17 14:00", ZOOM),
            event(
                "B",
                "Dana 1:1",
                "2026-11-17 14:00",
                "https://meet.google.com/abc-defg-hij",
            ),
        ];
        // Joined early: the call app took the microphone, and recording
        // it was offered then. No second offer when the event comes due,
        // nor later.
        let mut seen = BTreeMap::new();
        assert_eq!(due(&mut seen, &events, at("2026-11-17 13:57"), true), None);
        assert!(seen.is_empty());
        assert_eq!(due(&mut seen, &events, at("2026-11-17 13:58"), true), None);
        assert_eq!(seen.len(), 2);
        assert_eq!(due(&mut seen, &events, at("2026-11-17 14:01"), false), None);
    }
}
