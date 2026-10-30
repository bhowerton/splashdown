//! Whisper mode: raises quiet speech to a normal level before
//! speech-to-text.

use splashdown_core::AudioClip;

/// The level the loudest stretches of speech are raised to (RMS).
const TARGET_RMS: f32 = 0.1;
/// Never amplify more than this (≈ 28 dB), so room noise stays noise.
const MAX_GAIN: f32 = 25.0;
const FRAME_MS: u32 = 20;

/// Scales `clip` so its loudest stretches (the 95th percentile of 20 ms
/// frames) reach `TARGET_RMS`. Never turns audio down; peaks are softly
/// limited instead of clipping.
pub fn boost_quiet(clip: &AudioClip) -> AudioClip {
    let frame = (clip.sample_rate * FRAME_MS / 1000).max(1) as usize;
    let mut levels: Vec<f32> = clip
        .samples
        .chunks(frame)
        .map(|f| (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt())
        .collect();
    if levels.is_empty() {
        return clip.clone();
    }
    levels.sort_by(f32::total_cmp);
    let loud = levels[(levels.len() * 95 / 100).min(levels.len() - 1)];
    let gain = if loud > 0.0 {
        (TARGET_RMS / loud).clamp(1.0, MAX_GAIN)
    } else {
        1.0
    };
    if gain <= 1.0 {
        return clip.clone();
    }
    let samples = clip.samples.iter().map(|s| limit(s * gain)).collect();
    AudioClip::new(samples, clip.sample_rate)
}

/// Passes quiet samples through and bends loud ones under 1.0.
fn limit(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let over = a - KNEE;
        x.signum() * (KNEE + (1.0 - KNEE) * (over / (over + (1.0 - KNEE))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32, seconds: f32) -> Vec<f32> {
        (0..(seconds * 16_000.0) as usize)
            .map(|i| (i as f32 * 0.2).sin() * amplitude)
            .collect()
    }

    #[test]
    fn quiet_speech_is_raised_and_found() {
        // A whisper: well under the silence threshold.
        let mut samples = vec![0.0005; 8_000];
        samples.extend(tone(0.006, 1.0));
        samples.extend(vec![0.0005; 8_000]);
        let clip = AudioClip::new(samples, 16_000);
        assert!(crate::vad::trim_silence(&clip).is_none(), "too quiet as is");
        let boosted = boost_quiet(&clip);
        assert!(
            crate::vad::trim_silence(&boosted).is_some(),
            "heard once raised"
        );
        assert!(boosted.samples.iter().all(|s| s.abs() < 1.0));
    }

    #[test]
    fn normal_speech_is_left_alone() {
        let clip = AudioClip::new(tone(0.3, 1.0), 16_000);
        assert_eq!(boost_quiet(&clip).samples, clip.samples);
        let silent = AudioClip::new(vec![0.0; 1600], 16_000);
        assert_eq!(boost_quiet(&silent).samples, silent.samples);
    }

    #[test]
    fn limiter_stays_under_one() {
        for x in [0.5, 0.8, 0.9, 2.0, 30.0, -30.0] {
            assert!(limit(x).abs() < 1.0 || x.abs() <= 0.8);
        }
        assert_eq!(limit(0.5), 0.5);
    }
}
