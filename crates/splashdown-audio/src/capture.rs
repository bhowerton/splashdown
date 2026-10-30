//! Microphone capture via cpal.
//!
//! `cpal::Stream` isn't `Send` on every platform, so a `Recorder` must stay on
//! the thread that created it. The app runs it on a dedicated audio thread.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use splashdown_core::AudioClip;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("no microphone found")]
    NoDevice,
    #[error("microphone \"{0}\" not found")]
    DeviceNotFound(String),
    #[error("microphone error: {0}")]
    Device(String),
    #[error("unsupported microphone sample format {0:?}")]
    UnsupportedFormat(SampleFormat),
}

/// Names of the available input devices.
pub fn input_devices() -> Result<Vec<String>, CaptureError> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|e| CaptureError::Device(e.to_string()))?;
    Ok(devices.filter_map(|d| d.name().ok()).collect())
}

/// Shared, lock-free view of the current input level (RMS of the last
/// callback buffer, 0.0–1.0) for the overlay waveform, and of whether the
/// microphone has gone away.
#[derive(Clone, Default)]
pub struct Level(Arc<LevelInner>);

#[derive(Default)]
struct LevelInner {
    rms: AtomicU32,
    lost: AtomicBool,
}

impl Level {
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.rms.load(Ordering::Relaxed))
    }
    fn set(&self, v: f32) {
        self.0.rms.store(v.to_bits(), Ordering::Relaxed);
    }
    /// The microphone was unplugged or switched off mid-recording. What it
    /// recorded until then is kept.
    pub fn lost(&self) -> bool {
        self.0.lost.load(Ordering::Relaxed)
    }
}

pub struct Recorder {
    stream: Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    level: Level,
}

impl Recorder {
    /// Opens the named input device (or the system default) and starts
    /// recording immediately.
    pub fn start(device_name: Option<&str>) -> Result<Self, CaptureError> {
        let host = cpal::default_host();
        let device = match device_name {
            None => host.default_input_device().ok_or(CaptureError::NoDevice)?,
            Some(name) => host
                .input_devices()
                .map_err(|e| CaptureError::Device(e.to_string()))?
                .find(|d| d.name().is_ok_and(|n| n == name))
                .ok_or_else(|| CaptureError::DeviceNotFound(name.to_string()))?,
        };
        let config = device
            .default_input_config()
            .map_err(|e| CaptureError::Device(e.to_string()))?;

        let sample_rate = config.sample_rate().0;
        let channels = usize::from(config.channels().max(1));
        let buffer = Arc::new(Mutex::new(Vec::with_capacity(sample_rate as usize * 30)));
        let level = Level::default();
        let lost = level.clone();
        let on_error = move |e: cpal::StreamError| {
            tracing::warn!("microphone stream error: {e}");
            if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                lost.0.lost.store(true, Ordering::Relaxed);
            }
        };

        macro_rules! build {
            ($t:ty) => {{
                let buffer = buffer.clone();
                let level = level.clone();
                device.build_input_stream(
                    &config.clone().into(),
                    move |data: &[$t], _| {
                        push_mono(data, channels, &buffer, &level, |s| {
                            <f32 as cpal::FromSample<$t>>::from_sample_(s)
                        })
                    },
                    on_error,
                    None,
                )
            }};
        }

        let stream = match config.sample_format() {
            SampleFormat::F32 => build!(f32),
            SampleFormat::I16 => build!(i16),
            SampleFormat::U16 => build!(u16),
            SampleFormat::I32 => build!(i32),
            other => return Err(CaptureError::UnsupportedFormat(other)),
        }
        .map_err(|e| CaptureError::Device(e.to_string()))?;
        stream
            .play()
            .map_err(|e| CaptureError::Device(e.to_string()))?;

        Ok(Self {
            stream,
            buffer,
            sample_rate,
            level,
        })
    }

    pub fn level(&self) -> Level {
        self.level.clone()
    }

    /// Takes everything recorded so far, leaving the recorder running. Used
    /// to split content from instruction at the instruction-key tap.
    pub fn take(&self) -> AudioClip {
        let samples = std::mem::take(&mut *self.buffer.lock().expect("audio buffer"));
        AudioClip::new(samples, self.sample_rate)
    }

    /// Copies what's been recorded since the last `take`, without removing
    /// it. Used for the live preview while the user is still speaking.
    pub fn snapshot(&self) -> AudioClip {
        let samples = self.buffer.lock().expect("audio buffer").clone();
        AudioClip::new(samples, self.sample_rate)
    }

    /// Copies what's been recorded since the last `take`, starting at sample
    /// `from` (cheaper than `snapshot` for long recordings).
    pub fn since(&self, from: usize) -> AudioClip {
        let buffer = self.buffer.lock().expect("audio buffer");
        let samples = buffer.get(from..).map(<[f32]>::to_vec).unwrap_or_default();
        AudioClip::new(samples, self.sample_rate)
    }

    /// Stops recording and returns what was captured since the last `take`.
    pub fn stop(self) -> AudioClip {
        let _ = self.stream.pause();
        drop(self.stream);
        let samples = std::mem::take(&mut *self.buffer.lock().expect("audio buffer"));
        AudioClip::new(samples, self.sample_rate)
    }
}

fn push_mono<T: Copy>(
    data: &[T],
    channels: usize,
    buffer: &Mutex<Vec<f32>>,
    level: &Level,
    convert: impl Fn(T) -> f32,
) {
    let mut sum_sq = 0.0f32;
    let mut frames = 0usize;
    // Never block the audio thread; a dropped buffer beats a glitch.
    let Ok(mut buf) = buffer.try_lock() else {
        return;
    };
    for frame in data.chunks(channels) {
        let s = frame.iter().map(|&x| convert(x)).sum::<f32>() / channels as f32;
        buf.push(s);
        sum_sq += s * s;
        frames += 1;
    }
    if frames > 0 {
        level.set((sum_sq / frames as f32).sqrt());
    }
}
