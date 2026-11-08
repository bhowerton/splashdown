//! Start and stop sounds, and the system's reduce-motion setting.
//!
//! The sounds are Windows' own speech-recognition chimes from
//! `%WINDIR%\Media`, scaled to the chosen volume and played asynchronously
//! from memory with `PlaySound`. If a file is missing, nothing plays.

use splashdown_core::sync::Locked;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FLAGS, SND_MEMORY, SND_NODEFAULT};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::BOOL;
use windows::core::PCWSTR;

use crate::Sound;

/// The sound being played. `PlaySound` reads it asynchronously, so it must
/// stay alive and unchanged until playback is stopped.
static PLAYING: Mutex<Option<Vec<u8>>> = Mutex::new(None);

fn file(sound: Sound) -> Option<&'static [u8]> {
    static START: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    static STOP: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    let (cell, name) = match sound {
        Sound::Start => (&START, "Speech On.wav"),
        Sound::Stop => (&STOP, "Speech Off.wav"),
    };
    cell.get_or_init(|| {
        let dir =
            std::env::var_os("WINDIR").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        std::fs::read(dir.join("Media").join(name)).ok()
    })
    .as_deref()
}

pub(crate) fn play_sound(sound: Sound, volume: f32) {
    let volume = volume.clamp(0.0, 1.0);
    if volume < 0.01 {
        return;
    }
    let Some(wav) = file(sound) else {
        return;
    };
    let mut data = wav.to_vec();
    scale_wav(&mut data, volume);

    let mut playing = PLAYING.locked();
    // SAFETY: a null sound stops whatever this process is playing; once it
    // returns, PlaySound no longer reads the previous buffer, so it can be
    // replaced below.
    unsafe {
        let _ = PlaySoundW(PCWSTR::null(), None, SND_FLAGS(0));
    }
    let buf = playing.insert(data);
    // SAFETY: with SND_MEMORY the "name" is a pointer to a complete WAV
    // file in memory. It lives in `PLAYING` and isn't touched again until
    // the next call has stopped this playback (above).
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(buf.as_ptr().cast()),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

/// Scales 16-bit PCM samples in a WAV file by `volume`. Other encodings
/// are left as they are (played at full volume).
fn scale_wav(wav: &mut [u8], volume: f32) {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return;
    }
    let mut pcm16 = false;
    let mut at = 12;
    while at + 8 <= wav.len() {
        let id = [wav[at], wav[at + 1], wav[at + 2], wav[at + 3]];
        let size =
            u32::from_le_bytes([wav[at + 4], wav[at + 5], wav[at + 6], wav[at + 7]]) as usize;
        let body = at + 8;
        let end = body.saturating_add(size).min(wav.len());
        match &id {
            b"fmt " if end - body >= 16 => {
                let format = u16::from_le_bytes([wav[body], wav[body + 1]]);
                let bits = u16::from_le_bytes([wav[body + 14], wav[body + 15]]);
                pcm16 = format == 1 && bits == 16;
            }
            b"data" if pcm16 => {
                for sample in wav[body..end].as_chunks_mut::<2>().0 {
                    let v = i16::from_le_bytes(*sample);
                    *sample = ((f32::from(v) * volume).round() as i16).to_le_bytes();
                }
                return;
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        at = body.saturating_add(size).saturating_add(size & 1);
    }
}

pub(crate) fn reduce_motion() -> bool {
    let mut animations = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer,
    // which is valid for that.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut animations).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && !animations.as_bool()
}

#[cfg(test)]
mod tests {
    use super::scale_wav;

    #[test]
    fn scales_pcm16_samples() {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF\0\0\0\0WAVE");
        wav.extend_from_slice(b"fmt \x10\0\0\0");
        wav.extend_from_slice(&[1, 0, 1, 0, 0x44, 0xAC, 0, 0, 0x88, 0x58, 1, 0, 2, 0, 16, 0]);
        wav.extend_from_slice(b"data\x04\0\0\0");
        wav.extend_from_slice(&1000i16.to_le_bytes());
        wav.extend_from_slice(&(-1000i16).to_le_bytes());
        scale_wav(&mut wav, 0.5);
        let n = wav.len();
        assert_eq!(i16::from_le_bytes([wav[n - 4], wav[n - 3]]), 500);
        assert_eq!(i16::from_le_bytes([wav[n - 2], wav[n - 1]]), -500);
    }
}
