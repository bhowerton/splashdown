//! Where the speech is: trimming silence, finding pauses to cut long audio
//! at, and telling a voice from noise. A speech detector (Silero, set by
//! the app with `set_detector`) decides when there is one; otherwise
//! loudness against the room's noise floor does.

use std::sync::{Arc, RwLock};

use splashdown_core::AudioClip;

/// A speech detector that scores 16 kHz audio frame by frame (Silero, from
/// the providers crate). Without one, loudness decides.
pub trait SpeechDetector: Send + Sync {
    /// Samples per scored frame.
    fn frame(&self) -> usize;
    /// The chance of speech in each frame of `samples` (16 kHz mono),
    /// or `None` if detection failed.
    fn probabilities(&self, samples: &[f32]) -> Option<Vec<f32>>;
}

static DETECTOR: RwLock<Option<Arc<dyn SpeechDetector>>> = RwLock::new(None);

/// Uses `detector` from now on (`None`: loudness again).
pub fn set_detector(detector: Option<Arc<dyn SpeechDetector>>) {
    *DETECTOR.write().unwrap_or_else(|e| e.into_inner()) = detector;
}

fn detector() -> Option<Arc<dyn SpeechDetector>> {
    DETECTOR.read().unwrap_or_else(|e| e.into_inner()).clone()
}

const FRAME_MS: u32 = 20;
/// Silence kept before and after speech so word edges aren't clipped.
const PAD_MS: u32 = 200;
/// Absolute floor: frames quieter than this are never speech.
const MIN_RMS: f32 = 0.008;
/// A frame is speech if it's this much louder than the noise floor.
const NOISE_MULTIPLIER: f32 = 3.0;
/// At least this much speech is needed for the clip to count.
const MIN_SPEECH_MS: u32 = 120;
/// The detector's chance of speech that starts speech…
const SPEECH_STARTS: f32 = 0.5;
/// …and that keeps it going once started (Silero's own defaults).
const SPEECH_GOES_ON: f32 = 0.35;
/// Next to speech, frames scoring this much are still part of it when
/// trimming.
const SPEECH_EDGE: f32 = 0.1;

/// Which frames of a clip are speech.
pub struct Voicing {
    /// Samples per frame.
    pub frame: usize,
    pub voiced: Vec<bool>,
    /// The detector's chance of speech per frame (none by loudness).
    pub probabilities: Option<Vec<f32>>,
}

impl Voicing {
    /// Frames for `ms` milliseconds at `rate` (at least one).
    fn frames(&self, ms: u32, rate: u32) -> usize {
        ((rate as usize * ms as usize / 1000).div_ceil(self.frame)).max(1)
    }
}

/// Which frames of `clip` are speech: by the speech detector for 16 kHz
/// audio when one is set, otherwise by loudness against the room's noise
/// floor.
pub fn voicing(clip: &AudioClip) -> Voicing {
    by_detector(clip).unwrap_or_else(|| by_loudness(clip))
}

fn by_detector(clip: &AudioClip) -> Option<Voicing> {
    if clip.sample_rate == crate::TARGET_SAMPLE_RATE
        && let Some(detector) = detector()
        && let Some(probabilities) = detector.probabilities(&clip.samples)
    {
        let mut speaking = false;
        let voiced = probabilities
            .iter()
            .map(|&p| {
                speaking = p >= SPEECH_STARTS || (speaking && p >= SPEECH_GOES_ON);
                speaking
            })
            .collect();
        return Some(Voicing {
            frame: detector.frame(),
            voiced,
            probabilities: Some(probabilities),
        });
    }
    None
}

fn by_loudness(clip: &AudioClip) -> Voicing {
    let frame = (clip.sample_rate * FRAME_MS / 1000).max(1) as usize;
    let energies: Vec<f32> = clip.samples.chunks(frame).map(rms).collect();
    let mut sorted = energies.clone();
    sorted.sort_by(f32::total_cmp);
    // Noise floor: the 10th-percentile frame energy.
    let noise = sorted.get(sorted.len() / 10).copied().unwrap_or(0.0);
    let threshold = (noise * NOISE_MULTIPLIER).max(MIN_RMS);
    Voicing {
        frame,
        voiced: energies.iter().map(|&e| e > threshold).collect(),
        probabilities: None,
    }
}

