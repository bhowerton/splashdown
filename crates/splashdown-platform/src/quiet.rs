//! Quieting other sounds while recording (turning the output down, muting
//! it, or pausing players), and putting them back, even after a crash.

use super::*;
use splashdown_core::sync::Locked;

/// How other sounds (music, videos, calls) are quieted while recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quieten {
    /// Down to a quarter of their volume.
    Lower,
    Mute,
    /// Players that are playing are paused, and played again afterwards:
    /// Spotify and Music on macOS, any app showing media controls on
    /// Windows.
    Pause,
}

/// Other sounds, quieted while recording. Dropping it puts them back:
/// the volume (unless the user changed it meanwhile), or the music.
pub struct QuietOutput(u64);

/// Only ever dropped: that puts back what it quieted.
#[allow(dead_code)]
pub(super) enum Quieted {
    Output(imp::QuietOutput),
    Paused(players::Paused),
}

/// What was quieted, kept here rather than in `QuietOutput` so
/// `restore_output` can put it back when the app quits mid-recording. The
/// number says which `QuietOutput` it belongs to.
pub(super) static QUIETED: std::sync::Mutex<Option<(u64, Quieted)>> = std::sync::Mutex::new(None);

pub(super) static QUIET_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(super) fn quieted() -> std::sync::MutexGuard<'static, Option<(u64, Quieted)>> {
    QUIETED.locked()
}

/// Lowers or mutes the default output device, or pauses music. A device
/// whose volume can't be changed (some audio interfaces) gets its music
/// paused instead. `None` when there's nothing to do: no device, or it's
/// already silent or muted.
pub fn quiet_output(how: Quieten) -> Option<QuietOutput> {
    // Put back anything still quieted first, so it isn't taken as the
    // level to return to.
    drop(quieted().take());
    let quiet = match how {
        Quieten::Pause => Quieted::Paused(players::pause()),
        Quieten::Lower | Quieten::Mute => match imp::quiet_output(how) {
            Some(quiet) => Quieted::Output(quiet),
            None if !imp::can_quiet_output(how) => {
                tracing::info!("the sound's volume can't be changed; pausing music instead");
                Quieted::Paused(players::pause())
            }
            None => return None,
        },
    };
    let n = QUIET_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *quieted() = Some((n, quiet));
    Some(QuietOutput(n))
}

impl Drop for QuietOutput {
    fn drop(&mut self) {
        let mut slot = quieted();
        if slot.as_ref().is_some_and(|(n, _)| *n == self.0) {
            let quiet = slot.take();
            drop(slot);
            drop(quiet);
        }
    }
}

/// Puts back sound lowered or muted for a recording: for quitting or
/// restarting while one is in progress.
pub fn restore_output() {
    drop(quieted().take());
    players::settle(std::time::Duration::from_secs(3));
}
