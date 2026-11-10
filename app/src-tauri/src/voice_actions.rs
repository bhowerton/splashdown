//! Voice actions: carrying out what a spoken command was planned into
//! (`splashdown_core::action`): opening apps and web pages, media keys and
//! the volume, a to-do in today's note, calendar events and reminders, an
//! email draft, a Shortcut, the user's own actions (`my_actions`), and
//! Splashdown's own windows. Runs off the
//! session thread (an action may wait for the user's OK); the session
//! does what touches its own state (presets, retry, hands-free).
//!
//! Nothing here sends, deletes or pays: an email opens as a draft, and a
//! Shortcut or (if the user asked for it) an event is confirmed first.

use splashdown_core::sync::Locked;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use splashdown_config::{ActionSettings, SearchEngine};
use splashdown_core::action::{Action, best_name};
use splashdown_platform::{InstalledApp, MediaKey};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// What was done, for the pill, and how to take it back.
pub struct Done {
    pub text: String,
    pub undo: Option<Undo>,
}

/// Something an action added, which "undo that" takes back.
pub enum Undo {
    Note(crate::notes::Captured),
    #[cfg(target_os = "macos")]
    Calendar(splashdown_apple_shim::Added),
}

impl Undo {
    /// Takes it back; what the pill says.
    pub fn undo(self) -> Result<String, String> {
        match self {
            Undo::Note(captured) => match crate::notes::undo_capture(&captured)? {
                true => Ok("Removed the to-do".into()),
                false => Err("The note has changed since; nothing removed".into()),
            },
            #[cfg(target_os = "macos")]
            Undo::Calendar(added) => {
                tauri::async_runtime::block_on(splashdown_apple_shim::remove(&added))?;
                Ok(if added.reminder {
                    "Removed the reminder".into()
                } else {
                    "Removed the event".into()
                })
            }
        }
    }
}

fn done(text: impl Into<String>) -> Result<Done, String> {
    Ok(Done {
        text: text.into(),
        undo: None,
    })
}

/// Carries out `action`. `language` is the dictation language, if set
/// (for Wikipedia).
pub fn carry_out(
    app: &AppHandle,
    action: Action,
    settings: &ActionSettings,
    language: Option<&str>,
) -> Result<Done, String> {
    match action {
        Action::SwitchToApp { app: name } => switch_to_app(&name),
        Action::OpenApp { app: name } => open_app(&name),
        Action::SearchWeb { query, site } => {
            let url = search_url(settings.search_engine, &site, &query, language)?;
            open(&url)?;
            done(format!("Searching for “{query}”"))
        }
        Action::OpenWebsite { site } => match website(&site) {
            Some(url) => {
                open(&url)?;
                done(format!("Opened {site}"))
            }
            // Not a site we know by name: a search finds it, and nothing
            // opens that the user didn't see named.
            None => {
                open(&search_url(settings.search_engine, "web", &site, language)?)?;
                done(format!("Searching for {site}"))
            }
        },
        Action::Splashdown { what, .. } => own(app, &what),
        Action::AddTodo { text } => add_todo(app, &text),
        Action::Media { what, level } => media(&what, level),
        Action::CreateEvent {
            title,
            start,
            end,
            location,
        } => create_event(
            app,
            settings,
            &title,
            &start,
            end.as_deref(),
            location.as_deref(),
        ),
        Action::CreateReminder { text, due } => {
            create_reminder(app, settings, &text, due.as_deref())
        }
        Action::DraftEmail { to, subject, body } => {
            open(&mailto(to.as_deref(), subject.as_deref(), &body)?)?;
            done("Email draft opened, ready for you to check and send")
        }
        Action::RunShortcut { name, input } => run_shortcut(app, settings, &name, input.as_deref()),
        Action::MyAction { name, text } => {
            crate::my_actions::run(app, settings, &name, text.as_deref())
        }
    }
}

