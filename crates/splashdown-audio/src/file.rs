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

// Cutting by turns (one side of a meeting): a piece per stretch of speech.
/// A pause this long ends a turn...
const TURN_PAUSE_MS: u32 = 700;
/// ...once there's been this much speech.
const MIN_TURN_MS: u32 = 300;
/// A turn longer than this is cut at a short pause (or here).
const MAX_TURN: Duration = Duration::from_secs(30);
/// Silence kept around speech so words aren't clipped.
const PAD_MS: usize = 200;
/// With no speech, silence is handed on in steps this long, so the other
/// side's turns aren't held up waiting for this one.
const SILENCE_STEP: Duration = Duration::from_secs(5);

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
    /// One channel only (a meeting recording: You left, Them right), or
    /// all of them mixed.
    channel: Option<usize>,
}

impl AudioFile {
    pub fn open(path: &Path) -> Result<Self, FileError> {
        Self::open_channel(path, None)
    }

    /// Like `open`, reading only `channel` (0 is the first) when it's set.
    /// A file with fewer channels reads as silence on the missing ones.
    pub fn open_channel(path: &Path, channel: Option<usize>) -> Result<Self, FileError> {
        Self::opened(path, channel, false)
    }

    /// One side of a meeting recording, in pieces by turn: each stretch of
    /// speech is a piece that starts where the speech does, and silence
    /// comes as pieces with nothing said.
    pub fn open_turns(path: &Path, channel: usize) -> Result<Self, FileError> {
        Self::opened(path, Some(channel), true)
    }