/// Trims leading/trailing silence. Returns `None` if the clip has no speech.
///
/// With a speech detector, it decides whether there's speech, and the
/// trim keeps whatever either it or loudness takes for speech: the
/// detector hears quiet words that loudness misses, and loudness the
/// breathy start of a whispered word the detector scores low.
pub fn trim_silence(clip: &AudioClip) -> Option<AudioClip> {
    trim_silence_at(clip).map(|(clip, _)| clip)
}

/// `trim_silence`, and how many samples were cut from the start.
pub fn trim_silence_at(clip: &AudioClip) -> Option<(AudioClip, usize)> {
    let loud = speech_span(&by_loudness(clip), clip);
    let (start, end) = match by_detector(clip) {
        Some(detected) => {
            let (start, end) = speech_span(&detected, clip)?;
            loud.map_or((start, end), |(s, e)| (start.min(s), end.max(e)))
        }
        None => loud?,
    };
    Some((
        AudioClip::new(clip.samples[start..end].to_vec(), clip.sample_rate),
        start,
    ))
}

/// Where speech starts and ends in `clip`, with padding, by `voicing`;
/// `None` with too little speech.
fn speech_span(voicing: &Voicing, clip: &AudioClip) -> Option<(usize, usize)> {
    let voiced: Vec<usize> = voicing
        .voiced
        .iter()
        .enumerate()
        .filter_map(|(i, &v)| v.then_some(i))
        .collect();
    if voiced.len() < voicing.frames(MIN_SPEECH_MS, clip.sample_rate) {
        return None;
    }
    let (mut first, mut last) = (voiced[0], voiced[voiced.len() - 1]);
    // A soft start or end scores low but isn't silence: keep it.
    if let Some(p) = &voicing.probabilities {
        while first > 0 && p[first - 1] >= SPEECH_EDGE {
            first -= 1;
        }
        while last + 1 < p.len() && p[last + 1] >= SPEECH_EDGE {
            last += 1;
        }
    }
    let pad = voicing.frames(PAD_MS, clip.sample_rate);
    let start = first.saturating_sub(pad) * voicing.frame;
    // The last frame may be partial.
    let end = ((last + pad + 1) * voicing.frame).min(clip.samples.len());
    Some((start, end))
}

/// The latest pause of at least `min_pause_ms` that starts at or after
/// sample `from` and ends before the last `guard_ms` of the clip. Returns
/// the sample index in the middle of the pause, a safe place to split
/// speech. `None` if there's no such pause (or no speech before it): wait
/// for more audio.
pub fn find_pause(
    clip: &AudioClip,
    from: usize,
    min_pause_ms: u32,
    guard_ms: u32,
) -> Option<usize> {
    pause(clip, from, min_pause_ms, guard_ms, false)
}

/// `find_pause` for audio that must be cut now (it has reached its
/// longest): the speech detector bridges short gaps, so in fast,
/// continuous speech there may be no pause that long; then the longest
/// gap between words (at least `MIN_GAP_MS`) is used, which is still
/// better than cutting through a word.
pub fn find_cut(clip: &AudioClip, from: usize, min_pause_ms: u32) -> Option<usize> {
    pause(clip, from, min_pause_ms, 0, true)
}

fn pause(
    clip: &AudioClip,
    from: usize,
    min_pause_ms: u32,
    guard_ms: u32,
    any_gap: bool,
) -> Option<usize> {
    let voicing = voicing(clip);
    // Only split after some speech, or there's nothing to transcribe.
    if !voicing.voiced.contains(&true) {
        return None;
    }
    let frame = voicing.frame;
    let min_frames = voicing.frames(min_pause_ms, clip.sample_rate);
    let first = from.div_ceil(frame);
    let last = voicing
        .voiced
        .len()
        .saturating_sub((guard_ms as usize * clip.sample_rate as usize / 1000).div_ceil(frame));
    let mut best = None;
    let mut run_start = None;
    for (i, &voiced) in voicing.voiced.iter().enumerate().take(last).skip(first) {
        if !voiced {
            let start = *run_start.get_or_insert(i);
            if i + 1 - start >= min_frames {
                best = Some((start + i).div_ceil(2) * frame);
            }
        } else {
            run_start = None;
        }
    }
    if best.is_none()
        && any_gap
        && let Some(probabilities) = &voicing.probabilities
    {
        best = longest_gap(
            probabilities,
            first,
            last,
            voicing.frames(MIN_GAP_MS, clip.sample_rate),
        )
        .map(|i| i * frame);
    }
    best.map(|i: usize| i.min(clip.samples.len()))
}

