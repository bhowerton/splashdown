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
    pub(crate) fn set(&self, v: f32) {
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
        Self::record(&device, config)
    }

    /// Records what the default playback device plays (WASAPI loopback):
    /// the other side of a call, for meeting notes.
    #[cfg(windows)]
    pub fn start_system_output() -> Result<Self, CaptureError> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(CaptureError::NoDevice)?;
        let config = device
            .default_output_config()
            .map_err(|e| CaptureError::Device(e.to_string()))?;
        // An input stream on an output device is a loopback recording.
        Self::record(&device, config)
    }

    fn record(
        device: &cpal::Device,
        config: cpal::SupportedStreamConfig,
    ) -> Result<Self, CaptureError> {
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
                let mut mixer = Mixer::new(channels);
                device.build_input_stream(
                    &config.clone().into(),
                    move |data: &[$t], _| {
                        push_mono(data, &mut mixer, &buffer, &level, |s| {
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

/// A channel counts as carrying sound while its power is at least this
/// much of the loudest channel's (-20 dB).
const LIVE_CHANNEL: f32 = 0.01;
/// How much of each buffer's power goes into a channel's running power:
/// about a tenth of a second to follow a change.
const POWER_FOLLOWS: f32 = 0.1;

/// What the audio callback keeps from one call to the next: which channels
/// carry sound, and mono audio not yet handed to the recorder.
struct Mixer {
    channels: usize,
    /// Each channel's running power, once a buffer has come.
    power: Vec<f32>,
    /// This buffer's power per channel (kept here so the callback doesn't
    /// allocate).
    now: Vec<f32>,
    heard: bool,
    /// Mixed audio the recorder hasn't taken yet: a reader held the buffer
    /// when it came. It goes in, in order, with the next that gets through.
    waiting: Vec<f32>,
}

impl Mixer {
    fn new(channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            channels,
            power: vec![0.0; channels],
            now: vec![0.0; channels],
            heard: false,
            waiting: Vec::new(),
        }
    }

    /// Mixes interleaved frames down to mono onto `waiting`: the average
    /// of the channels that carry sound, not of all of them. An audio
    /// interface with a microphone on one input of eight would otherwise
    /// record it at an eighth of its level. Returns the buffer's loudness
    /// (RMS), if it held any frames.
    fn mix<T: Copy>(&mut self, data: &[T], convert: impl Fn(T) -> f32) -> Option<f32> {
        let n = self.channels;
        let frames = data.len() / n;
        if frames == 0 {
            return None;
        }
        let mut sum_sq = 0.0f32;
        if n == 1 {
            for &x in data {
                let s = convert(x);
                self.waiting.push(s);
                sum_sq += s * s;
            }
            return Some((sum_sq / frames as f32).sqrt());
        }
        self.now.fill(0.0);
        for frame in data.chunks_exact(n) {
            for (power, &x) in self.now.iter_mut().zip(frame) {
                let s = convert(x);
                *power += s * s;
            }
        }
        for (power, now) in self.power.iter_mut().zip(&self.now) {
            let now = now / frames as f32;
            *power = if self.heard {
                *power + POWER_FOLLOWS * (now - *power)
            } else {
                now
            };
        }
        self.heard = true;
        let loudest = self.power.iter().copied().fold(0.0f32, f32::max);
        let floor = loudest * LIVE_CHANNEL;
        // All of them in silence, when none stands out.
        let live = self.power.iter().filter(|&&p| p >= floor).count().max(1);
        for frame in data.chunks_exact(n) {
            let sum: f32 = frame
                .iter()
                .zip(&self.power)
                .filter(|&(_, &p)| p >= floor)
                .map(|(&x, _)| convert(x))
                .sum();
            let s = sum / live as f32;
            self.waiting.push(s);
            sum_sq += s * s;
        }
        Some((sum_sq / frames as f32).sqrt())
    }
}

fn push_mono<T: Copy>(
    data: &[T],
    mixer: &mut Mixer,
    buffer: &Mutex<Vec<f32>>,
    level: &Level,
    convert: impl Fn(T) -> f32,
) {
    if let Some(rms) = mixer.mix(data, convert) {
        level.set(rms);
    }
    // Never block the audio thread. While a reader holds the buffer (the
    // live preview copying the recording so far), the audio waits in the
    // mixer for the next call rather than being dropped.
    if let Ok(mut buf) = buffer.try_lock() {
        buf.append(&mut mixer.waiting);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize) -> Vec<f32> {
        (0..frames).map(|i| (i as f32 * 0.3).sin() * 0.5).collect()
    }

    #[test]
    fn nothing_is_dropped_while_a_reader_holds_the_buffer() {
        let buffer = Mutex::new(Vec::new());
        let level = Level::default();
        let mut mixer = Mixer::new(1);
        push_mono(&[0.1f32, 0.2], &mut mixer, &buffer, &level, |s| s);
        {
            // The live preview is copying the recording so far.
            let _reader = buffer.lock().unwrap();
            push_mono(&[0.3f32, 0.4], &mut mixer, &buffer, &level, |s| s);
            push_mono(&[0.5f32], &mut mixer, &buffer, &level, |s| s);
        }
        push_mono(&[0.6f32], &mut mixer, &buffer, &level, |s| s);
        assert_eq!(*buffer.lock().unwrap(), [0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        assert!(mixer.waiting.is_empty());
    }

    #[test]
    fn a_microphone_on_one_input_of_eight_keeps_its_level() {
        let voice = tone(480);
        let mut data = Vec::new();
        for &s in &voice {
            data.push(s);
            data.extend([0.0f32; 7]);
        }
        let mut mixer = Mixer::new(8);
        let rms = mixer.mix(&data, |s| s).unwrap();
        assert_eq!(mixer.waiting, voice);
        assert!(rms > 0.3, "{rms}");
    }

    #[test]
    fn channels_that_all_carry_sound_are_averaged() {
        // A stereo microphone: the same voice on both, one a little quieter.
        let voice = tone(480);
        let data: Vec<f32> = voice.iter().flat_map(|&s| [s, s * 0.5]).collect();
        let mut mixer = Mixer::new(2);
        mixer.mix(&data, |s| s).unwrap();
        for (mixed, s) in mixer.waiting.iter().zip(&voice) {
            assert!((mixed - s * 0.75).abs() < 1e-6);
        }
        // And silence on every channel is silence, not a division by zero.
        let mut mixer = Mixer::new(2);
        assert_eq!(mixer.mix(&[0.0f32; 8], |s| s), Some(0.0));
        assert_eq!(mixer.waiting, [0.0; 4]);
        assert_eq!(mixer.mix(&[0.0f32; 1], |s| s), None, "not a whole frame");
    }
}