    fn opened(path: &Path, channel: Option<usize>, turns: bool) -> Result<Self, FileError> {
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
        // The length is counted in the track's time base (milliseconds in
        // MKV, the media's own timescale in MP4), or else in samples.
        let duration = params.n_frames.and_then(|n| match params.time_base {
            Some(base) => {
                let t = base.calc_time(n);
                Some(Duration::from_secs_f64(t.seconds as f64 + t.frac))
            }
            None => (rate > 0).then(|| Duration::from_secs_f64(n as f64 / f64::from(rate))),
        });
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
            cutter: Cutter {
                turns,
                ..Cutter::default()
            },
            ended: false,
            channel,
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
        while self.block.len() < self.rate as usize * BLOCK_SECS {
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
                    // The audio's real rate can differ from the header's
                    // (HE-AAC) or change midway: what came before is
                    // resampled at the rate it had.
                    if spec.rate != self.rate && spec.rate > 0 {
                        let before = std::mem::take(&mut self.block);
                        self.cutter.push(&resample::resample(
                            &before,
                            self.rate,
                            TARGET_SAMPLE_RATE,
                        ));
                        self.rate = spec.rate;
                    }
                    let channels = spec.channels.count().max(1);
                    let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                    samples.copy_interleaved_ref(decoded);
                    self.block
                        .extend(samples.samples().chunks(channels).map(
                            |frame| match self.channel {
                                Some(c) => frame.get(c).copied().unwrap_or(0.0),
                                None => frame.iter().sum::<f32>() / channels as f32,
                            },
                        ));
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

/// Cuts one side of a meeting into turns as its audio arrives (16 kHz
/// mono): the live counterpart of `AudioFile::open_turns`.
#[derive(Default)]
pub struct TurnCutter(Cutter);

impl TurnCutter {
    pub fn new() -> Self {
        Self(Cutter {
            turns: true,
            ..Cutter::default()
        })
    }

    pub fn push(&mut self, samples: &[f32]) {
        self.0.push(samples);
    }

    /// The next finished turn (or stretch of silence); with `ended`, what's
    /// left.
    pub fn next(&mut self, ended: bool) -> Option<Piece> {
        self.0.next(ended)
    }
}

/// Cuts 16 kHz audio into pieces at pauses.
#[derive(Default)]
struct Cutter {
    /// Audio not yet in a piece.
    buffer: Vec<f32>,
    /// Samples already handed out.
    taken: usize,
    /// A piece per turn instead of about 30 s at a time.
    turns: bool,
}

impl Cutter {
    fn push(&mut self, samples: &[f32]) {
        self.buffer.extend_from_slice(samples);
    }

    /// The next piece, if there's enough audio for one (or `ended`, any
    /// audio at all).
    fn next(&mut self, ended: bool) -> Option<Piece> {
        if self.turns {
            return self.next_turn(ended);
        }
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
        Some(self.take(0, cut))
    }

    /// The next turn: from where speech starts to the first pause after it
    /// (at most `MAX_TURN`), or a stretch of silence.
    fn next_turn(&mut self, ended: bool) -> Option<Piece> {
        if self.buffer.is_empty() {
            return None;
        }
        let rate = TARGET_SAMPLE_RATE as usize;
        let voicing = vad::voicing(&AudioClip::new(self.buffer.clone(), TARGET_SAMPLE_RATE));
        let frame = voicing.frame;
        let frames = |ms: usize| (rate * ms / 1000).div_ceil(frame);
        let pad = frames(PAD_MS);
        let Some(first) = voicing.voiced.iter().position(|&v| v) else {
            // Nothing said yet. The newest audio could be the start of a
            // word, so a little is kept back.
            let step = SILENCE_STEP.as_secs() as usize * rate;
            if !ended && self.buffer.len() < step {
                return None;
            }
            let keep = if ended { 0 } else { pad * frame };
            return Some(self.take(0, self.buffer.len().saturating_sub(keep)));
        };
        let lead = first.saturating_sub(pad) * frame;
        let mut quiet = 0;
        let mut pause = None;
        let skip = first + frames(MIN_TURN_MS as usize);
        for (i, &voiced) in voicing.voiced.iter().enumerate().skip(skip) {
            if voiced {
                quiet = 0;
                continue;
            }
            quiet += 1;
            if quiet >= frames(TURN_PAUSE_MS as usize) {
                pause = Some((i + 1 - quiet + pad) * frame);
                break;
            }
        }
        let max = lead + MAX_TURN.as_secs() as usize * rate;
        let cut = match pause {
            Some(cut) if cut <= max => cut,
            // A long turn: cut at its latest short pause, or at the limit.
            _ if self.buffer.len() >= max => {
                let window = AudioClip::new(self.buffer[lead..max].to_vec(), TARGET_SAMPLE_RATE);
                let from = MIN_PIECE.as_secs() as usize * rate / 2;
                lead + vad::find_pause(&window, from, MIN_PAUSE_MS, 0)
                    .filter(|&cut| cut > 0)
                    .unwrap_or(max - lead)
            }
            _ if ended => self.buffer.len(),
            _ => return None,
        };
        Some(self.take(lead, cut.min(self.buffer.len())))
    }

    /// Hands out the buffer up to `to`, the piece starting at `from` (what
    /// comes before it is silence, skipped).
    fn take(&mut self, from: usize, to: usize) -> Piece {
        let rate = TARGET_SAMPLE_RATE as f64;
        let samples: Vec<f32> = self.buffer.drain(..to).skip(from).collect();
        let start = Duration::from_secs_f64((self.taken + from) as f64 / rate);
        self.taken += to;
        Piece {
            start,
            clip: AudioClip::new(samples, TARGET_SAMPLE_RATE),
        }
    }
}

/// A time in the file as the transcript shows it: `12:30`, or `1:02:30`
/// past an hour. Rounded, since pieces start a moment before the speech.
pub fn timestamp(at: Duration) -> String {
    let secs = (at.as_millis() as u64 + 500) / 1000;
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

    fn turns(samples: &[f32]) -> Vec<Piece> {
        let mut cutter = Cutter {
            turns: true,
            ..Cutter::default()
        };
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

    #[test]
    fn a_meeting_side_is_cut_into_turns() {
        // Silence, a turn, the other side's turn (silence here), a turn.
        let samples = audio(&[
            (2.0, false),
            (3.0, true),
            (8.0, false),
            (4.0, true),
            (1.0, false),
        ]);
        let pieces = turns(&samples);
        let spoken: Vec<&Piece> = pieces
            .iter()
            .filter(|p| p.clip.samples.iter().any(|s| s.abs() > 0.1))
            .collect();
        assert_eq!(spoken.len(), 2, "{pieces:?}");
        // Each starts where the speech does, less 0.2 s of padding (the
        // test's speech opens with a 0.1 s gap, so 2.1 s and 13.1 s).
        assert!(
            (spoken[0].start.as_secs_f32() - 1.9).abs() < 0.05,
            "{:?}",
            spoken[0].start
        );
        assert!(
            (spoken[1].start.as_secs_f32() - 12.9).abs() < 0.05,
            "{:?}",
            spoken[1].start
        );
        // Everything is accounted for, silence included, in order.
        let end = pieces.last().map(|p| p.start + p.clip.duration()).unwrap();
        assert!((end.as_secs_f32() - 18.0).abs() < 0.05, "{end:?}");
        assert!(
            pieces
                .windows(2)
                .all(|w| w[0].start + w[0].clip.duration() <= w[1].start)
        );
    }

    #[test]
    fn a_silent_side_keeps_moving() {
        let pieces = turns(&audio(&[(23.0, false)]));
        assert!(pieces.len() >= 2, "{}", pieces.len());
        assert!(
            pieces
                .iter()
                .all(|p| p.clip.samples.iter().all(|s| s.abs() < 0.01))
        );
    }

    #[test]
    fn a_long_turn_is_capped() {
        let pieces = turns(&audio(&[(70.0, true)]));
        assert!(pieces.len() >= 3, "{}", pieces.len());
        assert!(pieces.iter().all(|p| p.clip.duration() <= MAX_TURN));
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp(Duration::from_secs(0)), "0:00");
        assert_eq!(timestamp(Duration::from_secs(750)), "12:30");
        assert_eq!(timestamp(Duration::from_secs(3750)), "1:02:30");
        assert_eq!(timestamp(Duration::from_millis(11_800)), "0:12");
    }
}
