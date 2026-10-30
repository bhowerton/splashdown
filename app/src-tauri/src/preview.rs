//! Live preview: while the user speaks, a local speech-to-text model
//! re-transcribes the recording so far and the overlay shows the words.
//! It's display only; the inserted text always comes from the full pipeline
//! on release.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use splashdown_core::{AudioClip, SpeechToText, SttHints};

/// Only the end of a long recording is previewed, which keeps each pass
/// fast; the overlay shows the tail anyway.
const MAX_SECONDS: usize = 20;

pub struct Job {
    pub session: u64,
    pub clip: AudioClip,
}

pub struct Partial {
    pub session: u64,
    pub text: String,
}

/// Runs preview transcriptions on its own thread. Jobs that pile up while a
/// pass is running are dropped in favor of the newest one.
pub struct PreviewWorker {
    tx: Sender<Job>,
}

impl PreviewWorker {
    pub fn spawn(
        stt: Arc<dyn SpeechToText>,
        hints: SttHints,
        on_partial: impl Fn(Partial) + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("splashdown-preview".into())
            .spawn(move || run(rx, stt, hints, on_partial))
            .expect("spawn preview thread");
        Self { tx }
    }

    pub fn submit(&self, job: Job) {
        let _ = self.tx.send(job);
    }
}

fn run(
    rx: Receiver<Job>,
    stt: Arc<dyn SpeechToText>,
    hints: SttHints,
    on_partial: impl Fn(Partial),
) {
    while let Ok(mut job) = rx.recv() {
        // Skip to the newest job.
        while let Ok(newer) = rx.try_recv() {
            job = newer;
        }
        let clip = tail(job.clip);
        let text = match tauri::async_runtime::block_on(stt.transcribe(&clip, &hints)) {
            Ok(t) => t.text,
            Err(e) => {
                tracing::debug!("preview failed: {e}");
                String::new()
            }
        };
        on_partial(Partial {
            session: job.session,
            text: text.trim().to_string(),
        });
    }
}

/// Resamples to 16 kHz and keeps at most the last `MAX_SECONDS`.
fn tail(clip: AudioClip) -> AudioClip {
    let rate = splashdown_audio::TARGET_SAMPLE_RATE;
    let mut samples = splashdown_audio::resample::resample(&clip.samples, clip.sample_rate, rate);
    let max = MAX_SECONDS * rate as usize;
    if samples.len() > max {
        samples.drain(..samples.len() - max);
    }
    AudioClip::new(samples, rate)
}
