//! From a recording: speech-to-text, then the text pass.

use std::time::Instant;

use super::{AudioJob, Outcome, Pipeline, PipelineError, Prefix, TextJob};
use crate::provider::{OnText, SttHints, Usage};
use crate::{AudioClip, Mode};

impl Pipeline {
    pub async fn run(&self, job: AudioJob) -> Result<Outcome, PipelineError> {
        self.run_streaming(job, None).await
    }

    /// `run`, passing the AI's reply so far to `on_text` as it arrives, for
    /// typing it while the rest is written. Only plain dictation and
    /// dictation with an instruction stream, and only when no snippet was
    /// said (the model writes placeholders for those). The outcome is still
    /// what to insert in the end: the guard may reject what was streamed.
    pub async fn run_streaming(
        &self,
        job: AudioJob,
        on_text: Option<&mut OnText<'_>>,
    ) -> Result<Outcome, PipelineError> {
        let started = Instant::now();
        let speech = self.transcribe_speech(&job.speech, job.prefix, &job.hints);
        let ((speech, stt_usage), instruction) = match &job.instruction_speech {
            Some(clip) => {
                let (a, b) =
                    futures_util::future::join(speech, self.transcribe(clip, &job.hints)).await;
                let (instruction, usage) = b?;
                let (speech, speech_usage) = a?;
                (
                    (speech, speech_usage + usage),
                    Some(instruction).filter(|s| !s.is_empty()),
                )
            }
            None => (speech.await?, None),
        };
        let stt_time = started.elapsed();

        if speech.is_empty() {
            return Err(PipelineError::NoSpeech);
        }

        let (content, instruction) = match job.mode {
            Mode::Transform => (String::new(), Some(speech)),
            // A marker with nothing said after it is just a dictation.
            Mode::DictateInstruct if instruction.is_none() => (speech, None),
            _ => (speech, instruction),
        };
        let mode = match (job.mode, &instruction) {
            (Mode::DictateInstruct, None) => Mode::Dictate,
            (m, _) => m,
        };

        let (mode, instruction) = match (mode, instruction, job.standing_instruction) {
            (Mode::Dictate, None, Some(standing)) => (Mode::DictateInstruct, Some(standing)),
            (mode, instruction, _) => (mode, instruction),
        };
        let mut outcome = self
            .process_text_streaming(
                TextJob {
                    mode,
                    content,
                    instruction,
                    selection: job.selection,
                    context: job.context,
                },
                on_text,
            )
            .await?;
        outcome.stt_time = stt_time;
        outcome.stt_name = Some(self.stt.name());
        outcome.usage += stt_usage;
        Ok(outcome)
    }

    /// Transcribes a recording on its own (a spoken command).
    pub async fn transcribe_only(
        &self,
        clip: &AudioClip,
        hints: &SttHints,
    ) -> Result<(String, Usage), PipelineError> {
        self.transcribe(clip, hints).await
    }

    /// The speech, joined to its prefix if there is one. A failed prefix
    /// means transcribing the whole recording again.
    async fn transcribe_speech(
        &self,
        clip: &AudioClip,
        prefix: Option<Prefix>,
        hints: &SttHints,
    ) -> Result<(String, Usage), PipelineError> {
        let Some(prefix) = prefix else {
            return self.transcribe(clip, hints).await;
        };
        let (head, tail) =
            futures_util::future::join(prefix.transcript, self.transcribe(clip, hints)).await;
        match (head, tail) {
            (Ok((head, head_usage)), Ok((tail, tail_usage))) => Ok((
                format!("{head} {tail}").trim().to_string(),
                head_usage + tail_usage,
            )),
            // (The app logs the prefix's error where it ran.)
            _ => self.transcribe(&prefix.whole, hints).await,
        }
    }

    async fn transcribe(
        &self,
        clip: &AudioClip,
        hints: &SttHints,
    ) -> Result<(String, Usage), PipelineError> {
        // A prefix can leave nothing after it.
        if clip.samples.is_empty() {
            return Ok((String::new(), Usage::FREE));
        }
        let transcript = self
            .stt
            .transcribe(clip, hints)
            .await
            .map_err(PipelineError::Stt)?;
        Ok((transcript.text.trim().to_string(), transcript.usage))
    }
}
