//! Transcribing a meeting's kept recording again with another model,
//! keeping the names given to speakers and the rest of the note.

use super::*;

/// The recording kept with a meeting note (its `[Recording](…)` link), if
/// it's still there.
pub(crate) fn recording_of(store: &Store, note: &str) -> Option<PathBuf> {
    let body = store.read(note).ok()?.body;
    let link = body
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("[Recording](")?.strip_suffix(')'))?;
    let rel = link.replace("%20", " ");
    // Only the notes folder's own attachments: plain names all the way
    // (on Windows, `\x.wav` and `C:x.wav` aren't "absolute" but still
    // leave the folder).
    if !Path::new(&rel)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    let path = store.root().join(rel);
    path.is_file().then_some(path)
}

/// Transcribes a meeting's kept recording again with `speech`, and puts the
/// new transcript in place of the old one; `model` names it in the note.
/// Speakers keep the names they were given (by the time each turn starts);
/// the rest of the note stays as it is. `progress` hears the seconds done,
/// and answers false to stop. Returns why it stopped early, if it did:
/// then nothing has changed.
pub(crate) fn transcribe_again(
    settings: &Settings,
    store: &Store,
    note: &str,
    recording: &Path,
    speech: &crate::session::FileSpeech,
    model: &str,
    progress: &mut dyn FnMut(f64) -> bool,
) -> Result<Option<String>, String> {
    let one_sided = store
        .list()
        .into_iter()
        .find(|n| n.path == note)
        .is_some_and(|n| n.source != "meeting");
    let mut reader = hound::WavReader::open(recording).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.channels != 2 || spec.sample_rate != RATE as u32 || spec.bits_per_sample != 16 {
        return Err("the recording isn't one Splashdown made".into());
    }
    // The samples as the meeting's spool had them, for telling speakers
    // apart afterwards.
    let raw = spool_dir()?.join(format!("again-{}.raw", std::process::id()));
    let mut spool =
        std::io::BufWriter::new(std::fs::File::create(&raw).map_err(|e| e.to_string())?);
    let result = (|| {
        let mut job = Job::new(store, note, speech);
        job.one_sided = one_sided;
        job.collected = Some(Vec::new());
        let mut samples = reader.samples::<i16>();
        loop {
            if !progress(job.recorded().as_secs_f64()) {
                return Ok(Some("Stopped. The transcript is as it was.".to_string()));
            }
            let (mut you, mut them) = (Vec::new(), Vec::new());
            for _ in 0..RATE / 2 {
                let (Some(l), Some(r)) = (samples.next(), samples.next()) else {
                    break;
                };
                let (l, r) = (l.map_err(|e| e.to_string())?, r.map_err(|e| e.to_string())?);
                let _ = spool.write_all(&l.to_le_bytes());
                let _ = spool.write_all(&r.to_le_bytes());
                you.push(f32::from(l) / f32::from(i16::MAX));
                them.push(f32::from(r) / f32::from(i16::MAX));
            }
            if you.is_empty() {
                break;
            }
            job.push(&you, &them);
            job.transcribe(false);
        }
        job.finish();
        let turns = job.collected.take().unwrap_or_default();
        if turns.is_empty() {
            return Ok(Some(
                "No speech was found. The transcript is as it was.".to_string(),
            ));
        }
        let mut named = false;
        let day = chrono::Local::now().format("%b %-d");
        store
            .rewrite(note, |body| {
                let (body, kept) = with_transcript(
                    body,
                    &turns,
                    &format!("*Transcribed again with {model} on {day}.*"),
                );
                named = kept;
                body
            })
            .map_err(|e| e.to_string())?;
        drop(spool.flush());
        if settings.notes.meeting_speakers && !named {
            name_speakers(
                store,
                note,
                &raw,
                one_sided,
                settings.notes.remember_speakers,
            );
        }
        Ok(None)
    })();
    let _ = std::fs::remove_file(&raw);
    result
}

/// `body` with `turns` in place of its transcript, and `said` (what made
/// them) after them. A speaker's name or number in the old transcript is
/// kept for the new turn starting at the same time; returns whether there
/// were any. What follows the transcript (when it ended, the recording's
/// link) stays.
pub(super) fn with_transcript(body: &str, turns: &[String], said: &str) -> (String, bool) {
    let lines: Vec<&str> = body.lines().collect();
    let aside = |l: &str| {
        let l = l.trim();
        (l.len() > 1 && l.starts_with('*') && l.ends_with('*') && !l.starts_with("**"))
            || l.starts_with("[Recording](")
    };
    // Where the transcript starts: its first turn, or else the notes at the
    // end of the note about how it ended.
    let start = lines
        .iter()
        .position(|l| Turn::parse(l).is_some())
        .unwrap_or_else(|| {
            let mut at = lines.len();
            while at > 0 && (aside(lines[at - 1]) || lines[at - 1].trim().is_empty()) {
                at -= 1;
            }
            at
        });
    let names: std::collections::HashMap<&str, &str> = lines[start..]
        .iter()
        .filter_map(|l| Turn::parse(l))
        .filter(|t| !t.who.is_empty() && t.who != Side::You.label() && t.who != Side::Them.label())
        .map(|t| (t.stamp, t.who))
        .collect();
    let turns: Vec<String> = turns
        .iter()
        .map(|t| match Turn::parse(t) {
            Some(turn) if names.contains_key(turn.stamp) => Turn {
                who: names[turn.stamp],
                ..turn
            }
            .line(),
            _ => t.clone(),
        })
        .collect();
    let (links, notes): (Vec<&str>, Vec<&str>) = lines[start..]
        .iter()
        .copied()
        .filter(|l| aside(l) && !l.trim().starts_with("*Transcribed again"))
        .partition(|l| l.trim().starts_with("[Recording]("));
    let mut out = lines[..start].join("\n").trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(&turns.join("\n\n"));
    for line in notes.into_iter().chain([said]).chain(links) {
        out.push_str("\n\n");
        out.push_str(line.trim());
    }
    out.push('\n');
    (out, !names.is_empty())
}
