//! Ways to start dictation other than the dictation key: the control socket
//! (the `splash` CLI, `splashdown-mcp` for coding agents) and
//! `splashdown://` links (Raycast, Alfred, Shortcuts). Both end up as
//! `session::Handle::control` calls.

use std::sync::Arc;

use splashdown_control::{ClientError, Reply, Request};
use tauri::Manager;

use crate::commands::AppState;
use crate::session::Handle;

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
        open_link(&state.session, &url);
    }
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
}