fn open(url: &str) -> Result<(), String> {
    splashdown_platform::open_url(url).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- apps

/// Installed apps, kept for a while: listing them takes a moment.
static APPS: Mutex<Option<(Instant, Vec<InstalledApp>)>> = Mutex::new(None);
const APPS_FRESH: Duration = Duration::from_secs(10 * 60);

fn apps(fresh: bool) -> Vec<InstalledApp> {
    let mut slot = APPS.locked();
    if fresh
        || slot
            .as_ref()
            .is_none_or(|(at, _)| at.elapsed() > APPS_FRESH)
    {
        *slot = Some((Instant::now(), splashdown_platform::installed_apps()));
    }
    slot.as_ref().map(|(_, a)| a.clone()).unwrap_or_default()
}

/// Lists the installed apps in the background, so the first "open …"
/// doesn't wait for it.
pub fn warm_up() {
    std::thread::spawn(|| {
        apps(false);
        #[cfg(target_os = "macos")]
        shortcuts();
    });
}

/// Brings a running app forward, or opens it if it isn't running. On
/// Windows, opening a shortcut again starts another copy of apps that
/// allow several (Notepad, File Explorer), so a running one is looked for
/// first.
fn switch_to_app(name: &str) -> Result<Done, String> {
    let running = splashdown_platform::running_apps();
    let names: Vec<String> = running.iter().map(|(n, _)| n.clone()).collect();
    if let Some(i) = best_name(name, &names)
        && splashdown_platform::activate_app(running[i].1).is_ok()
    {
        return done(format!("Switched to {}", running[i].0));
    }
    open_app(name)
}

fn open_app(name: &str) -> Result<Done, String> {
    let find = |apps: &[InstalledApp]| {
        let names: Vec<String> = apps.iter().map(|a| a.name.clone()).collect();
        best_name(name, &names).map(|i| apps[i].clone())
    };
    // Installed since the list was made: look again once.
    let app = find(&apps(false)).or_else(|| find(&apps(true)));
    let Some(app) = app else {
        return Err(format!("No app called “{name}” on this computer"));
    };
    splashdown_platform::open_app(&app).map_err(|e| e.to_string())?;
    done(format!("Opened {}", app.name))
}

// ---------------------------------------------------------------- the web

fn url_with(base: &str, params: &[(&str, &str)]) -> Result<String, String> {
    reqwest::Url::parse_with_params(base, params)
        .map(String::from)
        .map_err(|e| e.to_string())
}

/// A search for `query` on `site` (one of `splashdown_core::action::SITES`).
fn search_url(
    engine: SearchEngine,
    site: &str,
    query: &str,
    language: Option<&str>,
) -> Result<String, String> {
    let q = [("q", query)];
    match (site, engine) {
        ("youtube", _) => url_with(
            "https://www.youtube.com/results",
            &[("search_query", query)],
        ),
        ("wikipedia", _) => url_with(
            "https://www.wikipedia.org/search-redirect.php",
            &[
                ("family", "wikipedia"),
                ("language", language.unwrap_or("en")),
                ("search", query),
            ],
        ),
        ("amazon", _) => url_with("https://www.amazon.com/s", &[("k", query)]),
        ("maps", _) if cfg!(target_os = "macos") => url_with("https://maps.apple.com/", &q),
        ("maps", SearchEngine::Bing) => url_with("https://www.bing.com/maps", &q),
        ("maps", _) => url_with(
            "https://www.google.com/maps/search/",
            &[("api", "1"), ("query", query)],
        ),
        ("images", SearchEngine::Google) => url_with(
            "https://www.google.com/search",
            &[("q", query), ("tbm", "isch")],
        ),
        ("images", SearchEngine::Duckduckgo) => url_with(
            "https://duckduckgo.com/",
            &[("q", query), ("ia", "images"), ("iax", "images")],
        ),
        ("images", SearchEngine::Bing) => url_with("https://www.bing.com/images/search", &q),
        ("news", SearchEngine::Google) => url_with(
            "https://www.google.com/search",
            &[("q", query), ("tbm", "nws")],
        ),
        ("news", SearchEngine::Duckduckgo) => url_with(
            "https://duckduckgo.com/",
            &[("q", query), ("ia", "news"), ("iar", "news")],
        ),
        ("news", SearchEngine::Bing) => url_with("https://www.bing.com/news/search", &q),
        (_, SearchEngine::Google) => url_with("https://www.google.com/search", &q),
        (_, SearchEngine::Duckduckgo) => url_with("https://duckduckgo.com/", &q),
        (_, SearchEngine::Bing) => url_with("https://www.bing.com/search", &q),
    }
}

/// Sites people name rather than spell out.
const SITES: &[(&str, &str)] = &[
    ("github", "github.com"),
    ("gitlab", "gitlab.com"),
    ("google", "www.google.com"),
    ("gmail", "mail.google.com"),
    ("google docs", "docs.google.com"),
    ("google drive", "drive.google.com"),
    ("google calendar", "calendar.google.com"),
    ("google maps", "maps.google.com"),
    ("youtube", "www.youtube.com"),
    ("wikipedia", "www.wikipedia.org"),
    ("reddit", "www.reddit.com"),
    ("hacker news", "news.ycombinator.com"),
    ("x", "x.com"),
    ("twitter", "x.com"),
    ("linkedin", "www.linkedin.com"),
    ("facebook", "www.facebook.com"),
    ("instagram", "www.instagram.com"),
    ("amazon", "www.amazon.com"),
    ("netflix", "www.netflix.com"),
    ("spotify", "open.spotify.com"),
    ("chatgpt", "chatgpt.com"),
    ("claude", "claude.ai"),
    ("notion", "www.notion.so"),
    ("figma", "www.figma.com"),
    ("slack", "app.slack.com"),
    ("bbc", "www.bbc.com"),
    ("bbc news", "www.bbc.com/news"),
    ("the guardian", "www.theguardian.com"),
    ("guardian", "www.theguardian.com"),
    ("new york times", "www.nytimes.com"),
    ("el pais", "elpais.com"),
    ("le monde", "www.lemonde.fr"),
    ("outlook", "outlook.live.com"),
    ("icloud", "www.icloud.com"),
];

/// The address of a site named `site`: one spelled out ("spiegel.de"), or
/// one from `SITES`. `None` for anything else.
fn website(site: &str) -> Option<String> {
    let name = splashdown_core::action::fold(site.trim());
    let name = name
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let domain_like = name.contains('.')
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '/'));
    if domain_like {
        return Some(format!("https://{name}"));
    }
    let bare = name.trim_end_matches(" website").trim_start_matches("the ");
    SITES
        .iter()
        .find(|(n, _)| *n == name || *n == bare)
        .map(|(_, domain)| format!("https://{domain}"))
}

