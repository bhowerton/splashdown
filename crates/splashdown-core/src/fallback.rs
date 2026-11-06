//! Reliability: when a cloud provider fails (offline, an outage, a bad
//! key), the same request goes to a model on this computer instead, so the
//! dictation isn't lost. The result's `Usage::on_device_fallback` says so.

use std::sync::Arc;

use async_trait::async_trait;

use crate::AudioClip;
use crate::provider::{
    Completion, OnText, ProviderError, ProviderResult, SpeechToText, SttHints, TextModel,
    TextRequest, Transcript,
};

/// Speech-to-text that falls back to `backup` when `primary` fails.
pub struct FallbackStt {
    pub primary: Arc<dyn SpeechToText>,
    pub backup: Arc<dyn SpeechToText>,
}

#[async_trait]
impl SpeechToText for FallbackStt {
    fn name(&self) -> String {
        self.primary.name()
    }

    async fn transcribe(&self, audio: &AudioClip, hints: &SttHints) -> ProviderResult<Transcript> {
        let error = match self.primary.transcribe(audio, hints).await {
            Ok(transcript) => return Ok(transcript),
            Err(e) => e,
        };
        match self.backup.transcribe(audio, hints).await {
            Ok(mut transcript) => {
                transcript.usage.on_device_fallback = true;
                Ok(transcript)
            }
            // The first error is the one worth showing.
            Err(_) => Err(error),
        }
    }

    fn warm_up(&self) {
        self.primary.warm_up();
    }

    fn ready(&self) -> bool {
        self.primary.ready()
    }
}

/// A text model that falls back to `backup` when `primary` fails.
pub struct FallbackModel {
    pub primary: Arc<dyn TextModel>,
    pub backup: Arc<dyn TextModel>,
}

#[async_trait]
impl TextModel for FallbackModel {
    fn name(&self) -> String {
        self.primary.name()
    }

    async fn complete(&self, req: &TextRequest) -> ProviderResult<Completion> {
        match self.primary.complete(req).await {
            Ok(completion) => Ok(completion),
            Err(e) => self.stand_in(req, e).await,
        }
    }

    /// Streams from the primary; the backup, if it's needed, answers all at
    /// once.
    async fn complete_streaming(
        &self,
        req: &TextRequest,
        on_text: &mut OnText<'_>,
    ) -> ProviderResult<Completion> {
        match self.primary.complete_streaming(req, on_text).await {
            Ok(completion) => Ok(completion),
            Err(e) => self.stand_in(req, e).await,
        }
    }

    fn supports_json_schema(&self) -> bool {
        self.primary.supports_json_schema()
    }

    fn context_window(&self) -> usize {
        self.primary.context_window()
    }

    // Only the primary warms up: the backup loads if it's ever needed.
    fn warm_up(&self) {
        self.primary.warm_up();
    }
}

impl FallbackModel {
    /// The backup's answer after the primary failed with `error`.
    async fn stand_in(
        &self,
        req: &TextRequest,
        error: ProviderError,
    ) -> ProviderResult<Completion> {
        let mut req = req.clone();
        if !self.backup.supports_json_schema() {
            req.json_schema = None;
        }
        match self.backup.complete(&req).await {
            Ok(mut completion) => {
                // What the primary cost before it stopped counts too.
                if let ProviderError::Interrupted { usage, .. } = &error {
                    completion.usage += *usage;
                }
                completion.usage.on_device_fallback = true;
                Ok(completion)
            }
            Err(_) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Usage;

    struct Stt(Result<&'static str, ()>);

    #[async_trait]
    impl SpeechToText for Stt {
        fn name(&self) -> String {
            "stt".into()
        }
        async fn transcribe(&self, _: &AudioClip, _: &SttHints) -> ProviderResult<Transcript> {
            self.0
                .map(|text| Transcript {
                    words: Vec::new(),
                    text: text.into(),
                    usage: Usage::priced(0.01),
                })
                .map_err(|()| ProviderError::Network("offline".into()))
        }
    }

    fn run(
        primary: Result<&'static str, ()>,
        backup: Result<&'static str, ()>,
    ) -> ProviderResult<Transcript> {
        let stt = FallbackStt {
            primary: Arc::new(Stt(primary)),
            backup: Arc::new(Stt(backup)),
        };
        futures_executor::block_on(
            stt.transcribe(&AudioClip::new(vec![0.0], 16_000), &SttHints::default()),
        )
    }

    #[test]
    fn falls_back_only_when_the_cloud_fails() {
        let t = run(Ok("cloud"), Ok("local")).unwrap();
        assert_eq!(
            (t.text.as_str(), t.usage.on_device_fallback),
            ("cloud", false)
        );
        let t = run(Err(()), Ok("local")).unwrap();
        assert_eq!(
            (t.text.as_str(), t.usage.on_device_fallback),
            ("local", true)
        );
        assert!(matches!(
            run(Err(()), Err(())),
            Err(ProviderError::Network(_))
        ));
        // The flag survives adding usages up.
        assert!((Usage::priced(0.01) + t.usage).on_device_fallback);
    }

    /// Fails part way through its reply, having cost a cent.
    struct Cut;

    #[async_trait]
    impl TextModel for Cut {
        fn name(&self) -> String {
            "cut".into()
        }
        async fn complete(&self, _: &TextRequest) -> ProviderResult<Completion> {
            Err(ProviderError::Interrupted {
                message: "cut off".into(),
                usage: Usage::priced(0.01),
            })
        }
        fn supports_json_schema(&self) -> bool {
            false
        }
        fn context_window(&self) -> usize {
            1000
        }
    }

    /// Answers for free.
    struct Local;

    #[async_trait]
    impl TextModel for Local {
        fn name(&self) -> String {
            "local".into()
        }
        async fn complete(&self, _: &TextRequest) -> ProviderResult<Completion> {
            Ok(Completion {
                text: "Done.".into(),
                usage: Usage::FREE,
            })
        }
        fn supports_json_schema(&self) -> bool {
            false
        }
        fn context_window(&self) -> usize {
            1000
        }
    }

    #[test]
    fn what_a_cut_off_reply_cost_still_counts() {
        let model = FallbackModel {
            primary: Arc::new(Cut),
            backup: Arc::new(Local),
        };
        let req = TextRequest {
            system: String::new(),
            user: String::new(),
            temperature: 0.0,
            max_tokens: 10,
            json_schema: None,
        };
        let done = futures_executor::block_on(model.complete(&req)).unwrap();
        assert_eq!(done.text, "Done.");
        assert!((done.usage.cost_usd - 0.01).abs() < 1e-12);
        assert!(done.usage.on_device_fallback);
    }
}
