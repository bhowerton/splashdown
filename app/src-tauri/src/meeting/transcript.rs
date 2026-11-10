//! The transcript: transcribing each side's turns, writing them into the
//! note as the call goes, finishing it, and telling speakers apart.

use super::*;

/// The turn-by-turn transcript, written into the note as it's finished.
pub(super) struct Job<'a> {
    pub(super) store: &'a Store,
    pub(super) note: &'a str,
    pub(super) stt: Arc<dyn splashdown_core::SpeechToText>,
    pub(super) hints: splashdown_core::SttHints,
    pub(super) cleanup: splashdown_audio::Cleanup,
    pub(super) cutters: [TurnCutter; 2],
    pub(super) meeting: Meeting,
    /// Samples taken from each side so far.
    pub(super) taken: [usize; 2],
    /// When someone last spoke, in recording time.
    pub(super) heard_at: Duration,
    /// Turns that ended before this (an earlier, unfinished run of the
    /// same meeting) are already in the note.
    pub(super) resume_at: Duration,
    /// The spool's sidecar, kept up to date with how much is written.
    pub(super) spool: Option<(PathBuf, Spool)>,
    /// A turn couldn't be written into the note.
    pub(super) lost_writes: bool,
    /// One side only (an app's audio): turns are written without You or
    /// Them.
    pub(super) one_sided: bool,
    /// Turns are kept here instead of written into the note (transcribing
    /// a recording again).
    pub(super) collected: Option<Vec<String>>,
    /// With the spoken language on Automatic: the recording's language, to
    /// transcribe again a short turn that came out in another.
    pub(super) language: splashdown_core::turn_language::TurnLanguage,
}

impl<'a> Job<'a> {
    pub(super) fn new(
        store: &'a Store,
        note: &'a str,
        speech: &crate::session::FileSpeech,
    ) -> Self {
        Self {
            store,
            note,
            stt: speech.stt.clone(),
            hints: speech.hints.clone(),
            cleanup: speech.cleanup,
            cutters: [TurnCutter::new(), TurnCutter::new()],
            meeting: Meeting::new(),
            taken: [0, 0],
            heard_at: Duration::ZERO,
            resume_at: Duration::ZERO,
            spool: None,
            lost_writes: false,
            one_sided: false,
            collected: None,
            // The computer's language until what's said shows otherwise.
            language: splashdown_core::turn_language::TurnLanguage::new(
                splashdown_platform::system_language(),
            ),
        }
    }

    pub(super) fn recorded(&self) -> Duration {
        Duration::from_secs_f64(self.taken[0].max(self.taken[1]) as f64 / RATE as f64)
    }

    pub(super) fn push(&mut self, you: &[f32], them: &[f32]) {
        self.cutters[0].push(you);
        self.cutters[1].push(them);
        self.taken[0] += you.len();
        self.taken[1] += them.len();
    }

    /// Transcribes the turns that are ready (all of them when `ended`) and
    /// writes the paragraphs that are final.
    pub(super) fn transcribe(&mut self, ended: bool) {
        loop {
            // The side whose next turn is ready, earliest first... a turn
            // only comes out once its end is known, so take one at a time.
            let mut progressed = false;
            for (i, side) in [Side::You, Side::Them].into_iter().enumerate() {
                let Some(piece) = self.cutters[i].next(ended) else {
                    continue;
                };
                progressed = true;
                let end = piece.start + piece.clip.duration();
                // What started before the resume point is in the note
                // (see `Meeting::settled`), even if it ended after it.
                if piece.start < self.resume_at {
                    self.meeting.reached(side, end);
                    continue;
                }
                let text = match splashdown_audio::prepare_with(&piece.clip, self.cleanup) {
                    Some(clip) => {
                        self.heard_at = self.heard_at.max(end);
                        let text = transcribe(self.stt.as_ref(), &clip, &self.hints);
                        self.in_its_language(&clip, text)
                    }
                    None => String::new(),
                };
                self.meeting.push(Segment {
                    side,
                    start: piece.start,
                    end,
                    text,
                });
            }
            let ready = self.meeting.ready();
            let settled = self.meeting.settled();
            self.write(ready, settled);
            if !progressed {
                return;
            }
        }
    }