/// The link that opens a new email in the mail app, for the user to check
/// and send. Also what a hook's email draft opens (`hooks/ask.rs`).
pub(crate) fn mailto(
    to: Option<&str>,
    subject: Option<&str>,
    body: &str,
) -> Result<String, String> {
    // An address only: a name ("Dana") is left for the user to fill in.
    let to = to
        .map(str::trim)
        .filter(|t| t.contains('@') && !t.contains(char::is_whitespace))
        .unwrap_or("");
    let mut params = vec![("body", body)];
    if let Some(subject) = subject {
        params.insert(0, ("subject", subject));
    }
    // `mailto:` has no host; the query is encoded as for any URL.
    let url = url_with("https://x/", &params)?;
    let query = url.split_once('?').map_or("", |(_, q)| q);
    // Spaces as %20: mail apps show a `+` literally.
    Ok(format!("mailto:{to}?{}", query.replace('+', "%20")))
}

// ---------------------------------------------------------------- Splashdown

fn own(app: &AppHandle, what: &str) -> Result<Done, String> {
    let on_main = |f: Box<dyn FnOnce(&AppHandle) + Send>| {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || f(&handle));
    };
    match what {
        "start_meeting" => {
            crate::meeting::start(app)?;
            done("Recording the meeting")
        }
        "stop_meeting" => {
            crate::meeting::stop()?;
            done("Stopping the meeting")
        }
        "new_note" => {
            on_main(Box::new(|a| crate::notes::open_to(a, Some("new"))));
            done("New note")
        }
        "open_notes" => {
            on_main(Box::new(crate::notes::open));
            done("Opened Notes")
        }
        "todays_note" => {
            on_main(Box::new(|a| crate::notes::open_to(a, Some("daily"))));
            done("Opened today's note")
        }
        "open_settings" => {
            on_main(Box::new(crate::tray::open_settings));
            done("Opened Settings")
        }
        "whisper_mode_on" | "whisper_mode_off" => {
            let on = what == "whisper_mode_on";
            let state = app.state::<AppState>();
            if state.settings().general.whisper_mode() != on {
                tauri::async_runtime::block_on(state.toggle_whisper_mode(app))?;
            }
            done(if on {
                "Whisper mode on"
            } else {
                "Whisper mode off"
            })
        }
        // The session does the rest itself (`Controller::own_action`).
        other => Err(format!("Splashdown can't do {other:?} from here")),
    }
}

