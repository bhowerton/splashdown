//! Media keys (play/pause, next, previous) for voice actions, and pausing
//! the apps that are playing while recording (and playing them again
//! after), through Windows' media sessions. The Windows twin of
//! `mac/media.rs`.

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as MediaSession,
    GlobalSystemMediaTransportControlsSessionManager as MediaSessions,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE,
    VK_MEDIA_PREV_TRACK,
};

use super::INJECTED_TAG;
use super::clipboard::{keyboard, send};
use crate::Result;

/// A media key (play/pause, next, previous), for voice actions: whichever
/// app is playing answers it.
pub(crate) fn press_media_key(key: crate::MediaKey) -> Result<()> {
    let vk = match key {
        crate::MediaKey::PlayPause => VK_MEDIA_PLAY_PAUSE,
        crate::MediaKey::Next => VK_MEDIA_NEXT_TRACK,
        crate::MediaKey::Previous => VK_MEDIA_PREV_TRACK,
    };
    let input = |up: bool| {
        keyboard(KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: if up {
                KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
            } else {
                KEYEVENTF_EXTENDEDKEY
            },
            time: 0,
            dwExtraInfo: INJECTED_TAG,
        })
    };
    send(&[input(false), input(true)])
}

/// The apps showing media controls (players, browser tabs with a video).
fn media_sessions() -> Vec<MediaSession> {
    MediaSessions::RequestAsync()
        .and_then(|request| request.join())
        .and_then(|sessions| sessions.GetSessions())
        .map(|list| list.into_iter().collect())
        .unwrap_or_default()
}

fn status(session: &MediaSession) -> Option<PlaybackStatus> {
    session.GetPlaybackInfo().ok()?.PlaybackStatus().ok()
}

/// Pauses the apps that are playing, and returns them (by app ID).
pub(crate) fn pause_players() -> Vec<String> {
    let mut paused = Vec::new();
    for session in media_sessions() {
        if status(&session) != Some(PlaybackStatus::Playing) {
            continue;
        }
        let Ok(id) = session.SourceAppUserModelId() else {
            continue;
        };
        if session.TryPauseAsync().and_then(|op| op.join()) == Ok(true) {
            paused.push(id.to_string());
        }
    }
    paused
}

/// Plays again the apps `pause_players` paused, unless they were started,
/// stopped or closed meanwhile.
pub(crate) fn resume_players(players: &[String]) {
    for session in media_sessions() {
        let ours = session
            .SourceAppUserModelId()
            .is_ok_and(|id| players.contains(&id.to_string()));
        if ours && status(&session) == Some(PlaybackStatus::Paused) {
            let _ = session.TryPlayAsync().and_then(|op| op.join());
        }
    }
}