    /// `text`, a turn transcribed from `clip`, transcribed again in the
    /// recording's language if it's short and came out in another (a
    /// multilingual model guessing wrong on a noisy turn). Only with the
    /// spoken language on Automatic: a language the user set is already
    /// used for every turn.
    fn in_its_language(&mut self, clip: &splashdown_core::AudioClip, text: String) -> String {
        if !self.hints.language.as_deref().is_none_or(str::is_empty) {
            return text;
        }
        let text = match self.language.language() {
            Some(lang) if self.language.is_stray(&text) => {
                let hints = splashdown_core::SttHints {
                    language: Some(lang.to_string()),
                    ..self.hints.clone()
                };
                let again = transcribe(self.stt.as_ref(), clip, &hints);
                tracing::info!("a turn came out in another language: transcribed again in {lang}");
                if again.is_empty() { text } else { again }
            }
            _ => text,
        };
        self.language.heard(&text);
        text
    }

    /// Transcribes what's left and writes the rest. Returns how many
    /// microphone turns were dropped as echo.
    pub(super) fn finish(&mut self) -> usize {
        self.transcribe(true);
        let meeting = std::mem::take(&mut self.meeting);
        let echoes = meeting.echoes;
        let rest = meeting.finish();
        // All of it is in the note now: if the app quits during what
        // follows (the summary, keeping the recording), finishing it later
        // adds no turn twice.
        let all = self.recorded();
        self.write(rest, all);
        echoes
    }

    /// Writes `paragraphs` into the note, and notes in the spool that the
    /// note now has every turn that started before `upto`.
    pub(super) fn write(&mut self, paragraphs: Vec<Paragraph>, upto: Duration) {
        for p in paragraphs {
            let stamp = timestamp(p.start);
            let line = Turn {
                stamp: &stamp,
                who: if self.one_sided { "" } else { p.side.label() },
                said: &p.text,
            }
            .line();
            if let Some(collected) = &mut self.collected {
                collected.push(line);
                continue;
            }
            // After one turn couldn't be written, none is: finishing the
            // meeting later starts from that turn, and would add again
            // whatever went in after it.
            if self.lost_writes {
                continue;
            }
            if let Err(e) = self.store.add_paragraph(self.note, &line) {
                tracing::warn!("couldn't add to the meeting note: {e}");
                self.lost_writes = true;
            }
        }
        // How far the note goes, for finishing it after a crash.
        if !self.lost_writes
            && let Some((path, spool)) = &mut self.spool
            && upto.as_secs_f64() > spool.written
        {
            spool.written = upto.as_secs_f64();
            if let Err(e) = crate::data::write(path, spool) {
                tracing::warn!("couldn't note how far the meeting's note goes: {e}");
            }
        }
    }
}

/// One turn, tried twice; a failure leaves a marker rather than losing
/// the meeting.
pub(super) fn transcribe(
    stt: &dyn splashdown_core::SpeechToText,
    clip: &splashdown_core::AudioClip,
    hints: &splashdown_core::SttHints,
) -> String {
    let attempt = || tauri::async_runtime::block_on(stt.transcribe(clip, hints));
    match attempt().or_else(|e| {
        tracing::warn!("a meeting turn failed, trying again: {e}");
        attempt()
    }) {
        Ok(t) => {
            // A cloud model's turns cost money, like any dictation's.
            crate::spend::shared().record(&[(stt.name(), t.usage)], chrono::Local::now());
            t.text.trim().to_string()
        }
        Err(e) => {
            tracing::warn!("a meeting turn couldn't be transcribed: {e}");
            "*(couldn't be transcribed)*".into()
        }
    }
}