fn add_todo(app: &AppHandle, text: &str) -> Result<Done, String> {
    let settings = app.state::<AppState>().settings().notes;
    let captured = crate::notes::add_todo(&settings, text)?;
    Ok(Done {
        text: format!("Added to {}", captured.label),
        undo: Some(Undo::Note(captured)),
    })
}

// ---------------------------------------------------------------- sound

fn media(what: &str, level: Option<u8>) -> Result<Done, String> {
    let key = |k| splashdown_platform::press_media_key(k).map_err(|e| e.to_string());
    let volume = |v: f32| splashdown_platform::set_output_volume(v).map_err(|e| e.to_string());
    let current = || splashdown_platform::output_volume().unwrap_or(0.5);
    match what {
        "play" | "pause" => key(MediaKey::PlayPause)?,
        "next_track" => key(MediaKey::Next)?,
        "previous_track" => key(MediaKey::Previous)?,
        "mute" | "unmute" => {
            splashdown_platform::set_output_muted(what == "mute").map_err(|e| e.to_string())?;
        }
        "volume_up" => volume(current() + 0.1)?,
        "volume_down" => volume(current() - 0.1)?,
        "set_volume" => volume(f32::from(level.unwrap_or(50)) / 100.0)?,
        other => return Err(format!("no media control {other:?}")),
    }
    done(match what {
        "play" => "Play".to_string(),
        "pause" => "Pause".to_string(),
        "next_track" => "Next".to_string(),
        "previous_track" => "Previous".to_string(),
        "mute" => "Sound off".to_string(),
        "unmute" => "Sound on".to_string(),
        _ => format!(
            "Volume {:.0}%",
            splashdown_platform::output_volume().unwrap_or(0.0) * 100.0
        ),
    })
}

// ---------------------------------------------------------------- calendar

/// "Thursday 5 Nov, 16:00".
fn when(time: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(time, "%Y-%m-%dT%H:%M").map_or_else(
        |_| time.to_string(),
        |t| t.format("%A %-d %b, %H:%M").to_string(),
    )
}

/// Asks before adding, when the user wants that. Whether to go ahead.
#[cfg(target_os = "macos")]
fn confirmed(app: &AppHandle, settings: &ActionSettings, what: &str) -> bool {
    !settings.confirm_events
        || crate::prompt::ask(app, "Add this?", what, &["Add it", "Cancel"]) == Some(0)
}