/// The shortest gap between words worth cutting at.
const MIN_GAP_MS: u32 = 60;

/// The middle frame of the longest run of frames in `first..last` below
/// the speech threshold, of at least `min` frames (the latest, of equals).
fn longest_gap(probabilities: &[f32], first: usize, last: usize, min: usize) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    let mut run_start = None;
    for (i, &p) in probabilities.iter().enumerate().take(last).skip(first) {
        if p >= SPEECH_STARTS {
            run_start = None;
            continue;
        }
        let start = *run_start.get_or_insert(i);
        let len = i + 1 - start;
        if len >= min && best.is_none_or(|(_, longest)| len >= longest) {
            best = Some(((start + i).div_ceil(2), len));
        }
    }
    best.map(|(middle, _)| middle)
}

/// One frame for the fallback speech detector: 16 ms at 16 kHz.
const VOICE_FRAME: usize = 256;
/// Frames scoring at least this are speech.
const VOICE_SCORE: f32 = 0.5;
/// A clip needs this many speech frames (about 0.1 s) to count.
const MIN_VOICE_FRAMES: usize = 6;

/// Whether a 16 kHz clip contains a voice, judged by the speech detector,
/// or without one by a small neural detector (`earshot`), rather than
/// loudness. A fan, a click or a door that's louder than the room passes
/// a loudness check, and speech models make up text for those ("Thank
/// you.").
pub fn has_voice(clip: &AudioClip) -> bool {
    debug_assert_eq!(clip.sample_rate, crate::TARGET_SAMPLE_RATE);
    // Without the detector (or if it failed on this clip), the small
    // fallback detector below, never loudness.
    if let Some(voicing) = by_detector(clip) {
        let needed = voicing.frames(MIN_SPEECH_MS, clip.sample_rate);
        return voicing.voiced.iter().filter(|&&v| v).count() >= needed;
    }
    let mut detector = earshot::Detector::default();
    clip.samples
        .as_chunks::<VOICE_FRAME>()
        .0
        .iter()
        .filter(|frame| detector.predict_f32(frame.as_slice()) >= VOICE_SCORE)
        .take(MIN_VOICE_FRAMES)
        .count()
        == MIN_VOICE_FRAMES
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
    fn noise_and_tones_have_no_voice() {
        // White noise and a steady hum are louder than the room, but not a
        // voice.
        let mut seed = 1u32;
        let noise: Vec<f32> = (0..32_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 * 0.6 - 0.3
            })
            .collect();
        assert!(!has_voice(&AudioClip::new(noise, 16_000)));
        let hum: Vec<f32> = (0..32_000)
            .map(|i| (i as f32 * std::f32::consts::TAU * 120.0 / 16_000.0).sin() * 0.3)
            .collect();
        assert!(!has_voice(&AudioClip::new(hum, 16_000)));
        assert!(!has_voice(&AudioClip::new(vec![0.0; 100], 16_000)));
    }

    #[test]
    fn the_longest_gap_between_words_when_theres_no_pause() {
        // Speech, a two-frame gap, speech, a four-frame gap, speech, a
        // three-frame gap.
        let p = [
            0.9, 0.9, 0.2, 0.3, 0.9, 0.9, 0.1, 0.1, 0.2, 0.1, 0.9, 0.4, 0.4, 0.4, 0.9,
        ];
        assert_eq!(longest_gap(&p, 0, p.len(), 2), Some(8));
        // Only part of the clip may be searched.
        assert_eq!(longest_gap(&p, 0, 6, 2), Some(3));
        assert_eq!(longest_gap(&p, 0, p.len(), 5), None);
        assert_eq!(longest_gap(&[0.9; 8], 0, 8, 1), None);
    }

    #[test]
    fn click_is_not_speech() {
        let mut samples = vec![0.0f32; 32_000];
        samples[16_000..16_100].fill(0.9);
        assert!(trim_silence(&AudioClip::new(samples, 16_000)).is_none());
    }
}