/// After the transcript: the summary (as the settings say, asking first
/// for a cloud AI) and the recording, if it's kept. `Err` when it was to be
/// kept and couldn't be: the caller then leaves the spool, the only copy.
pub(super) fn finish(
    app: &AppHandle,
    store: &Store,
    note: &str,
    raw: &Path,
    settings: &Settings,
    one_sided: bool,
) -> Result<(), String> {
    // Speakers first, so the summary can say who said what. Recognizing
    // people needs them told apart first.
    if settings.notes.tells_speakers_apart() {
        name_speakers(
            store,
            note,
            raw,
            one_sided,
            settings.notes.remember_speakers,
        );
    }
    summarize(app, store, note, settings, one_sided);
    let kept = if settings.notes.keep_meeting_audio {
        add_recording(store, note, raw)
    } else {
        Ok(())
    };
    let event = if one_sided {
        splashdown_config::HookEvent::Transcript
    } else {
        splashdown_config::HookEvent::Meeting
    };
    crate::hooks::note_finished(app, event, note);
    kept
}

/// Keeps the recording with the note, and links it from there.
pub(super) fn add_recording(store: &Store, note: &str, raw: &Path) -> Result<(), String> {
    let link = keep_recording(store, note, raw)?;
    let _ = store.add_paragraph(note, &format!("[Recording]({})", link.replace(' ', "%20")));
    Ok(())
}

/// Tells the other people apart (a meeting's Them side, or everyone in a
/// one-sided recording) with the downloaded diarization model, and labels
/// their turns Speaker 1, Speaker 2… Nothing changes with only one voice,
/// or without the model.
pub(super) fn name_speakers(
    store: &Store,
    note: &str,
    raw: &Path,
    one_sided: bool,
    remember: bool,
) {
    let Ok(models) = crate::engine::paths().map(|p| p.models_dir) else {
        return;
    };
    let model = splashdown_providers::models::Diarizer::path_in(&models);
    if !model.is_file() {
        return;
    }
    let Ok(bytes) = std::fs::read(raw) else {
        return;
    };
    // The spool is You left, Them right; a one-sided recording is all left.
    let channel = if one_sided { 0 } else { 2 };
    let pcm: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|f| f32::from(i16::from_le_bytes([f[channel], f[channel + 1]])) / f32::from(i16::MAX))
        .collect();
    let started = Instant::now();
    let turns = match splashdown_providers::diarize(&model, &pcm) {
        Ok(turns) => turns,
        Err(e) => {
            tracing::warn!("couldn't tell the speakers apart: {e}");
            return;
        }
    };
    tracing::info!(
        "speakers told apart in {:.1} s ({} turns)",
        started.elapsed().as_secs_f32(),
        turns.len()
    );
    let spans: Vec<(u32, f64, f64)> = turns
        .iter()
        .map(|t| (t.speaker, t.start.as_secs_f64(), t.end.as_secs_f64()))
        .collect();
    let side = if one_sided { None } else { Some("Them") };
    let mut order = Vec::new();
    let rewritten = store.rewrite(note, |body| {
        let (body, numbered) = labelled(body, &spans, side);
        order = numbered;
        body
    });
    if let Err(e) = rewritten {
        tracing::warn!("couldn't label the speakers: {e}");
        return;
    }
    if remember {
        let labels = voice_labels(&order, &turns, side);
        if !labels.is_empty() {
            super::voices::recognize_speakers(store, note, &pcm, &turns, &labels);
        }
    }
}

/// Who gets a voiceprint, by the label the transcript gives them: Speaker
/// 1, 2… when the other side was split; on a call with one other person,
/// that person, still labelled `side` ("Them").
fn voice_labels(
    order: &[u32],
    turns: &[splashdown_providers::SpeakerTurn],
    side: Option<&str>,
) -> Vec<(String, u32)> {
    if !order.is_empty() {
        return order
            .iter()
            .enumerate()
            .map(|(i, &speaker)| (format!("Speaker {}", i + 1), speaker))
            .collect();
    }
    let mut speakers: Vec<u32> = turns.iter().map(|t| t.speaker).collect();
    speakers.sort_unstable();
    speakers.dedup();
    match (side, speakers.as_slice()) {
        (Some(side), [only]) => vec![(side.to_string(), *only)],
        _ => Vec::new(),
    }
}

/// `body` with the turns from one side (`side`, or every unlabelled turn of
/// a one-sided recording with `None`) labelled by who spoke: each turn gets
/// the speaker of `spans` (speaker, start s, end s) it overlaps most,
/// numbered in the order they first speak. Unchanged unless there are two
/// or more.
#[cfg(test)]
pub(super) fn label_speakers(body: &str, spans: &[(u32, f64, f64)], side: Option<&str>) -> String {
    labelled(body, spans, side).0
}

