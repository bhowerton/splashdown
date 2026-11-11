//! Noticing calls: when a call app (Zoom, Teams, Meet in a browser…)
//! starts using the microphone, a small panel offers to record the meeting;
//! when the app it was recording stops using it, the panel offers to stop.
//! Nothing is recorded until the user clicks. Settings → Meetings
//! turns the offer off.
//!
//! The same panel offers a meeting on the calendar that's about to start,
//! with Join (`calls/calendar.rs`). The two share it, so one meeting gets
//! one offer: while the calendar's is up, and for a while after it was
//! answered, a call app that takes the microphone gets none of its own;
//! and a call offered a short while ago is taken to be the meeting that's
//! starting, which the calendar then leaves alone.

pub mod calendar;

use splashdown_core::sync::Locked;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::state::AppState;

pub const WINDOW: &str = "call-offer";
/// App event with what the panel should show (`Panel`).
const CHANGED: &str = "call-offer-changed";
/// How often the microphone is checked.
const POLL: Duration = Duration::from_secs(2);
/// A call app has to keep the microphone this long before the offer, so a
/// quick microphone check doesn't count.
const SETTLE: Duration = Duration::from_secs(4);
/// The recorded call app has to stay off the microphone this long before
/// the offer to stop (muting doesn't usually release it).
const ENDED_AFTER: Duration = Duration::from_secs(20);
/// The panel hides itself after this long.
const SHOW_FOR: Duration = Duration::from_secs(45);

/// Apps that make calls, by bundle ID prefix (macOS) or executable name or
/// package family name prefix (Windows), and what to call them. A browser
/// using the microphone is most likely a web meeting.
const CALL_APPS: &[(&str, &str)] = &[
    ("us.zoom.", "Zoom"),
    ("zoom.exe", "Zoom"),
    ("com.microsoft.teams", "Teams"),
    ("ms-teams.exe", "Teams"),
    ("teams.exe", "Teams"),
    ("msteams_", "Teams"),
    ("com.tinyspeck.slackmacgap", "Slack"),
    ("slack.exe", "Slack"),
    ("com.apple.facetime", "FaceTime"),
    ("com.apple.avconferenced", "FaceTime"),
    ("com.cisco.webex", "Webex"),
    ("cisco-systems.spark", "Webex"),
    ("ciscocollabhost.exe", "Webex"),
    ("atmgr.exe", "Webex"),
    ("com.hnc.discord", "Discord"),
    ("discord.exe", "Discord"),
    ("net.whatsapp.whatsapp", "WhatsApp"),
    ("desktop.whatsapp", "WhatsApp"),
    ("5319275a.whatsappdesktop_", "WhatsApp"),
    ("com.skype.", "Skype"),
    ("skype.exe", "Skype"),
    ("com.google.chrome", "Chrome"),
    ("chrome.exe", "Chrome"),
    ("com.microsoft.edgemac", "Edge"),
    ("msedge.exe", "Edge"),
    ("company.thebrowser.", "Arc"),
    ("arc.exe", "Arc"),
    ("com.brave.browser", "Brave"),
    ("brave.exe", "Brave"),
    ("org.mozilla.firefox", "Firefox"),
    ("firefox.exe", "Firefox"),
    ("com.apple.safari", "Safari"),
    ("com.apple.webkit.gpu", "Safari"),
];

/// The call app a microphone user belongs to, if it is one.
fn call_app(id: &str) -> Option<&'static str> {
    let id = id.to_lowercase();
    CALL_APPS
        .iter()
        .find(|(prefix, _)| id.starts_with(prefix))
        .map(|(_, name)| *name)
}

