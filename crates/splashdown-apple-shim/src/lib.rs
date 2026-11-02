//! Apple's on-device models — SpeechAnalyzer for speech-to-text and
//! FoundationModels for text — through a small Swift static library with a
//! C ABI. Requires macOS 26 and Apple Silicon at run time; on
//! older systems everything reports `Unavailable`.
//!
//! The crate is empty on every OS except macOS.
#![cfg(target_os = "macos")]

mod calendar;
mod ffi;
mod providers;
mod screenshot;
mod tap;

pub use calendar::{
    AddError, Added, CalendarEvent, add_event, add_reminder, calendar_access, calendar_now, remove,
};
pub use providers::{AppleLanguageModel, AppleSpeech};
pub use screenshot::{Screenshot, ScreenshotError, screenshot, zoom};
pub use tap::{app_audio, system_audio};

use std::ffi::CString;

/// Why an Apple model can't be used right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Available,
    /// Apple Intelligence is turned off in System Settings.
    NotEnabled,
    /// This Mac can't run it (e.g. Intel).
    NotEligible,
    /// The model or its assets are still downloading.
    NotReady,
    /// Needs macOS 26 or later.
    OsTooOld,
    /// The language isn't supported.
    UnsupportedLanguage,
    /// Speech assets for the language are available but not installed.
    NeedsDownload,
    Other(String),
}

impl Status {
    fn from_code(code: i32, message: &str) -> Self {
        match code {
            ffi::OK => Status::Available,
            ffi::NOT_ENABLED => Status::NotEnabled,
            ffi::NOT_ELIGIBLE => Status::NotEligible,
            ffi::NOT_READY | ffi::DOWNLOADING => Status::NotReady,
            ffi::OS_TOO_OLD => Status::OsTooOld,
            ffi::UNSUPPORTED_LANGUAGE => Status::UnsupportedLanguage,
            ffi::NEEDS_DOWNLOAD => Status::NeedsDownload,
            _ => Status::Other(message.to_string()),
        }
    }

    /// A sentence for the UI.
    pub fn describe(&self) -> String {
        match self {
            Status::Available => "ready".into(),
            Status::NotEnabled => {
                "turn on Apple Intelligence in System Settings › Apple Intelligence & Siri".into()
            }
            Status::NotEligible => "this Mac can't run Apple's on-device models".into(),
            Status::NotReady => "Apple's model is still downloading; try again shortly".into(),
            Status::OsTooOld => "needs macOS 26 or later".into(),
            Status::UnsupportedLanguage => "your language isn't supported yet".into(),
            Status::NeedsDownload => "the speech model for your language needs downloading".into(),
            Status::Other(m) => m.clone(),
        }
    }
}

/// Whether Apple's on-device language model can be used. Instant.
pub fn language_model_status() -> Status {
    // SAFETY: no arguments; the shim checks OS availability itself.
    let code = unsafe { ffi::splashdown_apple_llm_status() };
    Status::from_code(code, "")
}

/// Whether Apple's speech model is installed for `language` (ISO code, or
/// the system language). On success also returns the locale used.
pub async fn speech_status(language: Option<&str>) -> (Status, String) {
    let locale = language.map(|l| CString::new(l).unwrap_or_default());
    // SAFETY: `locale` is null or a NUL-terminated string that outlives the
    // call (the shim copies it before returning); `ctx` and `cb` come
    // straight from `call`, and the shim calls `cb` exactly once.
    let (code, text) = ffi::call(|ctx, cb| unsafe {
        ffi::splashdown_apple_stt_status(
            locale.as_ref().map_or(std::ptr::null(), |l| l.as_ptr()),
            ctx,
            cb,
        )
    })
    .await;
    (Status::from_code(code, &text), text)
}

/// Downloads Apple's speech assets for `language` (can take a minute).
pub async fn prepare_speech(language: Option<&str>) -> Result<(), Status> {
    let locale = language.map(|l| CString::new(l).unwrap_or_default());
    // SAFETY: as in `speech_status`: `locale` is null or a live C string the
    // shim copies, and `ctx`/`cb` come from `call` and are used once.
    let (code, text) = ffi::call(|ctx, cb| unsafe {
        ffi::splashdown_apple_stt_prepare(
            locale.as_ref().map_or(std::ptr::null(), |l| l.as_ptr()),
            ctx,
            cb,
        )
    })
    .await;
    match Status::from_code(code, &text) {
        Status::Available => Ok(()),
        other => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use splashdown_core::{SpeechToText, SttHints, TextModel, TextRequest};

    /// Links the Swift library and calls into it; the answer depends on the Mac.
    #[test]
    fn status_calls_work() {
        let llm = language_model_status();
        eprintln!("language model: {llm:?}");
        let (speech, locale) = futures_executor::block_on(speech_status(Some("en")));
        eprintln!("speech: {speech:?} ({locale})");
        // The answers depend on the Mac, but every status must explain
        // itself, and a usable language always names its locale.
        assert!(!llm.describe().is_empty());
        assert!(!speech.describe().is_empty());
        if matches!(speech, Status::Available | Status::NeedsDownload) {
            assert!(locale.starts_with("en"), "{locale}");
        }
    }

    /// Runs the real model. `cargo test -p splashdown-apple-shim -- --ignored`
    #[test]
    #[ignore = "needs Apple Intelligence"]
    fn language_model_cleans_up() {
        let model = AppleLanguageModel::new().expect("Apple's model is available");
        let out = futures_executor::block_on(model.complete(&TextRequest {
            system: "Fix punctuation and capitalization and remove filler words. Output only the text.".into(),
            user: "<transcript>\num so I think uh we should ship it on friday\n</transcript>".into(),
            temperature: 0.2,
            max_tokens: 200,
            json_schema: None,
        }))
        .expect("completion");
        eprintln!("{}", out.text);
        assert!(out.text.to_lowercase().contains("friday"));
    }

    /// Transcribes a WAV made by `splashdown-cli eval make-audio`.
    #[test]
    #[ignore = "needs Apple's speech model and eval audio"]
    fn speech_transcribes() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../eval/audio/short-greeting.wav"
        );
        let clip = splashdown_audio::wav::read(std::path::Path::new(path)).expect("eval audio");
        let t = futures_executor::block_on(AppleSpeech.transcribe(&clip, &SttHints::default()))
            .expect("transcription");
        eprintln!("{}", t.text);
        assert!(t.text.to_lowercase().contains("call"));
    }
}
