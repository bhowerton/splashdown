//! Audio handed over as it's recorded, for meetings: the microphone, and
//! what the computer plays (WASAPI loopback here on Windows; the macOS tap
//! lives in `splashdown-apple-shim` and feeds a `Pusher`). Unlike a
//! dictation's `Recorder`, nothing is kept beyond what hasn't been taken.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::capture::{CaptureError, Level, Recorder};
use crate::{TARGET_SAMPLE_RATE, resample};

/// How often the recorder's audio is moved over.
const MOVE_EVERY: Duration = Duration::from_millis(100);

#[derive(Default)]
struct Shared {
    /// Mono samples at `rate`, not yet taken.
    samples: Mutex<Vec<f32>>,
    /// 0 until the first audio says what it is.
    rate: AtomicU32,
    /// The source went away (unplugged, stopped by the system).
    lost: AtomicBool,
}

/// A source of audio recording now. Dropping it stops the recording.
pub struct LiveAudio {
    shared: Arc<Shared>,
    level: Level,
    /// Samples at the source rate held back until a whole resampling step
    /// is there, so nothing is lost or repeated between takes.
    carry: Vec<f32>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Keeps a recording elsewhere (the macOS tap) alive.
    _keep: Option<Box<dyn Send>>,
}

/// Feeds a `LiveAudio` from another recorder (the macOS tap's callback).
#[derive(Clone)]
pub struct Pusher(Arc<Shared>, Level);

impl Pusher {
    /// Mono samples at `rate` Hz. Called on an audio thread: never blocks.
    pub fn push(&self, samples: &[f32], rate: u32) {
        self.0.rate.store(rate, Ordering::Relaxed);
        if let Ok(mut buffer) = self.0.samples.try_lock() {
            buffer.extend_from_slice(samples);
        }
        if !samples.is_empty() {
            let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
            self.1.set(rms);
        }
    }
}

impl LiveAudio {
    /// The microphone (by name, or the system default).
    pub fn microphone(device: Option<&str>) -> Result<Self, CaptureError> {
        let device = device.map(str::to_string);
        Self::from_recorder(move || Recorder::start(device.as_deref()))
    }

    /// What the computer plays, through WASAPI loopback.
    #[cfg(windows)]
    pub fn system_output() -> Result<Self, CaptureError> {
        Self::from_recorder(Recorder::start_system_output)
    }

    /// One fed by a `Pusher`. `keep` (see `keeping`) holds whatever keeps
    /// that recording going, dropped when this is.
    pub fn pushed(keep: Box<dyn Send>) -> (Self, Pusher) {
        let shared = Arc::new(Shared::default());
        let level = Level::default();
        let live = Self {
            shared: shared.clone(),
            level: level.clone(),
            carry: Vec::new(),
            stop: Arc::default(),
            thread: None,
            _keep: Some(keep),
        };
        (live, Pusher(shared, level))
    }

    /// Holds `keep` (the recording feeding the `Pusher`) until this is
    /// dropped.
    pub fn keeping(mut self, keep: Box<dyn Send>) -> Self {
        self._keep = Some(keep);
        self
    }

    /// A `Recorder` on its own thread (cpal's streams can't move between
    /// threads everywhere), its audio moved over every `MOVE_EVERY`.
    fn from_recorder(
        start: impl FnOnce() -> Result<Recorder, CaptureError> + Send + 'static,
    ) -> Result<Self, CaptureError> {
        let shared = Arc::new(Shared::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = std::sync::mpsc::channel();
        let (into, until) = (shared.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("meeting-audio".into())
            .spawn(move || {
                let recorder = match start() {
                    Ok(recorder) => recorder,
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                let _ = ready.send(Ok(recorder.level()));
                while !until.load(Ordering::Relaxed) {
                    std::thread::sleep(MOVE_EVERY);
                    let clip = recorder.take();
                    into.rate.store(clip.sample_rate, Ordering::Relaxed);
                    into.samples
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend(clip.samples);
                    if recorder.level().lost() {
                        into.lost.store(true, Ordering::Relaxed);
                    }
                }
                drop(recorder.stop());
            })
            .map_err(|e| CaptureError::Device(e.to_string()))?;
        let level = started
            .recv()
            .map_err(|_| CaptureError::Device("the recorder didn't start".into()))??;
        Ok(Self {
            shared,
            level,
            carry: Vec::new(),
            stop,
            thread: Some(thread),
            _keep: None,
        })
    }

    /// Everything recorded since the last call, at 16 kHz.
    pub fn take(&mut self) -> Vec<f32> {
        let rate = self.shared.rate.load(Ordering::Relaxed);
        let new = std::mem::take(
            &mut *self
                .shared
                .samples
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        if rate == 0 {
            return Vec::new();
        }
        self.carry.extend(new);
        // Resample whole steps only: rate / gcd input samples make an exact
        // number of output samples.
        let step = (rate / gcd(rate, TARGET_SAMPLE_RATE)) as usize;
        let usable = self.carry.len() - self.carry.len() % step;
        let block: Vec<f32> = self.carry.drain(..usable).collect();
        resample::resample(&block, rate, TARGET_SAMPLE_RATE)
    }

    /// The current input level (0.0–1.0), for the meeting pill.
    pub fn level(&self) -> f32 {
        self.level.get()
    }

    /// The source went away (unplugged, or the system stopped it).
    pub fn lost(&self) -> bool {
        self.shared.lost.load(Ordering::Relaxed)
    }
}

impl Drop for LiveAudio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushed_audio_comes_out_at_16k_without_gaps() {
        let (mut live, pusher) = LiveAudio::pushed(Box::new(()));
        assert!(live.take().is_empty(), "nothing yet");
        // 1 s at 44.1 kHz in uneven pieces: exactly 16,000 samples out.
        let second = vec![0.25f32; 44_100];
        let mut out = Vec::new();
        for piece in second.chunks(1_000) {
            pusher.push(piece, 44_100);
            out.extend(live.take());
        }
        assert_eq!(out.len(), 16_000);
        assert!(live.level() > 0.2);
    }
}