fn create_event(
    app: &AppHandle,
    settings: &ActionSettings,
    title: &str,
    start: &str,
    end: Option<&str>,
    location: Option<&str>,
) -> Result<Done, String> {
    let described = format!("“{title}”, {}", when(start));
    #[cfg(target_os = "macos")]
    {
        if !confirmed(app, settings, &format!("Add {described} to your calendar.")) {
            return done("Not added");
        }
        let added = tauri::async_runtime::block_on(splashdown_apple_shim::add_event(
            title, start, end, location,
        ))
        .map_err(|e| calendar_error(e, "calendar", "Calendars"))?;
        Ok(Done {
            text: format!("Added {described} · say “undo that” to remove it"),
            undo: Some(Undo::Calendar(added)),
        })
    }
    // The calendar app shows the event to save or not: no question first.
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, settings);
        let dir = std::env::temp_dir().join("Splashdown events");
        let _ = std::fs::create_dir_all(&dir);
        // Earlier ones have been opened (and saved or not) by now.
        if let Ok(old) = std::fs::read_dir(&dir) {
            for entry in old.flatten() {
                let stale = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > Duration::from_secs(3600)));
                if stale {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let path = dir.join(format!(
            "Event {}.ics",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        std::fs::write(&path, ics(title, start, end, location)?)
            .map_err(|e| format!("Couldn't make the event: {e}"))?;
        open(&path.to_string_lossy())?;
        done(format!("{described} is open in your calendar app to save"))
    }
}

fn create_reminder(
    app: &AppHandle,
    settings: &ActionSettings,
    text: &str,
    due: Option<&str>,
) -> Result<Done, String> {
    #[cfg(target_os = "macos")]
    {
        let described = match due {
            Some(due) => format!("“{text}”, {}", when(due)),
            None => format!("“{text}”"),
        };
        if !confirmed(app, settings, &format!("Remind you of {described}.")) {
            return done("Not added");
        }
        let added =
            tauri::async_runtime::block_on(splashdown_apple_shim::add_reminder(text, due, None))
                .map_err(|e| calendar_error(e, "reminders", "Reminders"))?;
        Ok(Done {
            text: format!("Reminder: {described} · say “undo that” to remove it"),
            undo: Some(Undo::Calendar(added)),
        })
    }
    // No reminders app to add to: a to-do with its time in today's note.
    #[cfg(not(target_os = "macos"))]
    {
        let _ = settings;
        let todo = match due {
            Some(due) => format!("{text} ({})", when(due)),
            None => text.to_string(),
        };
        add_todo(app, &todo)
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn calendar_error(e: splashdown_apple_shim::AddError, what: &str, pane: &str) -> String {
    match e {
        splashdown_apple_shim::AddError::NoAccess => format!(
            "Splashdown can't add to your {what}: allow it in System Settings → Privacy & Security → {pane}"
        ),
        splashdown_apple_shim::AddError::Failed(e) => format!("Couldn't add it: {e}"),
    }
}

/// An iCalendar file for one event, in local ("floating") time.
#[cfg(any(not(target_os = "macos"), test))]
fn ics(
    title: &str,
    start: &str,
    end: Option<&str>,
    location: Option<&str>,
) -> Result<String, String> {
    let parse = |t: &str| {
        chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M")
            .map_err(|_| format!("not a time: {t}"))
    };
    let start = parse(start)?;
    let end = match end {
        Some(end) => parse(end)?,
        None => start + chrono::Duration::hours(1),
    };
    let text = |t: &str| {
        t.replace('\\', "\\\\")
            .replace(';', "\\;")
            .replace(',', "\\,")
            .replace('\n', "\\n")
    };
    let stamp = |t: chrono::NaiveDateTime| t.format("%Y%m%dT%H%M%S").to_string();
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//Splashdown//Voice actions//EN".into(),
        "BEGIN:VEVENT".into(),
        format!(
            "UID:{}@splashdown",
            stamp(start) + &chrono::Local::now().timestamp().to_string()
        ),
        format!("DTSTAMP:{}Z", chrono::Utc::now().format("%Y%m%dT%H%M%S")),
        format!("DTSTART:{}", stamp(start)),
        format!("DTEND:{}", stamp(end)),
        format!("SUMMARY:{}", text(title)),
    ];
    if let Some(location) = location {
        lines.push(format!("LOCATION:{}", text(location)));
    }
    lines.extend(["END:VEVENT".to_string(), "END:VCALENDAR".into()]);
    Ok(lines.join("\r\n") + "\r\n")
}

// ---------------------------------------------------------------- Shortcuts

/// The user's Shortcuts (Mac), kept for a while, for the planner and for
/// matching a spoken name.
#[cfg(target_os = "macos")]
static SHORTCUTS: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);

/// The user's Shortcuts' names (none off the Mac).
pub fn shortcuts() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let mut slot = SHORTCUTS.locked();
        if slot
            .as_ref()
            .is_none_or(|(at, _)| at.elapsed() > APPS_FRESH)
        {
            let names = std::process::Command::new("shortcuts")
                .arg("list")
                .output()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            *slot = Some((Instant::now(), names));
        }
        slot.as_ref().map(|(_, n)| n.clone()).unwrap_or_default()
    }
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

/// How long a Shortcut may run before Splashdown stops waiting for it.
#[cfg(target_os = "macos")]
const SHORTCUT_LIMIT: Duration = Duration::from_secs(60);

