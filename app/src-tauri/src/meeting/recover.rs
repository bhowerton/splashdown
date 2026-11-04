//! After a crash: finishing the meetings whose recordings were left in
//! the spool.

use super::*;

/// At launch: a meeting whose spool is still there wasn't finished (the
/// app crashed or was forced to quit). Offers to finish its transcript.
pub fn recover(app: &AppHandle) {
    let Ok(dir) = spool_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let spools: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    if spools.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        for meta in spools {
            if let Err(e) = recover_one(&app, &meta) {
                tracing::warn!("couldn't finish an earlier meeting: {e}");
            }
        }
    });
}

pub(super) fn recover_one(app: &AppHandle, meta: &Path) -> Result<(), String> {
    let spool: Spool =
        serde_json::from_str(&std::fs::read_to_string(meta).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let raw = meta.with_extension("raw");
    let when = chrono::DateTime::parse_from_rfc3339(&spool.started)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%b %-d, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_default();
    let answer = crate::prompt::ask(
        app,
        "Finish transcribing it?",
        &format!(
            "The {} recorded {when} wasn't finished when Splashdown closed. Its recording is still here.",
            if spool.app.is_some() {
                "audio"
            } else {
                "meeting"
            }
        ),
        &["Finish", "Delete it", "Later"],
    );
    if answer == Some(1) {
        let _ = std::fs::remove_file(&raw);
        let _ = std::fs::remove_file(meta);
        return Ok(());
    }
    if answer != Some(0) {
        return Ok(());
    }

    let state = app.state::<AppState>();
    let settings = state.settings();
    let speech = state.session.file_speech()?;
    let (store, _) = crate::notes::store_for(Some(spool.folder.clone()))?;
    let note = &spool.note;
    // The note already has the turns up to where the spool says (older
    // spools: its last time stamp).
    let resume_at = if spool.written > 0.0 {
        Duration::from_secs_f64(spool.written)
    } else {
        store
            .read(note)
            .ok()
            .and_then(|n| last_timestamp(&n.body))
            .unwrap_or_default()
    };
    let mut job = Job::new(&store, note, &speech);
    job.resume_at = resume_at;
    job.one_sided = spool.app.is_some();
    // Half a second at a time, as when it was recorded: all at once, every
    // turn would scan the whole meeting.
    let mut file = std::io::BufReader::new(std::fs::File::open(&raw).map_err(|e| e.to_string())?);
    let mut bytes = vec![0u8; RATE / 2 * 4];
    loop {
        let n = read_full(&mut file, &mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let (mut you, mut them) = (Vec::new(), Vec::new());
        for frame in bytes[..n].as_chunks::<4>().0 {
            you.push(f32::from(i16::from_le_bytes([frame[0], frame[1]])) / f32::from(i16::MAX));
            them.push(f32::from(i16::from_le_bytes([frame[2], frame[3]])) / f32::from(i16::MAX));
        }
        job.push(&you, &them);
        job.transcribe(false);
    }
    job.finish();
    if job.lost_writes {
        return Err("the meeting note couldn't be written to; its recording is kept".into());
    }
    let _ = store.add_paragraph(
        note,
        &format!(
            "*Finished after Splashdown closed; the recording ends at {}.*",
            timestamp(job.recorded())
        ),
    );
    finish(app, &store, note, &raw, &settings, spool.app.is_some());
    let _ = std::fs::remove_file(&raw);
    let _ = std::fs::remove_file(meta);
    Ok(())
}

/// Fills `buf` from `file` as far as it goes; returns how much was read.
pub(super) fn read_full(file: &mut impl std::io::Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// The time of the last transcript turn in a meeting note.
pub(super) fn last_timestamp(body: &str) -> Option<Duration> {
    body.lines().rev().find_map(Turn::parse).map(|t| t.at())
}
