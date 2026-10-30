//! Offline windowed-sinc resampler. Recordings are short and resampled once
//! on key-up, so a direct convolution is fast enough (~10 ms for 10 s).

use std::f64::consts::PI;

/// Half-width of the filter in input samples at the output rate.
const HALF_TAPS: f64 = 16.0;

/// Resamples mono `samples` from `from` Hz to `to` Hz.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() || from == 0 || to == 0 {
        return samples.to_vec();
    }

    let ratio = f64::from(from) / f64::from(to);
    // Low-pass at the lower of the two Nyquist rates (slightly below, to
    // leave room for the transition band).
    let cutoff = (1.0 / ratio).min(1.0) * 0.95;
    let half_width = HALF_TAPS / cutoff;

    let out_len = ((samples.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let center = n as f64 * ratio;
        let start = (center - half_width).ceil().max(0.0) as usize;
        let end = ((center + half_width).floor() as usize).min(samples.len() - 1);

        let mut acc = 0.0f64;
        let mut norm = 0.0f64;
        for (k, &sample) in samples.iter().enumerate().take(end + 1).skip(start) {
            let x = k as f64 - center;
            let w = cutoff * sinc(cutoff * x) * blackman(x / half_width);
            acc += f64::from(sample) * w;
            norm += w;
        }
        out.push(if norm.abs() > 1e-9 {
            (acc / norm) as f32
        } else {
            0.0
        });
    }
    out
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Blackman window over `t` in [-1, 1].
fn blackman(t: f64) -> f64 {
    if t.abs() > 1.0 {
        return 0.0;
    }
    let u = (t + 1.0) / 2.0;
    0.42 - 0.5 * (2.0 * PI * u).cos() + 0.08 * (4.0 * PI * u).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, rate: u32, secs: f64) -> Vec<f32> {
        (0..(f64::from(rate) * secs) as usize)
            .map(|i| (2.0 * PI * freq * i as f64 / f64::from(rate)).sin() as f32 * 0.5)
            .collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn output_length_matches_ratio() {
        let out = resample(&vec![0.0; 48_000], 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        let out = resample(&vec![0.0; 44_100], 44_100, 16_000);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn keeps_speech_band_tone() {
        // 1 kHz is well inside the 8 kHz output band: amplitude preserved.
        let out = resample(&tone(1000.0, 48_000, 0.5), 48_000, 16_000);
        let mid = &out[1000..7000];
        assert!(
            (rms(mid) - 0.5 / 2f32.sqrt()).abs() < 0.01,
            "rms {}",
            rms(mid)
        );
    }

    #[test]
    fn removes_above_nyquist_tone() {
        // 12 kHz can't exist at 16 kHz; it must be filtered, not aliased.
        let out = resample(&tone(12_000.0, 48_000, 0.5), 48_000, 16_000);
        assert!(
            rms(&out[1000..7000]) < 0.01,
            "rms {}",
            rms(&out[1000..7000])
        );
    }

    #[test]
    fn same_rate_is_identity() {
        let input = vec![0.1, 0.2, 0.3];
        assert_eq!(resample(&input, 16_000, 16_000), input);
    }
}
