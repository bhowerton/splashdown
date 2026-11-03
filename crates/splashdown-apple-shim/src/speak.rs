//! Reading text aloud with the system's voices, through the Swift shim's
//! AVSpeechSynthesizer calls. Reading goes on in the background; `speaking`
//! says whether it still is.

use std::ffi::{CStr, CString, c_char, c_void};

use crate::ffi;

/// A voice that can read text aloud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    /// The system's identifier for it.
    pub id: String,
    pub name: String,
    /// e.g. "en-GB".
    pub language: String,
    /// 1 usual, 2 enhanced, 3 premium.
    pub quality: u8,
}

/// Reads `text` aloud, stopping anything being read. `voice` is used when
/// it speaks `language` (the text's, "de"); otherwise the best voice for
/// the language is. `speed` 1.0 is the voice's usual pace.
pub fn speak(text: &str, voice: Option<&str>, language: Option<&str>, speed: f32) {
    let c = |s: &str| CString::new(s.replace('\0', "")).unwrap_or_default();
    let (text, voice, language) = (
        c(text),
        c(voice.unwrap_or_default()),
        c(language.unwrap_or_default()),
    );
    // SAFETY: the strings are NUL-terminated and outlive the call, which
    // copies them.
    unsafe {
        ffi::splashdown_speak(
            text.as_ptr(),
            voice.as_ptr(),
            language.as_ptr(),
            f64::from(speed),
        );
    }
}

/// Stops reading at once.
pub fn stop_speaking() {
    // SAFETY: no arguments; safe from any thread.
    unsafe { ffi::splashdown_speak_stop() }
}

/// Whether something is still being read.
pub fn speaking() -> bool {
    // SAFETY: no arguments; safe from any thread.
    unsafe { ffi::splashdown_speaking() }
}

/// The voices that can read text, as the system lists them.
pub fn voices() -> Vec<Voice> {
    extern "C" fn on_reply(ctx: *mut c_void, _status: i32, text: *const c_char) {
        if text.is_null() {
            return;
        }
        // SAFETY: `ctx` is the `String` below, alive for the whole call, and
        // `text` is a NUL-terminated string valid during this callback.
        unsafe {
            *ctx.cast::<String>() = CStr::from_ptr(text).to_string_lossy().into_owned();
        }
    }
    let mut listed = String::new();
    // SAFETY: the shim calls back once, before returning.
    unsafe { ffi::splashdown_voices((&raw mut listed).cast(), on_reply) };
    listed
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            Some(Voice {
                id: parts.next()?.to_string(),
                name: parts.next()?.to_string(),
                language: parts.next()?.to_string(),
                quality: parts.next()?.parse().unwrap_or(1),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs the Mac's voices; run with --ignored"]
    fn lists_voices() {
        let voices = voices();
        assert!(voices.iter().any(|v| v.language.starts_with("en")));
        assert!(voices.iter().all(|v| (1..=3).contains(&v.quality)));
        assert!(!speaking());
    }
}
