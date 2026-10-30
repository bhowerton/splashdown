//! WAV encoding for providers and decoding for the CLI and eval harness.

use std::io::Cursor;
use std::path::Path;

use splashdown_core::AudioClip;

#[derive(Debug, thiserror::Error)]
pub enum WavError {
    #[error("couldn't read WAV: {0}")]
    Hound(#[from] hound::Error),
    #[error("unsupported WAV format: {0}")]
    Unsupported(String),
}

/// Encodes the clip as 16-bit PCM mono WAV.
pub fn encode(clip: &AudioClip) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: clip.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Cursor::new(Vec::with_capacity(44 + clip.samples.len() * 2));
    {
        // Writing to an in-memory buffer can't fail.
        let mut writer = hound::WavWriter::new(&mut buf, spec).expect("wav header");
        for &s in &clip.samples {
            let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            writer.write_sample(v).expect("wav sample");
        }
        writer.finalize().expect("wav finalize");
    }
    buf.into_inner()
}

/// Reads a WAV file (PCM 8/16/24/32-bit or float), downmixed to mono.
pub fn read(path: &Path) -> Result<AudioClip, WavError> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.into_samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = match spec.bits_per_sample {
                8 => f32::from(i8::MAX),
                16 => f32::from(i16::MAX),
                24 => 8_388_607.0,
                32 => i32::MAX as f32,
                b => return Err(WavError::Unsupported(format!("{b}-bit integer"))),
            };
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };

    let samples = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok(AudioClip::new(samples, spec.sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_then_read_round_trips() {
        let clip = AudioClip::new(vec![0.0, 0.5, -0.5, 1.0, -1.0], 16_000);
        let bytes = encode(&clip);
        assert_eq!(&bytes[0..4], b"RIFF");

        let dir = std::env::temp_dir().join(format!("splashdown-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        std::fs::write(&path, &bytes).unwrap();
        let back = read(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(back.sample_rate, 16_000);
        for (a, b) in clip.samples.iter().zip(&back.samples) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }
}