/// `label_speakers`, and the speakers of `spans` in the order they were
/// numbered (Speaker 1 first); empty when nothing was labelled.
fn labelled(body: &str, spans: &[(u32, f64, f64)], side: Option<&str>) -> (String, Vec<u32>) {
    // Each turn of that side: its line, and when it starts and ends (the
    // next turn's start).
    let lines: Vec<&str> = body.split('\n').collect();
    let mut turns: Vec<(usize, f64)> = Vec::new();
    let mut starts: Vec<f64> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(turn) = Turn::parse(line) else {
            continue;
        };
        let at = turn.at().as_secs_f64();
        starts.push(at);
        if turn.who == side.unwrap_or("") {
            turns.push((i, at));
        }
    }
    let end_of = |at: f64| {
        starts
            .iter()
            .copied()
            .filter(|&s| s > at)
            .fold(f64::INFINITY, f64::min)
    };
    let speaker_of = |from: f64, to: f64| {
        let mut best: Option<(u32, f64)> = None;
        for &(speaker, s, e) in spans {
            let overlap = e.min(to) - s.max(from);
            if overlap > 0.0 && best.is_none_or(|(_, o)| overlap > o) {
                best = Some((speaker, overlap));
            }
        }
        best.map(|(speaker, _)| speaker)
    };
    let found: Vec<Option<u32>> = turns
        .iter()
        .map(|&(_, at)| speaker_of(at, end_of(at).min(at + 120.0)))
        .collect();
    // Numbered as they first speak.
    let mut order: Vec<u32> = Vec::new();
    for s in found.iter().flatten() {
        if !order.contains(s) {
            order.push(*s);
        }
    }
    if order.len() < 2 {
        return (body.to_string(), Vec::new());
    }
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for (&(i, _), speaker) in turns.iter().zip(&found) {
        let (Some(speaker), Some(turn)) = (speaker, Turn::parse(lines[i])) else {
            continue;
        };
        let n = order.iter().position(|s| s == speaker).unwrap_or(0) + 1;
        let who = format!("Speaker {n}");
        out[i] = Turn { who: &who, ..turn }.line();
    }
    (out.join("\n"), order)
}

/// The spool as a WAV in `Attachments/`. Returns its path for the link.
pub(super) fn keep_recording(store: &Store, note: &str, raw: &Path) -> Result<String, String> {
    let stem = Path::new(note)
        .file_stem()
        .map_or_else(|| "Meeting".into(), |s| s.to_string_lossy().into_owned());
    let (path, rel) = store
        .new_attachment(&stem, "wav")
        .map_err(|e| e.to_string())?;
    let written = (|| {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(raw).map_err(|e| e.to_string())?;
        for pair in bytes.as_chunks::<2>().0 {
            writer
                .write_sample(i16::from_le_bytes(*pair))
                .map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())
    })();
    if let Err(e) = written {
        // Half a recording (the disk filled up) isn't one to leave behind.
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(rel)
}

#[cfg(test)]
mod voice_tests {
    use super::*;
    use splashdown_providers::SpeakerTurn;

    fn turn(speaker: u32, start: u64) -> SpeakerTurn {
        SpeakerTurn {
            speaker,
            start: Duration::from_secs(start),
            end: Duration::from_secs(start + 5),
        }
    }

    #[test]
    fn a_split_side_gets_its_speaker_labels() {
        let turns = [turn(3, 0), turn(1, 10)];
        assert_eq!(
            voice_labels(&[3, 1], &turns, Some("Them")),
            [("Speaker 1".to_string(), 3), ("Speaker 2".to_string(), 1)]
        );
    }

    #[test]
    fn one_other_person_is_them() {
        let turns = [turn(2, 0), turn(2, 30)];
        assert_eq!(
            voice_labels(&[], &turns, Some("Them")),
            [("Them".to_string(), 2)]
        );
        // A recording of the room has no label to name.
        assert!(voice_labels(&[], &turns, None).is_empty());
        // Nothing heard on their side.
        assert!(voice_labels(&[], &[], Some("Them")).is_empty());
    }
}
