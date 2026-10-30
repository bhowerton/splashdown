//! Interactive microphone recording for `splashdown-cli record`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use splashdown_audio::{Level, Recorder};
use splashdown_core::AudioClip;

/// Records until Enter. With `instruct`, the first Enter splits content from
/// instruction. Returns the prepared (16 kHz, trimmed) clips.
pub fn record(
    device: Option<&str>,
    instruct: bool,
    save: Option<&Path>,
) -> Result<(AudioClip, Option<AudioClip>)> {
    let recorder = Recorder::start(device)?;
    let meter = Meter::start(recorder.level());

    let (content, instruction) = if instruct {
        meter.label("● Speak the content — Enter switches to the instruction");
        wait_for_enter()?;
        let content = recorder.take();
        meter.label("◆ Speak the instruction — Enter to finish");
        wait_for_enter()?;
        (content, Some(recorder.stop()))
    } else {
        meter.label("● Recording — press Enter to stop");
        wait_for_enter()?;
        (recorder.stop(), None)
    };
    meter.stop();

    if let Some(path) = save {
        std::fs::write(path, splashdown_audio::wav::encode(&content))?;
        if let Some(instr) = &instruction {
            std::fs::write(instruction_path(path), splashdown_audio::wav::encode(instr))?;
        }
    }

    if content.samples.iter().all(|s| *s == 0.0) && !content.is_empty() {
        bail!(
            "the microphone returned pure silence — macOS may be blocking it. \
             Allow your terminal app in System Settings → Privacy & Security → Microphone."
        );
    }
    let Some(content) = splashdown_audio::prepare(&content) else {
        bail!("didn't catch that — no speech found");
    };
    let instruction = instruction.as_ref().and_then(splashdown_audio::prepare);
    Ok((content, instruction))
}

fn instruction_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("recording");
    path.with_file_name(format!("{stem}-instruction.wav"))
}

fn wait_for_enter() -> Result<()> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(())
}

/// A one-line live level meter on stderr.
struct Meter {
    stop: Arc<AtomicBool>,
    label: Arc<std::sync::Mutex<String>>,
    handle: std::thread::JoinHandle<()>,
}

impl Meter {
    fn start(level: Level) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let label = Arc::new(std::sync::Mutex::new(String::new()));
        let handle = {
            let (stop, label) = (stop.clone(), label.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    // Speech RMS is typically 0.02–0.2; scale so it fills the bar.
                    let bars = ((level.get() * 120.0).sqrt() * 6.0).min(20.0) as usize;
                    let text = label.lock().map(|l| l.clone()).unwrap_or_default();
                    eprint!("\r\x1b[2K{text}  {:<20}", "▮".repeat(bars));
                    let _ = std::io::stderr().flush();
                    std::thread::sleep(Duration::from_millis(60));
                }
                eprint!("\r\x1b[2K");
            })
        };
        Self {
            stop,
            label,
            handle,
        }
    }

    fn label(&self, text: &str) {
        if let Ok(mut l) = self.label.lock() {
            *l = text.to_string();
        }
    }

    fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_file_name() {
        assert_eq!(
            instruction_path(Path::new("/tmp/take1.wav")),
            Path::new("/tmp/take1-instruction.wav")
        );
    }
}