fn run_shortcut(
    app: &AppHandle,
    settings: &ActionSettings,
    name: &str,
    input: Option<&str>,
) -> Result<Done, String> {
    #[cfg(target_os = "macos")]
    {
        let names = shortcuts();
        let Some(name) = best_name(name, &names).map(|i| names[i].clone()) else {
            return Err(format!("No Shortcut called “{name}”"));
        };
        if !settings.trusted_shortcuts.contains(&name) {
            let answer = crate::prompt::ask(
                app,
                "Run this Shortcut?",
                &format!("“{name}” can do anything it was built to do."),
                &["Run it", "Always allow", "Cancel"],
            );
            match answer {
                Some(0) => {}
                Some(1) => trust_shortcut(app, &name)?,
                _ => return done("Not run"),
            }
        }
        let mut command = std::process::Command::new("shortcuts");
        command.arg("run").arg(&name);
        let input_file = match input {
            Some(text) => {
                let path = std::env::temp_dir().join("splashdown-shortcut-input.txt");
                std::fs::write(&path, text).map_err(|e| e.to_string())?;
                command.arg("--input-path").arg(&path);
                Some(path)
            }
            None => None,
        };
        let mut child = command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Couldn't run the Shortcut: {e}"))?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break Some(status);
            }
            if started.elapsed() > SHORTCUT_LIMIT {
                break None;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if let Some(path) = input_file {
            let _ = std::fs::remove_file(path);
        }
        match status {
            Some(s) if s.success() => done(format!("Ran “{name}”")),
            Some(_) => Err(format!("“{name}” stopped with an error")),
            // Still running (or waiting for an answer of its own): left be.
            None => done(format!("“{name}” is still running")),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, settings, name, input);
        Err("Shortcuts are a Mac app".into())
    }
}

/// "Always allow": the Shortcut runs without asking from now on.
#[cfg(target_os = "macos")]
fn trust_shortcut(app: &AppHandle, name: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.update(app, |s| {
        if !s.actions.trusted_shortcuts.iter().any(|n| n == name) {
            s.actions.trusted_shortcuts.push(name.to_string());
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_go_where_they_were_asked_to() {
        let url = |site, q| search_url(SearchEngine::Google, site, q, Some("de")).unwrap();
        assert_eq!(
            url("web", "flights to Lisbon & back"),
            "https://www.google.com/search?q=flights+to+Lisbon+%26+back"
        );
        assert_eq!(
            url("youtube", "paella"),
            "https://www.youtube.com/results?search_query=paella"
        );
        assert!(url("wikipedia", "Marie Curie").contains("language=de"));
        assert!(
            search_url(SearchEngine::Duckduckgo, "web", "x", None)
                .unwrap()
                .starts_with("https://duckduckgo.com/?q=x")
        );
    }

    #[test]
    fn websites_are_spelled_out_or_known() {
        assert_eq!(website("spiegel.de").as_deref(), Some("https://spiegel.de"));
        assert_eq!(website("GitHub").as_deref(), Some("https://github.com"));
        assert_eq!(
            website("the BBC website").as_deref(),
            Some("https://www.bbc.com")
        );
        assert_eq!(website("El País").as_deref(), Some("https://elpais.com"));
        assert_eq!(website("my bank"), None, "searched for instead");
        assert_eq!(website("javascript:alert(1)"), None);
    }

    #[test]
    fn email_drafts_only_address_an_address() {
        assert_eq!(
            mailto(Some("Dana"), Some("Late"), "Ten minutes late, sorry").unwrap(),
            "mailto:?subject=Late&body=Ten%20minutes%20late%2C%20sorry"
        );
        assert!(
            mailto(Some("anna@example.com"), None, "Hi")
                .unwrap()
                .starts_with("mailto:anna@example.com?body=Hi")
        );
    }

    #[test]
    fn an_event_file_has_local_times_and_escaped_text() {
        let ics = ics(
            "Lunch, with Priya",
            "2026-11-03T12:00",
            None,
            Some("Café; B"),
        )
        .unwrap();
        assert!(ics.contains("DTSTART:20261103T120000\r\n"));
        assert!(
            ics.contains("DTEND:20261103T130000\r\n"),
            "an hour by default"
        );
        assert!(ics.contains("SUMMARY:Lunch\\, with Priya\r\n"));
        assert!(ics.contains("LOCATION:Café\\; B\r\n"));
    }
}
