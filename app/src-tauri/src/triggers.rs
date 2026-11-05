//! Ways to start dictation other than the dictation key: the control socket
//! (`splashdown-cli`, `splashdown-mcp` for coding agents) and
//! `splashdown://` links (Raycast, Alfred, Shortcuts), and on macOS the
//! app's own Shortcuts, Spotlight and Siri actions. All end up as
//! `session::Handle::control` calls.

use std::sync::Arc;

use splashdown_control::{ClientError, Reply, Request};
use tauri::Manager;

use crate::session::Handle;
use crate::state::AppState;

/// Asks a copy of the app that's already running to show Settings. True
/// if one answered: this copy should quit, or both would answer the
/// dictation key and insert the text twice.
pub fn hand_over() -> bool {
    match splashdown_control::send(&Request::Settings) {
        Err(ClientError::NotRunning) => false,
        // Can't tell: two copies are better than none.
        Err(ClientError::Io(e)) => {
            eprintln!("couldn't check for another copy of Splashdown: {e}");
            false
        }
        // Any answer, even an older copy's "bad request", is a copy.
        Ok(_) | Err(_) => true,
    }
}

/// Starts listening on the control socket. If it can't for another
/// reason, only the dictation key works. False if another copy of the
/// app has it (started at the same moment as this one): quit.
pub fn serve(session: Handle) -> bool {
    let handler = Arc::new(move |request: Request| Reply::from(session.control(request)));
    match splashdown_control::serve(handler) {
        Ok(()) => tracing::info!("control socket ready"),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            tracing::warn!("{e}: quitting");
            return false;
        }
        Err(e) => tracing::warn!("control socket unavailable: {e}"),
    }
    true
}

/// Lets Splashdown's actions in Shortcuts, Spotlight and Siri run (macOS:
/// App Intents, in the Swift shim). Each sends a control request as JSON,
/// answered like the socket's.
#[cfg(target_os = "macos")]
pub fn serve_intents(session: Handle) {
    splashdown_apple_shim::intents::start(move |json| {
        session
            .control(intent_request(&json)?)
            .map(Option::unwrap_or_default)
    });
}

/// An action's request (what `Intents.swift` sends).
#[cfg(target_os = "macos")]
fn intent_request(json: &str) -> Result<Request, String> {
    serde_json::from_str(json)
        .map_err(|e| format!("Splashdown didn't understand that action ({e})."))
}

/// Handles `splashdown://` links the system hands the app (macOS
/// `application:openURLs:`; the scheme is registered in Info.plist).
/// Called on the main thread, so it never waits for the session.
pub fn on_run_event(app: &tauri::AppHandle, event: &tauri::RunEvent) {
    let urls = opened_urls(event);
    if urls.is_empty() {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    for url in urls {
        if let Some(path) = note_link(&url) {
            crate::notes::open_to(app, Some(&format!("note:{path}")));
            continue;
        }
        open_link(&state.session, &url);
    }
}

/// `splashdown://open?note=<path>` (what coding agents are given to point
/// at a note) → the note's path.
fn note_link(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("splashdown") {
        return None;
    }
    let rest = rest.trim_start_matches('/');
    let (name, query) = rest.split_once('?')?;
    if !name.trim_end_matches('/').eq_ignore_ascii_case("open") {
        return None;
    }
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("note="))
        .map(decode)
        .filter(|p| !p.is_empty())
}

/// Percent-encodes `text` for a link: everything but letters, digits and
/// `-._~`.
pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Undoes [`encode`] (and `+` for a space, as forms write it). A broken
/// escape is kept as it is.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| char::from(b).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => match (
                bytes.get(i + 1).and_then(|&b| hex(b)),
                bytes.get(i + 2).and_then(|&b| hex(b)),
            ) {
                (Some(hi), Some(lo)) => {
                    out.push(hi * 16 + lo);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(target_os = "macos")]
fn opened_urls(event: &tauri::RunEvent) -> Vec<String> {
    match event {
        tauri::RunEvent::Opened { urls } => urls.iter().map(|u| u.to_string()).collect(),
        _ => Vec::new(),
    }
}

/// Windows hands links to a new process on its command line, which needs
/// the single-instance plumbing the Windows port doesn't have yet.
#[cfg(not(target_os = "macos"))]
fn opened_urls(_event: &tauri::RunEvent) -> Vec<String> {
    Vec::new()
}

fn open_link(session: &Handle, url: &str) {
    let Some(request) = link_request(url) else {
        tracing::warn!("ignoring an unknown splashdown:// link");
        return;
    };
    let session = session.clone();
    std::thread::spawn(move || {
        if let Err(e) = session.control(request) {
            tracing::info!("link: {e}");
        }
    });
}

/// `splashdown://toggle` (or `start`, `stop`, `cancel`) → its request.
/// Links can't `listen`: there'd be nobody to give the answer to.
fn link_request(url: &str) -> Option<Request> {
    let (scheme, rest) = url.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("splashdown") {
        return None;
    }
    let name = rest.trim_start_matches('/').split(['/', '?', '#']).next()?;
    Request::from_name(&name.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn intents_send_control_requests() {
        // As AddToNoteIntent sends it.
        assert_eq!(
            intent_request(r#"{"cmd":"note","text":"Buy milk"}"#),
            Ok(Request::Note {
                text: Some("Buy milk".into())
            })
        );
        assert!(intent_request(r#"{"cmd":"fly"}"#).is_err());
    }

    #[test]
    fn links_map_to_requests() {
        assert_eq!(link_request("splashdown://toggle"), Some(Request::Toggle));
        assert_eq!(link_request("splashdown://start/"), Some(Request::Start));
        assert_eq!(link_request("splashdown:stop"), Some(Request::Stop));
        assert_eq!(
            link_request("SplashDown://Cancel?from=raycast"),
            Some(Request::Cancel)
        );
        assert_eq!(link_request("splashdown://listen"), None);
        assert_eq!(link_request("splashdown://"), None);
        assert_eq!(link_request("https://toggle"), None);
    }

    #[test]
    fn note_links_carry_a_path() {
        let path = "Meetings/Weekly sync · Nov 3.md";
        let link = format!("splashdown://open?note={}", encode(path));
        assert_eq!(note_link(&link).as_deref(), Some(path));
        assert_eq!(
            note_link("splashdown://open/?x=1&note=a+b.md").as_deref(),
            Some("a b.md")
        );
        assert_eq!(
            note_link("splashdown://open?note=100%"),
            Some("100%".into())
        );
        assert_eq!(note_link("splashdown://open"), None);
        assert_eq!(note_link("splashdown://toggle?note=a.md"), None);
        assert_eq!(note_link("https://open?note=a.md"), None);
        // An open-note link isn't a request.
        assert_eq!(link_request("splashdown://open?note=a.md"), None);
    }
}
