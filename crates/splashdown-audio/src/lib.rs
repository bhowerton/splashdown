//! Microphone capture (cpal), resampling to 16 kHz mono, noise reduction,
//! silence trimming, WAV encoding, and reading audio and video files for
//! transcription.

mod capture;
pub mod denoise;
pub mod devices;
pub mod file;
pub mod gain;
pub mod live;
pub mod playback;
pub mod resample;
pub mod vad;
pub mod wav;

pub use capture::{CaptureError, Level, Recorder, input_devices};

use splashdown_core::AudioClip;

/// Sample rate every provider receives.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Prepares a recording for speech-to-text: resample to 16 kHz and trim
/// silence. Returns `None` if there's no speech in it: nothing louder than
/// the room, or nothing the speech detector takes for a voice.
pub fn prepare(clip: &AudioClip) -> Option<AudioClip> {
    let resampled = AudioClip::new(
        resample::resample(&clip.samples, clip.sample_rate, TARGET_SAMPLE_RATE),
        TARGET_SAMPLE_RATE,
    );
    vad::trim_silence(&resampled).filter(vad::has_voice)
}

/// Optional clean-up before speech-to-text, from the user's settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cleanup {
    /// Remove steady background noise (`denoise`).
    pub denoise: bool,
    /// Whisper mode: raise quiet speech to a normal level, so it isn't
    /// taken for silence and the speech model hears it
    /// (`gain::boost_quiet`).
    pub quiet: bool,
}

impl Cleanup {
    /// Applies the chosen steps, noise first: removing it before the boost
    /// means the boost measures the voice, not the room, and doesn't raise
    /// the noise along with it.
    pub fn apply(self, clip: &AudioClip) -> AudioClip {
        let clip = if self.denoise {
            denoise::denoise(clip)
        } else {
            clip.clone()
        };
        if self.quiet {
            gain::boost_quiet(&clip)
        } else {
            clip
        }
    }
}

/// `prepare_with`, and where in `clip` the prepared audio starts (the
/// silence trimmed from its start), for placing words in time.
pub fn prepare_at(clip: &AudioClip, cleanup: Cleanup) -> Option<(AudioClip, std::time::Duration)> {
    let cleaned = if cleanup == Cleanup::default() {
        clip.clone()
    } else {
        cleanup.apply(clip)
    };
    let resampled = AudioClip::new(
        resample::resample(&cleaned.samples, cleaned.sample_rate, TARGET_SAMPLE_RATE),
        TARGET_SAMPLE_RATE,
    );
    let (trimmed, start) = vad::trim_silence_at(&resampled)?;
    vad::has_voice(&trimmed).then(|| {
        let offset = std::time::Duration::from_secs_f64(start as f64 / TARGET_SAMPLE_RATE as f64);
        (trimmed, offset)
    })
}

/// `prepare` after the `cleanup` steps.
pub fn prepare_with(clip: &AudioClip, cleanup: Cleanup) -> Option<AudioClip> {
    if cleanup == Cleanup::default() {
        return prepare(clip);
    }
    prepare(&cleanup.apply(clip))
}
