//! Ways to start dictation other than the dictation key: the control socket
//! (`splashdown-cli`, `splashdown-mcp` for coding agents) and
//! `splashdown://` links (Raycast, Alfred, Shortcuts). Both end up as
//! `session::Handle::control` calls. The app's own Shortcuts actions are in
//! `intents.rs`.

use std::sync::Arc;

use splashdown_control::{ClientError, Reply, Request};
use tauri::Manager;

use crate::session::Handle;
use crate::state::AppState;

/// A `splashdown://` link among the words this copy was started with:
/// how Windows delivers a click on one (the link type's command is this
/// program and the link, `register_links`).
pub fn launch_link() -> Option<String> {
    std::env::args().skip(1).find(|arg| is_link(arg))
}

fn is_link(text: &str) -> bool {
    text.split_once(':')
        .is_some_and(|(scheme, rest)| scheme.eq_ignore_ascii_case("splashdown") && !rest.is_empty())
}

/// Makes this copy the one Windows starts for a `splashdown://` link.
/// Not a debug build, which would take the links from the installed app.
pub fn register_links() {
    if cfg!(debug_assertions) {
        return;
    }
    if let Err(e) = splashdown_platform::register_link_scheme("splashdown", "Splashdown") {
        tracing::warn!("splashdown:// links won't open the app: {e}");
    }
}

/// Asks a copy of the app that's already running to show Settings, or to
/// open the link this copy was started with. True if one answered: this
/// copy should quit, or both would answer the dictation key and insert
/// the text twice.
pub fn hand_over() -> bool {
    let request = match launch_link() {
        Some(url) => Request::Open { url },
        None => Request::Settings,
    };
    match splashdown_control::send(&request) {
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

/// Handles `splashdown://` links macOS hands the app
/// (`application:openURLs:`; the link type is declared in Info.plist).
/// Called on the main thread, so it never waits for the session.
pub fn on_run_event(app: &tauri::AppHandle, event: &tauri::RunEvent) {
    for url in opened_urls(event) {
        open_url(app, &url);
    }
}

/// Opens one `splashdown://` link: from macOS, from the copy of the app
/// Windows started for it (`Request::Open`), or the one this copy was
/// started with. On the main thread; never waits for the session.
pub fn open_url(app: &tauri::AppHandle, url: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    // An invite to a team's words: asks before anything changes.
    if crate::shared_words::is_invite_link(url) {
        crate::shared_words::open_invite(app, url);
        return;
    }
    if let Some(path) = note_link(url) {
        crate::notes::open_to(app, Some(&format!("note:{path}")));
        return;
    }
    open_link(&state.session, url);
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
pub fn decode(text: &str) -> String {
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

/// Only macOS hands links to a running app this way: on Windows a click
/// starts another copy with the link (`launch_link`, `hand_over`).
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
    fn a_link_is_told_from_other_arguments() {
        assert!(is_link("splashdown://toggle"));
        assert!(is_link("SPLASHDOWN:team-words?in=Dropbox&file=a.txt"));
        assert!(!is_link("splashdown:"));
        assert!(!is_link("--hidden"));
        assert!(!is_link("C:\\Users\\priya\\notes.txt"));
        assert!(!is_link("https://splashdown.app/team/"));
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
