//! Playing speech made on the computer (the natural reading voice): mono
//! pieces at their own rate, played on the default output device as they
//! arrive, converted to its rate and channels.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

/// What the output callback plays from: samples at the device's rate,
/// mono (copied to every channel).
type Queue = Arc<Mutex<VecDeque<f32>>>;

/// How far ahead of what's playing speech is queued.
const AHEAD_SECONDS: usize = 5;

/// Plays `pieces` (mono, at `rate`) as they come, until they run out or
/// `stop` is set. Blocks until it's done.
pub fn play<I>(pieces: I, rate: u32, stop: &AtomicBool) -> Result<(), String>
where
    I: IntoIterator<Item = Result<Vec<f32>, String>>,
{
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no sound output")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let queue: Queue = Arc::default();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => open::<f32>(&device, &config.config(), &queue),
        cpal::SampleFormat::I16 => open::<i16>(&device, &config.config(), &queue),
        cpal::SampleFormat::U16 => open::<u16>(&device, &config.config(), &queue),
        cpal::SampleFormat::I32 => open::<i32>(&device, &config.config(), &queue),
        other => return Err(format!("can't play {other} audio")),
    }?;
    stream.play().map_err(|e| e.to_string())?;

    let mut convert = Converter::new(rate, config.sample_rate().0);
    for piece in pieces {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        let piece = piece?;
        let converted = convert.push(&piece);
        queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(converted);
        // Speech is made much faster than it's played: keep only a few
        // seconds ahead, so a long text doesn't fill memory.
        let ahead = AHEAD_SECONDS * config.sample_rate().0 as usize;
        while !stop.load(Ordering::SeqCst)
            && queue.lock().unwrap_or_else(|e| e.into_inner()).len() > ahead
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    // Until what's queued has been played.
    while !stop.load(Ordering::SeqCst)
        && !queue.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    // The device's own buffer.
    if !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(120));
    }
    Ok(())
}

fn open<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: &Queue,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    let queue = queue.clone();
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                let mut queue = queue.lock().unwrap_or_else(|e| e.into_inner());
                for frame in out.chunks_mut(channels) {
                    let sample = queue.pop_front().unwrap_or(0.0);
                    frame.fill(T::from_sample(sample));
                }
            },
            |e| tracing::warn!("playing speech: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Converts a stream of mono samples from one rate to another, linearly,
/// carrying its position from one piece to the next.
struct Converter {
    /// Input samples per output sample.
    step: f64,
    /// Where the next output sample falls, counted from `last`.
    at: f64,
    /// The last input sample of the previous piece.
    last: f32,
}

impl Converter {
    fn new(from: u32, to: u32) -> Self {
        Self {
            step: f64::from(from) / f64::from(to.max(1)),
            at: 1.0,
            last: 0.0,
        }
    }

    fn push(&mut self, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 1);
        // Index 0 is `last`, then the piece.
        let sample = |i: usize| if i == 0 { self.last } else { input[i - 1] };
        while self.at <= input.len() as f64 {
            let i = self.at.floor() as usize;
            let frac = (self.at - i as f64) as f32;
            let a = sample(i);
            let b = if i < input.len() { sample(i + 1) } else { a };
            out.push(a + (b - a) * frac);
            self.at += self.step;
        }
        self.at -= input.len() as f64;
        if let Some(&last) = input.last() {
            self.last = last;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_the_rate_across_pieces() {
        // 24 kHz to 48 kHz: twice as many samples, whatever the pieces (the
        // last one waits for the next piece to land between).
        let ramp: Vec<f32> = (0..240).map(|i| i as f32).collect();
        let mut whole = Converter::new(24_000, 48_000);
        let at_once = whole.push(&ramp);
        let mut pieces = Converter::new(24_000, 48_000);
        let mut split = Vec::new();
        for piece in ramp.chunks(37) {
            split.extend(pieces.push(piece));
        }
        assert_eq!(at_once.len(), 479);
        assert_eq!(split, at_once);
        // Halfway between neighbours.
        assert!((at_once[11] - 5.5).abs() < 1e-4, "{}", at_once[11]);
        // The same rate passes samples through.
        let mut same = Converter::new(24_000, 24_000);
        assert_eq!(same.push(&[1.0, 2.0, 3.0]), vec![1.0, 2.0, 3.0]);
    }
}
