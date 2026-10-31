//! Audio and video files for transcription: decoded with
//! Symphonia, resampled to 16 kHz mono a block at a time, and cut into
//! pieces of about 30 s at pauses, so a long file is never in memory whole.

use std::fs::File;
use std::path::Path;
use std::time::Duration;

use splashdown_core::AudioClip;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{TARGET_SAMPLE_RATE, resample, vad};

/// Pieces are cut at a pause after this much audio...
const MIN_PIECE: Duration = Duration::from_secs(20);
/// ...and at this length if there's no pause before it.
const MAX_PIECE: Duration = Duration::from_secs(40);
/// A pause this long is a safe place to cut.
const MIN_PAUSE_MS: u32 = 300;
/// Source audio resampled at a time, in seconds.
const BLOCK_SECS: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("couldn't open the file: {0}")]
    Open(#[from] std::io::Error),
    #[error("Splashdown can't read this kind of file yet ({0}). Try converting it to MP3 or M4A.")]
    Unsupported(String),
    #[error("the file has no sound in it")]
    NoAudio,
    #[error("couldn't read the file: {0}")]
    Decode(String),
}

/// A stretch of the file, ready for speech-to-text.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    /// Where it starts in the file.
    pub start: Duration,
    /// 16 kHz mono.
    pub clip: AudioClip,
}

/// An open file, read piece by piece with `next_piece`.
pub struct AudioFile {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    rate: u32,
    duration: Option<Duration>,
    /// Mono audio at the file's rate, waiting to be resampled.
    block: Vec<f32>,
    cutter: Cutter,
    ended: bool,
}

impl AudioFile {
    pub fn open(path: &Path) -> Result<Self, FileError> {
        let source = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(extension);
        }
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                source,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| match e {
                Error::IoError(e) => FileError::Open(e),
                _ => FileError::Unsupported(kind(path)),
            })?;
        let format = probed.format;
        // A video's first track is usually the picture: take the first
        // audio track.
        let track = format
            .tracks()
            .iter()
            .find(|t| {
                t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some()
            })
            .ok_or(FileError::NoAudio)?;
        let params = &track.codec_params;
        let rate = params.sample_rate.unwrap_or(TARGET_SAMPLE_RATE);
        let duration = params
            .n_frames
            .filter(|_| rate > 0)
            .map(|n| Duration::from_secs_f64(n as f64 / f64::from(rate)));
        let decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions::default())
            .map_err(|_| FileError::Unsupported(kind(path)))?;
        Ok(Self {
            track: track.id,
            format,
            decoder,
            rate,
            duration,
            block: Vec::new(),
            cutter: Cutter::default(),
            ended: false,
        })
    }

    /// How long the file is, when its header says.
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// The next piece, or `None` at the end of the file.
    pub fn next_piece(&mut self) -> Result<Option<Piece>, FileError> {
        loop {
            if let Some(piece) = self.cutter.next(self.ended) {
                return Ok(Some(piece));
            }
            if self.ended {
                return Ok(None);
            }
            self.decode_block()?;
        }
    }

    /// Decodes about `BLOCK_SECS` of audio and hands it to the cutter at
    /// 16 kHz.
    fn decode_block(&mut self) -> Result<(), FileError> {
        let want = self.rate as usize * BLOCK_SECS;
        while self.block.len() < want {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                // The usual way a file ends.
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.ended = true;
                    break;
                }
                // A chained stream (a new OGG section): what came before
                // is the recording.
                Err(Error::ResetRequired) => {
                    self.ended = true;
                    break;
                }
                Err(e) => return Err(FileError::Decode(e.to_string())),
            };
            if packet.track_id() != self.track {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let channels = spec.channels.count().max(1);
                    let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                    samples.copy_interleaved_ref(decoded);
                    self.block.extend(
                        samples
                            .samples()
                            .chunks(channels)
                            .map(|frame| frame.iter().sum::<f32>() / channels as f32),
                    );
                }
                // A damaged packet: skip it, as players do.
                Err(Error::DecodeError(e)) => tracing::debug!("skipped a damaged packet: {e}"),
                Err(e) => return Err(FileError::Decode(e.to_string())),
            }
        }
        let block = std::mem::take(&mut self.block);
        self.cutter
            .push(&resample::resample(&block, self.rate, TARGET_SAMPLE_RATE));
        Ok(())
    }
}

/// "M4A file", for messages.
fn kind(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map_or("this file".to_string(), |e| {
            format!("{} file", e.to_uppercase())
        })
}

/// Cuts 16 kHz audio into pieces at pauses.
#[derive(Default)]
struct Cutter {
    /// Audio not yet in a piece.
    buffer: Vec<f32>,
    /// Samples already handed out.
    taken: usize,
}

impl Cutter {
    fn push(&mut self, samples: &[f32]) {
        self.buffer.extend_from_slice(samples);
    }

