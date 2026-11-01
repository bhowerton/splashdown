//! Noticing calls: when a call app (Zoom, Teams, Meet in a browser…)
//! starts using the microphone, a small panel offers to record the meeting;
//! when the app it was recording stops using it, the panel offers to stop.
//! Nothing is recorded until the user clicks. Settings → Notes → Meetings
//! turns the offer off.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

use crate::commands::AppState;

pub const WINDOW: &str = "call-offer";
/// App event with what the panel should show (`Offer`).
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
}

/// The offer on screen, if any.
fn shown() -> MutexGuard<'static, Option<(Offer, Instant)>> {
    static SHOWN: OnceLock<Mutex<Option<(Offer, Instant)>>> = OnceLock::new();
    SHOWN
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

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
    fn poll(&mut self, now: Instant, using: &[&'static str], meeting: bool) -> Option<Offer> {
        self.using.retain(|app, _| using.contains(app));
        self.offered.retain(|app| using.contains(app));
        for app in using {
            self.using.entry(app).or_insert(now);
        }
        if !meeting {
            self.recording = None;
            self.offered_stop = false;
            let (app, _) = self
                .using
                .iter()
                .filter(|(app, since)| {
                    now.duration_since(**since) >= SETTLE && !self.offered.contains(app)
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
                // (Settings are in place a moment after launch.)
                let on = app
                    .try_state::<AppState>()
                    .is_some_and(|s| s.settings().notes.offer_on_calls());
                if !on {
                    watcher = Watcher::default();
                    continue;
                }
                let mut using: Vec<&'static str> = splashdown_platform::microphone_users()
                    .iter()
                    .filter_map(|id| call_app(id))
                    .collect();
                using.sort_unstable();
                using.dedup();
                let meeting = crate::meeting::is_running();
                if let Some(offer) = watcher.poll(Instant::now(), &using, meeting) {
                    tracing::info!("call offer: {offer:?}");
                    show(&app, offer);
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("couldn't watch for calls: {e}");
    }
}

/// Shows the panel in the top right corner, without taking focus from the
/// call.
fn show(app: &AppHandle, offer: Offer) {
    *shown() = Some((offer, Instant::now()));
    let window = match app.get_webview_window(WINDOW) {
        Some(window) => window,
        None => {
            let built =
                WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("offer.html".into()))
                    .title("Splashdown")
                    .inner_size(340.0, 112.0)
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
    if let (Ok(Some(monitor)), Ok(size)) = (window.primary_monitor(), window.outer_size()) {
        let margin = (16.0 * monitor.scale_factor()) as i32;
        let top = (40.0 * monitor.scale_factor()) as i32;
        let area = monitor.work_area();
        let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
        let _ = window.set_position(PhysicalPosition::new(x, area.position.y + top));
    }
    let _ = window.show();
    let _ = app.emit_to(WINDOW, CHANGED, offer);
}

/// Hides the panel once it has been up long enough.
fn expire(app: &AppHandle) {
    let mut shown = shown();
    if shown.is_some_and(|(_, at)| at.elapsed() >= SHOW_FOR) {
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
pub fn call_offer_state() -> Option<Offer> {
    shown().map(|(offer, _)| offer)
}

/// The panel's buttons: "record", "stop", "never" (don't offer again), or
/// anything else to dismiss it.
#[tauri::command]
pub async fn call_offer_answer(answer: String, app: AppHandle) -> Result<(), String> {
    hide(&app);
    match answer.as_str() {
        "record" => crate::meeting::start(&app).map(|_| ()),
        "stop" => crate::meeting::stop(),
        "never" => {
            let state = app.state::<AppState>();
            let mut settings = state.settings();
            settings.notes.offer_on_calls = Some(false);
            settings.save(&crate::engine::paths()?.config_file())?;
            *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
            crate::commands::settings_changed_elsewhere(&app);
            Ok(())
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
        assert_eq!(w.poll(at(0), &["Zoom"], false), None);
        assert_eq!(w.poll(at(2), &[], false), None);
        // A call: offered once it has settled, and only once.
        assert_eq!(w.poll(at(4), &["Zoom"], false), None);
        assert_eq!(
            w.poll(at(8), &["Zoom"], false),
            Some(Offer::Record { app: "Zoom" })
        );
        assert_eq!(w.poll(at(10), &["Zoom"], false), None);
        // Recording; Zoom lets go of the microphone for a moment (no
        // offer), then for good.
        assert_eq!(w.poll(at(12), &["Zoom"], true), None);
        assert_eq!(w.poll(at(14), &[], true), None);
        assert_eq!(w.poll(at(16), &["Zoom"], true), None);
        assert_eq!(w.poll(at(18), &[], true), None);
        assert_eq!(w.poll(at(40), &[], true), Some(Offer::Stop { app: "Zoom" }));
        assert_eq!(w.poll(at(60), &[], true), None, "offered once");
        // The meeting stopped; the next call is offered again.
        assert_eq!(w.poll(at(62), &[], false), None);
        assert_eq!(w.poll(at(64), &["Teams"], false), None);
        assert_eq!(
            w.poll(at(68), &["Teams"], false),
            Some(Offer::Record { app: "Teams" })
        );
    }

    #[test]
    fn a_meeting_started_by_hand_isnt_offered_again() {
        let mut w = Watcher::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert_eq!(w.poll(at(0), &["Zoom"], true), None);
        assert_eq!(w.poll(at(10), &["Zoom"], true), None);
        // Stopped by hand while the call goes on: no offer to record it.
        assert_eq!(w.poll(at(12), &["Zoom"], false), None);
    }
}