/// What the panel offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Offer {
    /// `app` started using the microphone: record?
    Record { app: &'static str },
    /// The recorded `app` stopped using it: stop?
    Stop { app: &'static str },
    /// A calendar event with a link to a call on `service` is about to
    /// start: join, record?
    Join { service: &'static str },
}

/// What the panel shows: the offer, and what to know before answering.
#[derive(Debug, Clone, Serialize)]
pub struct Panel {
    offer: Offer,
    /// The calendar event on now (macOS, with calendar naming on), so the
    /// user can tell it's the right call; or the one that's about to start.
    event: Option<crate::meeting::Event>,
    /// For the offer to join: when the event starts, in milliseconds since
    /// 1970.
    starts: Option<i64>,
    /// For the offer to join: Join was clicked, and Record is what's left.
    joined: bool,
    /// For the offer to join: the link Join opens. Kept here; the page
    /// never gets it, and can't have another opened.
    #[serde(skip)]
    link: Option<String>,
    /// What recording will do with the settings as they are:
    /// "Transcribed on this computer", "Speakers told apart"…
    summary: Vec<String>,
    /// For the offer to stop: how long has been recorded ("34 min").
    recorded: Option<String>,
    /// Seconds the panel stays up.
    seconds: u64,
}

/// The panel for `offer`, with the settings as they are now.
fn panel(app: &AppHandle, offer: Offer) -> Panel {
    let mut panel = Panel {
        offer,
        event: None,
        starts: None,
        joined: false,
        link: None,
        summary: Vec::new(),
        recorded: None,
        seconds: SHOW_FOR.as_secs(),
    };
    match offer {
        Offer::Record { .. } | Offer::Join { .. } => {
            let Some(state) = app.try_state::<AppState>() else {
                return panel;
            };
            let notes = state.settings().notes;
            // (The offer to join comes with its own event.)
            if notes.meeting_calendar && matches!(offer, Offer::Record { .. }) {
                panel.event = crate::meeting::calendar_event();
            }
            // Where the speech goes, as the meeting would send it.
            if let Ok(speech) = state.session.file_speech() {
                let (place, local) = crate::notes::files::place(&speech.stt.name());
                panel.summary.push(if local {
                    "Transcribed on this computer".to_string()
                } else {
                    format!("Transcribed {place}")
                });
            }
            if notes.tells_speakers_apart() {
                panel.summary.push("speakers told apart".into());
            }
            if notes.remember_speakers && cfg!(target_os = "macos") {
                panel.summary.push("people you've named recognized".into());
            }
            if notes.keep_meeting_audio {
                panel.summary.push("recording kept".into());
            }
        }
        Offer::Stop { .. } => {
            panel.recorded = crate::meeting::status()
                .map(|s| crate::notes::files::length(Duration::from_secs_f64(s.elapsed.max(0.0))));
        }
    }
    panel
}

/// The offer on screen, if any.
fn shown() -> MutexGuard<'static, Option<(Panel, Instant)>> {
    static SHOWN: OnceLock<Mutex<Option<(Panel, Instant)>>> = OnceLock::new();
    SHOWN.get_or_init(Mutex::default).locked()
}

/// When a call app on the microphone was last offered to be recorded: for
/// the calendar, which leaves a meeting that was just offered alone.
static CALL_OFFERED: Mutex<Option<Instant>> = Mutex::new(None);

/// Call apps on the microphone, and since when.
#[derive(Default)]
struct Watcher {
    using: HashMap<&'static str, Instant>,
    /// Already offered while they kept the microphone.
    offered: Vec<&'static str>,
    /// The call app being recorded, and when it stopped using the
    /// microphone.
    recording: Option<(&'static str, Option<Instant>)>,
    /// The offer to stop was made for this meeting.
    offered_stop: bool,
}

impl Watcher {
    /// Takes the call apps using the microphone now; returns an offer to
    /// make, if any.
    /// `skip`: apps the user said not to offer to record for (still
    /// followed, to offer to stop a meeting they started themselves).
    /// `held`: the calendar's offer has the meeting that's starting, so
    /// the call apps on the microphone count as offered already.
    fn poll(
        &mut self,
        now: Instant,
        using: &[&'static str],
        meeting: bool,
        skip: &[String],
        held: bool,
    ) -> Option<Offer> {
        self.using.retain(|app, _| using.contains(app));
        self.offered.retain(|app| using.contains(app));
        for app in using {
            self.using.entry(app).or_insert(now);
        }
        if !meeting {
            self.recording = None;
            self.offered_stop = false;
            if held {
                // One offer for one meeting: no second one for as long as
                // they keep the microphone.
                for app in using {
                    if !self.offered.contains(app) {
                        self.offered.push(*app);
                    }
                }
                return None;
            }
            let (app, _) = self
                .using
                .iter()
                .filter(|(app, since)| {
                    now.duration_since(**since) >= SETTLE
                        && !self.offered.contains(app)
                        && !skip.iter().any(|s| s == *app)
                })
                .min_by_key(|(_, since)| **since)?;
            let app = *app;
            self.offered.push(app);
            return Some(Offer::Record { app });
        }
        // A meeting is recording: the call app it's for is the one on the
        // microphone longest.
        if self.recording.is_none() {
            let first = self.using.iter().min_by_key(|(_, since)| **since);
            self.recording = first.map(|(app, _)| (*app, None));
            // Offered already (or it started some other way): no offer
            // to record it again later.
            self.offered.extend(self.using.keys().copied());
        }
        let (app, left) = self.recording.as_mut()?;
        if self.using.contains_key(app) {
            *left = None;
            return None;
        }
        let left = *left.get_or_insert(now);
        if now.duration_since(left) >= ENDED_AFTER && !self.offered_stop {
            self.offered_stop = true;
            return Some(Offer::Stop { app });
        }
        None
    }
}

/// Watches the microphone for calls, on its own thread, for as long as the
/// app runs.
pub fn watch(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("calls".into())
        .spawn(move || {
            let mut watcher = Watcher::default();
            loop {
                std::thread::sleep(POLL);
                expire(&app);
                // Recording started some other way while the offer to
                // join was up: there's nothing left to offer.
                if crate::meeting::is_running() && calendar::offered_now() {
                    hide(&app);
                }
                // (Settings are in place a moment after launch.)
                let Some(notes) = app.try_state::<AppState>().map(|s| s.settings().notes) else {
                    continue;
                };
                if !notes.offer_on_calls() {
                    watcher = Watcher::default();
                    continue;
                }
                let meeting = match crate::meeting::recording_call() {
                    Some(true) => true,
                    None => false,
                    // An app's sound or the microphone alone is recording:
                    // a call can't be recorded too, and its end isn't this
                    // recording's end.
                    Some(false) => {
                        watcher = Watcher::default();
                        continue;
                    }
                };
                let mut using: Vec<&'static str> = splashdown_platform::microphone_users()
                    .iter()
                    .filter_map(|id| call_app(id))
                    .collect();
                using.sort_unstable();
                using.dedup();
                let held = calendar::has_the_call();
                if let Some(offer) =
                    watcher.poll(Instant::now(), &using, meeting, &notes.offer_skip, held)
                {
                    tracing::info!("call offer: {offer:?}");
                    if matches!(offer, Offer::Record { .. }) {
                        *CALL_OFFERED.locked() = Some(Instant::now());
                    }
                    show(&app, panel(&app, offer));
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("couldn't watch for calls: {e}");
    }
}

/// Shows `panel` in the top right corner, without taking focus from the
/// call.
fn show(app: &AppHandle, panel: Panel) {
    *shown() = Some((panel.clone(), Instant::now()));
    let window = match app.get_webview_window(WINDOW) {
        Some(window) => window,
        None => {
            let built =
                WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("offer.html".into()))
                    .title("Splashdown")
                    .inner_size(WIDTH, record_height(false))
                    .resizable(false)
                    .decorations(false)
                    .always_on_top(true)
                    .visible_on_all_workspaces(true)
                    .skip_taskbar(true)
                    // Never takes the keyboard from the call.
                    .focusable(false)
                    .focused(false)
                    .visible(false)
                    .build();
            match built {
                Ok(window) => {
                    let handle = window.clone();
                    window.on_window_event(move |event| {
                        if let WindowEvent::CloseRequested { api, .. } = event {
                            api.prevent_close();
                            let _ = handle.hide();
                        }
                    });
                    window
                }
                Err(e) => {
                    tracing::warn!("couldn't show the call offer: {e}");
                    return;
                }
            }
        }
    };
    let height = match panel.offer {
        Offer::Record { .. } | Offer::Join { .. } => record_height(panel.event.is_some()),
        Offer::Stop { .. } => STOP_HEIGHT,
    };
    let _ = window.set_size(tauri::LogicalSize::new(WIDTH, height));
    crate::placement::place(
        &window,
        crate::placement::Spot::TopRight {
            top: TOP,
            right: 16.0,
        },
    );
    let _ = window.show();
    let _ = app.emit_to(WINDOW, CHANGED, panel);
}

/// Where the panel goes, from the top of the screen's work area.
const TOP: f64 = 40.0;
/// The panel's size: wider and taller than a plain question, for what to
/// know before recording. The stop offer is shorter: a line and two
/// buttons.
const WIDTH: f64 = 360.0;
const STOP_HEIGHT: f64 = 132.0;

/// The record offer's height: 180, and 200 with the calendar event's line.
fn record_height(event: bool) -> f64 {
    if event { 200.0 } else { 180.0 }
}

/// Below the panel at its tallest: where another panel can go without
/// covering it (the app's questions).
pub const BELOW: f64 = TOP + 200.0 + 12.0;

/// Hides the panel once it has been up as long as it said it would.
fn expire(app: &AppHandle) {
    let mut shown = shown();
    if shown
        .as_ref()
        .is_some_and(|(panel, at)| at.elapsed() >= Duration::from_secs(panel.seconds))
    {
        *shown = None;
        if let Some(window) = app.get_webview_window(WINDOW) {
            let _ = window.hide();
        }
    }
}

fn hide(app: &AppHandle) {
    *shown() = None;
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.hide();
    }
}

// ---------------------------------------------------------------- commands

/// The offer on screen (for a page that just loaded).
#[tauri::command]
pub fn call_offer_state() -> Option<Panel> {
    shown().as_ref().map(|(panel, _)| panel.clone())
}

/// The panel's buttons: "record", "stop", "never" (don't offer for this
/// app again), "settings" (Change…, which leaves the panel up), "join" (a
/// meeting on the calendar, which leaves it up too), or anything else to
/// dismiss it.
#[tauri::command]
pub async fn call_offer_answer(answer: String, app: AppHandle) -> Result<(), String> {
    if answer == "settings" {
        crate::tray::open_settings_tab(&app, "meetings");
        return Ok(());
    }
    let panel = shown().as_ref().map(|(panel, _)| panel.clone());
    let offered = panel.as_ref().map(|panel| panel.offer);
    if let Some(panel) = panel
        && matches!(offered, Some(Offer::Join { .. }))
    {
        return calendar::answer(&app, panel, &answer);
    }
    hide(&app);
    match answer.as_str() {
        "record" => crate::meeting::start(&app).map(|_| ()),
        "stop" => crate::meeting::stop(),
        "never" => {
            let Some(Offer::Record { app: call }) = offered else {
                return Ok(());
            };
            let state = app.state::<AppState>();
            state
                .update(&app, |s| {
                    if !s.notes.offer_skip.iter().any(|a| a == call) {
                        s.notes.offer_skip.push(call.to_string());
                    }
                })
                .map(|_| ())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_apps_are_recognized() {
        assert_eq!(call_app("us.zoom.xos"), Some("Zoom"));
        assert_eq!(call_app("Zoom.exe"), Some("Zoom"));
        assert_eq!(call_app("MSTeams_8wekyb3d8bbwe"), Some("Teams"));
        assert_eq!(call_app("com.google.Chrome.helper"), Some("Chrome"));
        assert_eq!(call_app("com.apple.CoreSpeech"), None);
        assert_eq!(call_app("app.splashdown.Splashdown"), None);
    }

    #[test]
    fn a_call_is_offered_once_and_its_end_noticed() {
        let mut w = Watcher::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // A quick microphone check isn't a call.
        assert_eq!(w.poll(at(0), &["Zoom"], false, &[], false), None);
        assert_eq!(w.poll(at(2), &[], false, &[], false), None);
        // A call: offered once it has settled, and only once.
        assert_eq!(w.poll(at(4), &["Zoom"], false, &[], false), None);
        assert_eq!(
            w.poll(at(8), &["Zoom"], false, &[], false),
            Some(Offer::Record { app: "Zoom" })
        );
        assert_eq!(w.poll(at(10), &["Zoom"], false, &[], false), None);
        // Recording; Zoom lets go of the microphone for a moment (no
        // offer), then for good.
        assert_eq!(w.poll(at(12), &["Zoom"], true, &[], false), None);
        assert_eq!(w.poll(at(14), &[], true, &[], false), None);
        assert_eq!(w.poll(at(16), &["Zoom"], true, &[], false), None);
        assert_eq!(w.poll(at(18), &[], true, &[], false), None);
        assert_eq!(
            w.poll(at(40), &[], true, &[], false),
            Some(Offer::Stop { app: "Zoom" })
        );
        assert_eq!(w.poll(at(60), &[], true, &[], false), None, "offered once");
        // The meeting stopped; the next call is offered again.
        assert_eq!(w.poll(at(62), &[], false, &[], false), None);
        assert_eq!(w.poll(at(64), &["Teams"], false, &[], false), None);
        assert_eq!(
            w.poll(at(68), &["Teams"], false, &[], false),
            Some(Offer::Record { app: "Teams" })
        );
    }

    #[test]
    fn a_meeting_started_by_hand_isnt_offered_again() {
        let mut w = Watcher::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert_eq!(w.poll(at(0), &["Zoom"], true, &[], false), None);
        assert_eq!(w.poll(at(10), &["Zoom"], true, &[], false), None);
        // Stopped by hand while the call goes on: no offer to record it.
        assert_eq!(w.poll(at(12), &["Zoom"], false, &[], false), None);
    }

    #[test]
    fn a_skipped_app_is_never_offered_but_its_end_is_noticed() {
        let mut w = Watcher::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let skip = ["Zoom".to_string()];
        assert_eq!(w.poll(at(0), &["Zoom"], false, &skip, false), None);
        assert_eq!(w.poll(at(8), &["Zoom"], false, &skip, false), None);
        // Recorded by hand anyway: when Zoom lets go, stopping is offered.
        assert_eq!(w.poll(at(10), &["Zoom"], true, &skip, false), None);
        assert_eq!(w.poll(at(12), &[], true, &skip, false), None);
        assert_eq!(
            w.poll(at(40), &[], true, &skip, false),
            Some(Offer::Stop { app: "Zoom" })
        );
    }

    #[test]
    fn a_meeting_the_calendar_offered_gets_no_second_offer() {
        let mut w = Watcher::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // The calendar's offer is up, Join was clicked, and Zoom takes
        // the microphone: that's the same meeting.
        assert_eq!(w.poll(at(0), &[], false, &[], true), None);
        assert_eq!(w.poll(at(2), &["Zoom"], false, &[], true), None);
        assert_eq!(w.poll(at(10), &["Zoom"], false, &[], true), None);
        // Not offered later in that call either, once the calendar's
        // offer is long gone.
        assert_eq!(w.poll(at(700), &["Zoom"], false, &[], false), None);
        // Recorded by hand after all: its end is noticed as ever.
        assert_eq!(w.poll(at(710), &["Zoom"], true, &[], false), None);
        assert_eq!(w.poll(at(712), &[], true, &[], false), None);
        assert_eq!(
            w.poll(at(740), &[], true, &[], false),
            Some(Offer::Stop { app: "Zoom" })
        );
        // The next call is offered again.
        assert_eq!(w.poll(at(800), &[], false, &[], false), None);
        assert_eq!(w.poll(at(802), &["Zoom"], false, &[], false), None);
        assert_eq!(
            w.poll(at(806), &["Zoom"], false, &[], false),
            Some(Offer::Record { app: "Zoom" })
        );
        // A call that was on before the calendar's offer, and offered
        // then, isn't offered again after it.
        assert_eq!(w.poll(at(808), &["Zoom"], false, &[], true), None);
        assert_eq!(w.poll(at(900), &["Zoom"], false, &[], false), None);
    }

    #[test]
    fn the_page_gets_the_offer_to_join_without_its_link() {
        let panel = Panel {
            offer: Offer::Join { service: "Zoom" },
            event: Some(crate::meeting::Event {
                title: "Pricing review".into(),
                people: vec!["Dana Lee".into()],
            }),
            starts: Some(1_794_322_800_000),
            joined: false,
            link: Some("https://zoom.us/j/5551234567".into()),
            summary: Vec::new(),
            recorded: None,
            seconds: 180,
        };
        let json = serde_json::to_value(&panel).unwrap();
        assert_eq!(json["offer"]["join"]["service"], "Zoom");
        assert_eq!(json["event"]["title"], "Pricing review");
        assert_eq!(json["starts"], 1_794_322_800_000_i64);
        assert_eq!(json["joined"], false);
        assert!(json.get("link").is_none(), "{json}");
        assert!(!json.to_string().contains("zoom.us"), "{json}");
    }
}
