//! `splashdown-cli meeting record` and `meeting transcribe`: recording
//! You and Them on separate channels, and turning a recording into a
//! transcript, as the app does.

use super::*;

/// What the computer plays, as a live recording.
pub(super) fn system_audio() -> Result<splashdown_audio::live::LiveAudio> {
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::system_audio().map_err(anyhow::Error::msg);
    #[cfg(windows)]
    return Ok(splashdown_audio::live::LiveAudio::system_output()?);
    #[cfg(not(any(target_os = "macos", windows)))]
    anyhow::bail!("recording the computer's sound isn't supported here");
}

/// Records both sides of a meeting into a stereo WAV.
pub(super) fn record_meeting(
    out: &std::path::Path,
    seconds: u64,
    device: Option<&str>,
) -> Result<()> {
    use splashdown_audio::live::LiveAudio;

    let mut you = LiveAudio::microphone(device)?;
    let mut them = system_audio()?;
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let end = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut loudest = (0.0f32, 0.0f32);
    while std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(200));
        left.extend(you.take());
        right.extend(them.take());
        loudest = (loudest.0.max(you.level()), loudest.1.max(them.level()));
        eprint!(
            "\rrecording {:>3} s · You {:.3} · Them {:.3}   ",
            left.len() / 16_000,
            you.level(),
            them.level()
        );
    }
    eprintln!();
    eprintln!(
        "audio delivered: You {:.1} s, Them {:.1} s",
        left.len() as f64 / 16_000.0,
        right.len() as f64 / 16_000.0
    );
    let frames = left.len().max(right.len());
    left.resize(frames, 0.0);
    right.resize(frames, 0.0);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: splashdown_audio::TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(out, spec)?;
    for (l, r) in left.iter().zip(&right) {
        for s in [l, r] {
            writer.write_sample((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)?;
        }
    }
    writer.finalize()?;
    eprintln!(
        "wrote {:.1} s to {} (loudest: You {:.3}, Them {:.3}{})",
        frames as f64 / 16_000.0,
        out.display(),
        loudest.0,
        loudest.1,
        if loudest.1 == 0.0 {
            "; Them was silent: nothing played, or recording the computer's sound isn't allowed"
        } else {
            ""
        }
    );
    Ok(())
}

/// Transcribes both sides of a meeting recording a piece at a time, oldest
/// first, and prints each paragraph once it's final.
pub(super) fn transcribe_meeting(
    rt: &tokio::runtime::Runtime,
    path: &std::path::Path,
    stt: &dyn splashdown_core::SpeechToText,
    hints: &splashdown_core::SttHints,
) -> Result<()> {
    use splashdown_audio::file::{AudioFile, Piece, timestamp};
    use splashdown_core::meeting::{Meeting, Paragraph, Segment, Side};

    let open = |channel| {
        AudioFile::open_turns(path, channel).with_context(|| format!("reading {}", path.display()))
    };
    let mut sides = [(Side::You, open(0)?), (Side::Them, open(1)?)];
    let mut next: [Option<Piece>; 2] = [sides[0].1.next_piece()?, sides[1].1.next_piece()?];
    let print = |paragraphs: Vec<Paragraph>| {
        for p in paragraphs {
            println!(
                "\n**[{}] {}:** {}",
                timestamp(p.start),
                p.side.label(),
                p.text
            );
        }
    };
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    println!("# {stem}");
    let start = std::time::Instant::now();
    let mut meeting = Meeting::new();
    // The side whose next piece starts first goes next, so the transcript
    // moves forward evenly.
    while let Some(i) = (0..2)
        .filter(|&i| next[i].is_some())
        .min_by_key(|&i| next[i].as_ref().map(|p| p.start))
    {
        let piece = next[i].take().expect("chosen because it's there");
        let end = piece.start + piece.clip.duration();
        let text = match splashdown_audio::prepare(&piece.clip) {
            Some(clip) => rt.block_on(stt.transcribe(&clip, hints))?.text,
            None => String::new(),
        };
        meeting.push(Segment {
            side: sides[i].0,
            start: piece.start,
            end,
            text,
        });
        next[i] = sides[i].1.next_piece()?;
        if next[i].is_none() {
            // This side is over: nothing more will come from it.
            meeting.reached(sides[i].0, std::time::Duration::MAX);
        }
        print(meeting.ready());
    }
    let echoes = meeting.echoes;
    print(meeting.finish());
    eprintln!(
        "transcribed in {:.1} s with {}; {echoes} piece(s) dropped as echo",
        start.elapsed().as_secs_f64(),
        stt.name()
    );
    Ok(())
}
