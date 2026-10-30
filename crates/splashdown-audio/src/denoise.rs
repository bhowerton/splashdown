//! Background noise reduction: removes steady noise (fans, traffic, café
//! chatter) from a recording before speech-to-text, with RNNoise (the
//! `nnnoiseless` port, pure Rust).
//!
//! macOS has a better-known option, voice isolation, but it's part of
//! AVAudioEngine's voice processing; our capture goes through cpal, which
//! doesn't expose it, and it wouldn't help on Windows.

use nnnoiseless::DenoiseState;
use splashdown_core::AudioClip;

use crate::resample;

/// The only rate RNNoise works at.
pub const RATE: u32 = 48_000;
/// RNNoise expects samples at 16-bit scale, not -1.0..1.0.
const SCALE: f32 = 32_768.0;

/// Returns `clip` with steady background noise removed, at 48 kHz.
///
/// Timing, release build on an M-series Mac: about 30 ms for a 10-second
/// clip recorded at 48 kHz (most Mac microphones); about 180 ms at 44.1 kHz
/// or 16 kHz, most of it resampling to 48 kHz first.
pub fn denoise(clip: &AudioClip) -> AudioClip {
    if clip.samples.is_empty() {
        return AudioClip::new(Vec::new(), RATE);
    }
    let samples = resample::resample(&clip.samples, clip.sample_rate, RATE);
    let frame = DenoiseState::FRAME_SIZE;
    let mut state = DenoiseState::new();
    let mut input = vec![0.0; frame];
    let mut output = vec![0.0; frame];
    let mut out = Vec::with_capacity(samples.len() + frame);
    // Each frame's output is the frame before it, so one frame of silence
    // goes in at the end and the first frame out is dropped.
    for i in 0..=samples.len().div_ceil(frame) {
        let piece = samples.get(i * frame..).unwrap_or_default();
        input.fill(0.0);
        for (to, from) in input.iter_mut().zip(piece) {
            *to = from * SCALE;
        }
        state.process_frame(&mut output, &input);
        if i > 0 {
            out.extend(output.iter().map(|s| (s / SCALE).clamp(-1.0, 1.0)));
        }
    }
    out.truncate(samples.len());
    AudioClip::new(out, RATE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// Test signals are made at 16 kHz, like a headset microphone, so the
    /// noise is band-limited the way real noise is. (White noise right up to
    /// 24 kHz is RNNoise's weak spot: it learned from recorded noise, and
    /// leaves anywhere from 15% to 90% of that.)
    const TEST_RATE: u32 = 16_000;

    /// Repeatable white noise with this RMS level.
    fn white(len: usize, level: f32) -> Vec<f32> {
        let mut x: u32 = 0x9E37_79B9;
        let mut uniform = move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as f32 / u32::MAX as f32
        };
        (0..len)
            .map(|_| {
                let (a, b) = (uniform().max(1e-9), uniform());
                (-2.0 * a.ln()).sqrt() * (2.0 * PI * b).cos() * level
            })
            .collect()
    }

    /// A low rumble, like a fan or traffic.
    fn rumble(len: usize, level: f32) -> Vec<f32> {
        let mut acc = 0.0;
        let raw: Vec<f32> = white(len, 1.0)
            .into_iter()
            .map(|w| {
                acc = acc * 0.98 + w;
                acc
            })
            .collect();
        let scale = level / rms(&raw);
        raw.into_iter().map(|s| s * scale).collect()
    }

    /// Something like a voice: a 140 Hz buzz with its harmonics (shaped by
    /// two formants), gliding in pitch, in syllables four times a second.
    fn voice(len: usize) -> Vec<f32> {
        let rate = TEST_RATE as f32;
        let mut phase = 0.0f32;
        (0..len)
            .map(|i| {
                let t = i as f32 / rate;
                let f0 = 140.0 + 20.0 * (2.0 * PI * 0.7 * t).sin();
                phase += 2.0 * PI * f0 / rate;
                let mut s = 0.0;
                for k in 1..30 {
                    let f = f0 * k as f32;
                    let formants = (-((f - 700.0) / 300.0).powi(2)).exp()
                        + 0.6 * (-((f - 1_800.0) / 400.0).powi(2)).exp()
                        + 0.05;
                    s += formants * (phase * k as f32).sin();
                }
                let syllable = (PI * 4.0 * t).sin().abs().powf(0.6);
                s * syllable * 0.08
            })
            .collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt()
    }

    /// How much of `before`'s level is left after denoising, ignoring the
    /// first two seconds while the model settles.
    fn left(before: &[f32], after: &AudioClip) -> f32 {
        let before = resample::resample(before, TEST_RATE, RATE);
        let settled = 2 * RATE as usize..before.len();
        rms(&after.samples[settled.clone()]) / rms(&before[settled])
    }

    #[test]
    fn keeps_a_voice_and_removes_noise() {
        let len = 5 * TEST_RATE as usize;
        let speech = voice(len);
        let hiss = white(len, 0.015);
        let noisy: Vec<f32> = speech.iter().zip(&hiss).map(|(s, n)| s + n).collect();

        let clean = denoise(&AudioClip::new(noisy, TEST_RATE));
        assert_eq!(clean.samples.len(), 5 * RATE as usize);
        let kept = left(&speech, &clean);
        assert!(kept > 0.6, "voice kept at {kept:.2} of its level");

        let hiss_left = left(&hiss, &denoise(&AudioClip::new(hiss.clone(), TEST_RATE)));
        assert!(hiss_left < 0.1, "hiss left at {hiss_left:.3}");
        let fan = rumble(len, 0.02);
        let fan_left = left(&fan, &denoise(&AudioClip::new(fan.clone(), TEST_RATE)));
        assert!(fan_left < 0.1, "rumble left at {fan_left:.3}");
    }

    #[test]
    fn other_rates_and_empty_clips() {
        let out = denoise(&AudioClip::new(white(16_000, 0.01), 16_000));
        assert_eq!(out.sample_rate, RATE);
        assert_eq!(out.samples.len(), 48_000);
        assert!(
            denoise(&AudioClip::new(Vec::new(), 16_000))
                .samples
                .is_empty()
        );
    }

    /// `cargo test -p splashdown-audio --release -- --ignored --nocapture
    /// denoise_speed`
    #[test]
    #[ignore]
    fn denoise_speed() {
        for rate in [48_000, 44_100, 16_000] {
            let clip = AudioClip::new(white(10 * rate as usize, 0.05), rate);
            let start = std::time::Instant::now();
            let _ = denoise(&clip);
            println!("10 s at {rate} Hz: {:?}", start.elapsed());
        }
    }
}
