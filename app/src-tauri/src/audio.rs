//! Owns the microphone on a dedicated thread (cpal streams can't move
//! between threads) and serves start/take/stop requests.

use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use splashdown_audio::{Level, Recorder};
use splashdown_core::AudioClip;

/// How long a finished recording carries on after the key is released.
const RELEASE_TAIL: Duration = Duration::from_millis(200);

enum Command {
    /// Start recording from the preferred device (or the automatic choice).
    /// The reply names the microphone that was opened.
    Start(Vec<String>, Sender<Result<(Level, Option<String>), String>>),
    /// Return what's been recorded so far and keep recording.
    Take(Sender<AudioClip>),
    /// Copy what's been recorded so far, leaving it in place.
    Snapshot(Sender<AudioClip>),
    Stop(Sender<AudioClip>),
    /// Copy what's been recorded from a sample on, leaving it in place.
    Since(usize, Sender<AudioClip>),
}

pub struct AudioWorker {
    tx: Sender<Command>,
}

impl AudioWorker {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel::<Command>();
        std::thread::Builder::new()
            .name("splashdown-audio".into())
            .spawn(move || {
                let mut recorder: Option<Recorder> = None;
                for cmd in rx {
                    match cmd {
                        Command::Start(preferred, reply) => {
                            // A new session replaces any unfinished one.
                            drop(recorder.take());
                            let inputs = splashdown_audio::devices::inputs();
                            let device = splashdown_audio::devices::choose(&preferred, &inputs);
                            let name = splashdown_audio::devices::resolve(&preferred, &inputs);
                            let started = std::time::Instant::now();
                            let result =
                                Recorder::start(device.as_deref()).map_err(|e| e.to_string());
                            tracing::debug!(
                                "mic {} opened in {} ms",
                                device.as_deref().unwrap_or("(system default)"),
                                started.elapsed().as_millis()
                            );
                            let _ = reply.send(
                                result
                                    .as_ref()
                                    .map(|r| (r.level(), name))
                                    .map_err(Clone::clone),
                            );
                            recorder = result.ok();
                        }
                        Command::Take(reply) => {
                            let clip = recorder
                                .as_ref()
                                .map(Recorder::take)
                                .unwrap_or_else(empty_clip);
                            let _ = reply.send(clip);
                        }
                        Command::Snapshot(reply) => {
                            let clip = recorder
                                .as_ref()
                                .map(Recorder::snapshot)
                                .unwrap_or_else(empty_clip);
                            let _ = reply.send(clip);
                        }
                        Command::Since(from, reply) => {
                            let clip = recorder
                                .as_ref()
                                .map(|r| r.since(from))
                                .unwrap_or_else(empty_clip);
                            let _ = reply.send(clip);
                        }
                        Command::Stop(reply) => {
                            let clip = recorder
                                .take()
                                .map(Recorder::stop)
                                .unwrap_or_else(empty_clip);
                            let _ = reply.send(clip);
                        }
                    }
                }
            })
            .expect("spawn audio thread");
        Self { tx }
    }

    pub fn start(&self, preferred: Vec<String>) -> Result<(Level, Option<String>), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Command::Start(preferred, tx))
            .map_err(|_| "audio thread stopped".to_string())?;
        rx.recv().map_err(|_| "audio thread stopped".to_string())?
    }

    pub fn take(&self) -> AudioClip {
        self.request(Command::Take)
    }

    pub fn snapshot(&self) -> AudioClip {
        self.request(Command::Snapshot)
    }

    pub fn stop(&self) -> AudioClip {
        self.request(Command::Stop)
    }

    /// Stops a recording the user finished. People let go of the key as
    /// they say the last syllable, so it keeps recording for `RELEASE_TAIL`
    /// first; cancelling uses `stop`, which doesn't wait.
    pub fn finish(&self) -> AudioClip {
        std::thread::sleep(RELEASE_TAIL);
        self.stop()
    }

    /// The recording from sample `from` on (since the last `take`).
    pub fn since(&self, from: usize) -> AudioClip {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Command::Since(from, tx)).is_err() {
            return empty_clip();
        }
        rx.recv().unwrap_or_else(|_| empty_clip())
    }

    fn request(&self, make: fn(Sender<AudioClip>) -> Command) -> AudioClip {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(make(tx)).is_err() {
            return empty_clip();
        }
        rx.recv().unwrap_or_else(|_| empty_clip())
    }
}

fn empty_clip() -> AudioClip {
    AudioClip::new(Vec::new(), splashdown_audio::TARGET_SAMPLE_RATE)
}