    /// The next piece, if there's enough audio for one (or `ended`, any
    /// audio at all).
    fn next(&mut self, ended: bool) -> Option<Piece> {
        let rate = TARGET_SAMPLE_RATE as usize;
        let max = MAX_PIECE.as_secs() as usize * rate;
        if self.buffer.is_empty() || (!ended && self.buffer.len() < max) {
            return None;
        }
        let cut = if ended && self.buffer.len() <= max {
            self.buffer.len()
        } else {
            let window = AudioClip::new(self.buffer[..max].to_vec(), TARGET_SAMPLE_RATE);
            let from = MIN_PIECE.as_secs() as usize * rate;
            vad::find_pause(&window, from, MIN_PAUSE_MS, 0)
                .filter(|&cut| cut > 0)
                .unwrap_or(max)
        };
        let samples: Vec<f32> = self.buffer.drain(..cut).collect();
        let start = Duration::from_secs_f64(self.taken as f64 / rate as f64);
        self.taken += cut;
        Some(Piece {
            start,
            clip: AudioClip::new(samples, TARGET_SAMPLE_RATE),
        })
    }
}

/// A time in the file as the transcript shows it: `12:30`, or `1:02:30`
/// past an hour.
pub fn timestamp(at: Duration) -> String {
    let secs = at.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: usize = TARGET_SAMPLE_RATE as usize;

    /// `pattern` is (seconds, speaking) stretches. Speech has a 100 ms gap
    /// every 300 ms, like the breaks between syllables, too short to cut at.
    fn audio(pattern: &[(f32, bool)]) -> Vec<f32> {
        pattern
            .iter()
            .flat_map(|&(secs, speaking)| {
                (0..(secs * RATE as f32) as usize).map(move |i| {
                    if speaking && i % (RATE * 3 / 10) >= RATE / 10 {
                        (i as f32 * 0.3).sin() * 0.3
                    } else {
                        0.001
                    }
                })
            })
            .collect()
    }

    fn pieces(samples: &[f32]) -> Vec<Piece> {
        let mut cutter = Cutter::default();
        let mut out = Vec::new();
        for block in samples.chunks(RATE * BLOCK_SECS) {
            cutter.push(block);
            while let Some(piece) = cutter.next(false) {
                out.push(piece);
            }
        }
        while let Some(piece) = cutter.next(true) {
            out.push(piece);
        }
        out
    }

    fn secs(piece: &Piece) -> f32 {
        piece.clip.samples.len() as f32 / RATE as f32
    }

    #[test]
    fn cuts_at_a_pause_between_20_and_40_seconds() {
        let samples = audio(&[(25.0, true), (1.0, false), (30.0, true)]);
        let pieces = pieces(&samples);
        assert_eq!(pieces.len(), 2);
        // In the middle of the pause.
        assert!(
            (secs(&pieces[0]) - 25.5).abs() < 0.1,
            "{}",
            secs(&pieces[0])
        );
        assert_eq!(pieces[1].start, Duration::from_secs_f32(secs(&pieces[0])));
        let total: usize = pieces.iter().map(|p| p.clip.samples.len()).sum();
        assert_eq!(total, samples.len());
    }

    #[test]
    fn a_pause_too_early_is_ignored_and_long_speech_is_capped() {
        let samples = audio(&[(5.0, true), (1.0, false), (90.0, true)]);
        let pieces = pieces(&samples);
        assert_eq!(pieces.len(), 3);
        assert!((secs(&pieces[0]) - 40.0).abs() < 0.01);
        assert!((secs(&pieces[1]) - 40.0).abs() < 0.01);
        assert_eq!(pieces[2].start, Duration::from_secs(80));
    }

    #[test]
    fn a_short_file_is_one_piece() {
        let pieces = pieces(&audio(&[(3.0, true)]));
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].start, Duration::ZERO);
    }

    #[test]
    fn reads_a_wav_file_in_pieces() {
        // 48 kHz stereo-free mono, 50 s: two pieces at 16 kHz.
        let at_48k: Vec<f32> = crate::resample::resample(
            &audio(&[(30.0, true), (1.0, false), (19.0, true)]),
            16_000,
            48_000,
        );
        let dir = std::env::temp_dir().join(format!("splashdown-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("talk.wav");
        std::fs::write(&path, crate::wav::encode(&AudioClip::new(at_48k, 48_000))).unwrap();

        let mut file = AudioFile::open(&path).unwrap();
        let duration = file.duration().unwrap().as_secs_f32();
        assert!((duration - 50.0).abs() < 0.01, "{duration}");
        let mut pieces = Vec::new();
        while let Some(piece) = file.next_piece().unwrap() {
            pieces.push(piece);
        }
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(pieces.len(), 2);
        assert!(
            (secs(&pieces[0]) - 30.5).abs() < 0.1,
            "{}",
            secs(&pieces[0])
        );
        let total: f32 = pieces.iter().map(secs).sum();
        assert!((total - 50.0).abs() < 0.01, "{total}");
    }

    #[test]
    fn other_files_are_refused_kindly() {
        let dir = std::env::temp_dir().join(format!("splashdown-notaudio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notes.txt");
        std::fs::write(&path, "not audio at all, just some words in a text file").unwrap();
        let err = AudioFile::open(&path).err().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            matches!(err, FileError::Unsupported(ref k) if k == "TXT file"),
            "{err}"
        );
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp(Duration::from_secs(0)), "0:00");
        assert_eq!(timestamp(Duration::from_secs(750)), "12:30");
        assert_eq!(timestamp(Duration::from_secs(3750)), "1:02:30");
    }
}
