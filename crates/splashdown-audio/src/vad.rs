//! Energy-based silence trimming. Good enough to drop dead air around push-
//! to-talk recordings and to detect "nothing was said"; a model-based VAD
//! can replace it later without changing the interface.

use splashdown_core::AudioClip;

const FRAME_MS: u32 = 20;
/// Silence kept before and after speech so word edges aren't clipped.
const PAD_MS: u32 = 200;
/// Absolute floor: frames quieter than this are never speech.
const MIN_RMS: f32 = 0.008;
/// A frame is speech if it's this much louder than the noise floor.
const NOISE_MULTIPLIER: f32 = 3.0;
/// At least this much speech is needed for the clip to count.
const MIN_SPEECH_MS: u32 = 120;

/// Trims leading/trailing silence. Returns `None` if the clip has no speech.
pub fn trim_silence(clip: &AudioClip) -> Option<AudioClip> {
    let frame = (clip.sample_rate * FRAME_MS / 1000).max(1) as usize;
    let energies: Vec<f32> = clip.samples.chunks(frame).map(rms).collect();
    if energies.is_empty() {
        return None;
    }

    // Noise floor: the 10th-percentile frame energy.
    let mut sorted = energies.clone();
    sorted.sort_by(f32::total_cmp);
    let noise = sorted[sorted.len() / 10];
    let threshold = (noise * NOISE_MULTIPLIER).max(MIN_RMS);

    let voiced: Vec<usize> = energies
        .iter()
        .enumerate()
        .filter(|(_, e)| **e > threshold)
        .map(|(i, _)| i)
        .collect();
    let min_frames = (MIN_SPEECH_MS / FRAME_MS) as usize;
    if voiced.len() < min_frames {
        return None;
    }

    let pad = (PAD_MS / FRAME_MS) as usize;
    let first = voiced[0].saturating_sub(pad);
    let last = (voiced[voiced.len() - 1] + pad + 1).min(energies.len());
    let start = first * frame;
    let end = (last * frame).min(clip.samples.len());
    Some(AudioClip::new(
        clip.samples[start..end].to_vec(),
        clip.sample_rate,
    ))
}

/// The latest pause of at least `min_pause_ms` that starts at or after
/// sample `from` and ends before the last `guard_ms` of the clip. Returns
/// the sample index in the middle of the pause, a safe place to split
/// speech. `None` if there's no such pause (or no speech before it).
pub fn find_pause(
    clip: &AudioClip,
    from: usize,
    min_pause_ms: u32,
    guard_ms: u32,
) -> Option<usize> {
    let frame = (clip.sample_rate * FRAME_MS / 1000).max(1) as usize;
    let energies: Vec<f32> = clip.samples.chunks(frame).map(rms).collect();
    if energies.is_empty() {
        return None;
    }
    let mut sorted = energies.clone();
    sorted.sort_by(f32::total_cmp);
    let noise = sorted[sorted.len() / 10];
    let threshold = (noise * NOISE_MULTIPLIER).max(MIN_RMS);
    // Only split after some speech, or there's nothing to transcribe.
    if !energies.iter().any(|e| *e > threshold) {
        return None;
    }

    let min_frames = (min_pause_ms / FRAME_MS).max(1) as usize;
    let first = from.div_ceil(frame);
    let last = energies
        .len()
        .saturating_sub((guard_ms / FRAME_MS) as usize);
    let mut best = None;
    let mut run_start = None;
    for (i, energy) in energies.iter().enumerate().take(last).skip(first) {
        if *energy <= threshold {
            let start = *run_start.get_or_insert(i);
            if i + 1 - start >= min_frames {
                best = Some((start + i).div_ceil(2) * frame);
            }
        } else {
            run_start = None;
        }
    }
    best.map(|i: usize| i.min(clip.samples.len()))
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip_with_speech(rate: u32, silence_s: f32, speech_s: f32) -> AudioClip {
        let n = |s: f32| (s * rate as f32) as usize;
        let mut samples = vec![0.001f32; n(silence_s)];
        samples.extend((0..n(speech_s)).map(|i| (i as f32 * 0.3).sin() * 0.3));
        samples.extend(vec![0.001f32; n(silence_s)]);
        AudioClip::new(samples, rate)
    }

    #[test]
    fn trims_surrounding_silence_with_padding() {
        let clip = clip_with_speech(16_000, 2.0, 1.0);
        let trimmed = trim_silence(&clip).unwrap();
        let secs = trimmed.duration().as_secs_f32();
        assert!((1.3..1.5).contains(&secs), "got {secs}s");
    }

    #[test]
    fn silence_has_no_speech() {
        let clip = AudioClip::new(vec![0.001; 32_000], 16_000);
        assert!(trim_silence(&clip).is_none());
        assert!(trim_silence(&AudioClip::new(vec![], 16_000)).is_none());
    }

    #[test]
    fn finds_the_latest_pause_after_speech() {
        let rate = 16_000;
        let speech = |s: f32| (0..(s * rate as f32) as usize).map(|i| (i as f32 * 0.3).sin() * 0.3);
        let quiet = |s: f32| std::iter::repeat_n(0.001f32, (s * rate as f32) as usize);
        // speech 2 s · pause 0.6 s · speech 2 s · pause 0.6 s · speech 1 s
        let samples: Vec<f32> = speech(2.0)
            .chain(quiet(0.6))
            .chain(speech(2.0))
            .chain(quiet(0.6))
            .chain(speech(1.0))
            .collect();
        let clip = AudioClip::new(samples, rate);
        let at = find_pause(&clip, 0, 400, 300).expect("a pause") as f32 / rate as f32;
        assert!(
            (4.8..5.0).contains(&at),
            "split in the second pause, got {at}s"
        );
        // Starting after the second pause: none left.
        assert!(find_pause(&clip, (5.3 * rate as f32) as usize, 400, 300).is_none());
        // Pauses shorter than asked for don't count.
        assert!(find_pause(&clip, 0, 800, 300).is_none());
        // Silence alone isn't split.
        let silent = AudioClip::new(vec![0.001; 5 * rate as usize], rate);
        assert!(find_pause(&silent, 0, 400, 300).is_none());
    }

    #[test]
    fn click_is_not_speech() {
        let mut samples = vec![0.0f32; 32_000];
        samples[16_000..16_100].fill(0.9);
        assert!(trim_silence(&AudioClip::new(samples, 16_000)).is_none());
    }
}
