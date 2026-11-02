//! Pausing music while recording, and playing it again afterwards: the
//! players are asked one by one (AppleScript on macOS, the media controls
//! Windows shows on Windows), which takes a moment, so it happens on a
//! thread of its own. Jobs run in the order they're sent, so playing again
//! after one recording always comes before pausing for the next.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

enum Job {
    Pause(u64),
    Resume(u64),
    Done(Sender<()>),
}

static JOBS: OnceLock<Sender<Job>> = OnceLock::new();
static COUNT: AtomicU64 = AtomicU64::new(0);

fn send(job: Job) {
    let jobs = JOBS.get_or_init(|| {
        let (jobs, received) = channel::<Job>();
        let started = std::thread::Builder::new()
            .name("splashdown-players".into())
            .spawn(move || {
                // What each pause paused, by its number.
                let mut paused = HashMap::new();
                for job in received {
                    match job {
                        Job::Pause(n) => {
                            let players = crate::imp::pause_players();
                            if !players.is_empty() {
                                tracing::info!("paused {} while recording", players.join(", "));
                            }
                            paused.insert(n, players);
                        }
                        Job::Resume(n) => {
                            if let Some(players) = paused.remove(&n)
                                && !players.is_empty()
                            {
                                crate::imp::resume_players(&players);
                            }
                        }
                        Job::Done(reply) => {
                            let _ = reply.send(());
                        }
                    }
                }
            });
        if let Err(e) = started {
            tracing::warn!("couldn't start pausing music: {e}");
        }
        jobs
    });
    let _ = jobs.send(job);
}

/// Music paused for a recording; dropping it plays what was paused again.
pub(crate) struct Paused(u64);

/// Pauses the players that are playing (in the background).
pub(crate) fn pause() -> Paused {
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    send(Job::Pause(n));
    Paused(n)
}

impl Drop for Paused {
    fn drop(&mut self) {
        send(Job::Resume(self.0));
    }
}

/// Waits (up to `limit`) until the pausing and playing asked for so far
/// is done: before a media key, which would otherwise race it, and before
/// quitting.
pub(crate) fn settle(limit: Duration) {
    if JOBS.get().is_none() {
        return;
    }
    let (reply, done) = channel();
    send(Job::Done(reply));
    let _ = done.recv_timeout(limit);
}
