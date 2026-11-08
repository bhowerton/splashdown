//! Recording a meeting: the microphone and what the computer plays, kept
//! in step, spooled to disk as it goes, and cut into turns for the
//! transcript.

use super::*;

/// What a spool file is for, next to it as JSON.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Spool {
    pub(super) note: String,
    /// The notes folder the note is in.
    pub(super) folder: PathBuf,
    /// When the meeting started (RFC 3339).
    pub(super) started: String,
    /// Seconds of the recording whose turns are in the note.
    #[serde(default)]
    pub(super) written: f64,
    /// The app whose audio this is, when it isn't a meeting.
    #[serde(default)]
    pub(super) app: Option<String>,
}

pub(super) fn spool_dir() -> Result<PathBuf, String> {
    Ok(crate::engine::paths()?.config_dir.join("meetings"))
}

/// Where a meeting comes from, for its title: a call app in front when it
/// started.
pub(super) fn call_app() -> (Option<String>, String) {
    let Some(focus) = splashdown_platform::Focus::capture() else {
        return (None, "Meeting".into());
    };
    let app = &focus.app;
    let host = app.url.as_deref().unwrap_or_default();
    let name = app.name.to_lowercase();
    let label = if host.contains("meet.google.com") {
        Some("Google Meet")
    } else if name.contains("zoom") {
        Some("Zoom")
    } else if name.contains("teams") {
        Some("Teams")
    } else if name.contains("slack") {
        Some("Slack")
    } else if name.contains("facetime") {
        Some("FaceTime")
    } else if name.contains("webex") {
        Some("Webex")
    } else if name.contains("discord") {
        Some("Discord")
    } else {
        None
    };
    match label {
        Some(label) => (Some(app.name.clone()), format!("{label} meeting")),
        None => (None, "Meeting".into()),
    }
}

/// Records a meeting (or an app's audio) until it's stopped, then finishes
/// its note.
pub(super) fn record(
    app: &AppHandle,
    source: &Source,
    stop: &AtomicBool,
    keep: &AtomicBool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state.settings();
    let listening = match source {
        Source::App { name, .. } => Some(name.clone()),
        Source::Microphone => Some(MICROPHONE.to_string()),
        Source::Meeting => None,
    };
    let microphone_only = matches!(source, Source::Microphone);
    // Telling people they're recorded is about other people: a call, or
    // the room.
    if !matches!(source, Source::App { .. }) && !settings.notes.meeting_reminded {
        remind(app);
        match state.update(app, |s| s.notes.meeting_reminded = true) {
            Ok(saved) => settings = saved,
            Err(e) => tracing::warn!("couldn't save the meeting reminder: {e}"),
        }
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
    }
    let speech = state.session.file_speech()?;
    let (place, _) = crate::notes::files::place(&speech.stt.name());

    // An app's audio takes the microphone's place: it's the one side, and
    // keeps the time.
    let microphone = || {
        LiveAudio::microphone(
            splashdown_audio::devices::choose(
                &settings.general.microphones(),
                &splashdown_audio::devices::inputs(),
            )
            .as_deref(),
        )
        .map_err(|e| e.to_string())
    };
    let (mut you, mut them, them_problem) = match source {
        Source::Meeting => {
            let you = microphone()?;
            match system_audio() {
                Ok(them) => (you, Some(them), None),
                Err(e) => (you, None, Some(e)),
            }
        }
        Source::App { sources, .. } => (app_audio(sources)?, None, None),
        Source::Microphone => (microphone()?, None, None),
    };

    let now = chrono::Local::now();
    // A meeting on the calendar names the note and says who's in it.
    let event = match source {
        Source::Meeting if settings.notes.meeting_calendar => calendar_event(),
        _ => None,
    };
    let (app_name, title) = match &listening {
        _ if microphone_only => (None, "Recording".to_string()),
        Some(name) => (Some(name.clone()), name.clone()),
        None => {
            let (app_name, title) = call_app();
            (app_name, event.as_ref().map_or(title, |e| e.title.clone()))
        }
    };
    let (store, _) = crate::notes::store_for(settings.notes.folder.clone())?;
    let recorded_from = match &listening {
        _ if microphone_only => "Recorded from the microphone".to_string(),
        Some(name) => format!("Recorded from {name}"),
        None => "Recorded on this computer".into(),
    };
    let with = event
        .as_ref()
        .filter(|e| !e.people.is_empty())
        .map(|e| format!("{}\n\n", super::voices::with_line(&e.people)))
        .unwrap_or_default();
    let header = format!(
        "# {title} · {}\n\n{with}{recorded_from} · transcribed {place}.\n",
        now.format("%b %-d, %-I:%M %p")
    );
    let note = store
        .create_in(
            if listening.is_some() {
                ""
            } else {
                settings.notes.meeting_folder()
            },
            &header,
            now,
            &Origin {
                app: app_name.as_deref(),
                source: Some(match source {
                    Source::Meeting => "meeting",
                    Source::App { .. } => APP_AUDIO,
                    Source::Microphone => RECORDING,
                }),
                ..Origin::default()
            },
        )
        .map_err(|e| e.to_string())?;
    if let Some(problem) = &them_problem {
        let _ = store.add_paragraph(
            &note.path,
            &format!("*Only your side is recorded: {problem}*"),
        );
    }

    // The spool: both sides as they're recorded, until the note is done.
    let dir = spool_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = now.format("%Y%m%d-%H%M%S").to_string();
    let raw_path = dir.join(format!("{id}.raw"));
    let spool = Spool {
        note: note.path.clone(),
        folder: store.root().to_path_buf(),
        started: now.to_rfc3339(),
        written: 0.0,
        app: listening.clone(),
    };
    let meta_path = dir.join(format!("{id}.json"));
    std::fs::write(
        &meta_path,
        serde_json::to_string(&spool).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut raw =
        std::io::BufWriter::new(std::fs::File::create(&raw_path).map_err(|e| e.to_string())?);

    crate::notes::open(app);
    let mut step = InStep::default();
    let mut status = Status {
        note: note.path.clone(),
        them_problem: them_problem.clone(),
        place: place.to_string(),
        app: listening.clone(),
        ..Status::default()
    };
    show(app, Some(&status));
    let started = Instant::now();
    let mut asked: Option<Instant> = None;
    let mut ended = None;
    // Samples recorded on each side (in step), and when someone was last
    // heard, in milliseconds of recording, from the transcriber.
    let mut recorded = 0usize;
    let heard = AtomicU64::new(0);
    let mut kept_at = Duration::ZERO;
    let mut output = splashdown_audio::devices::default_output();
    let mut output_checked = Instant::now();

    // Transcribing runs beside the recording: a turn waiting for the
    // speech model (busy with a dictation, say) never holds up capture,
    // the spool or Stop.
    let one_sided = listening.is_some();
    let (echoes, lost_writes) = std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel::<(Vec<f32>, Vec<f32>)>();
        let (store, note_path, speech, heard) = (&store, &note.path, &speech, &heard);
        let spool = (meta_path.clone(), spool);
        let transcriber = scope.spawn(move || {
            let mut job = Job::new(store, note_path, speech);
            job.one_sided = one_sided;
            job.spool = Some(spool);
            while let Ok((you, them)) = rx.recv() {
                job.push(&you, &them);
                // Whatever else arrived meanwhile, then the turns.
                while let Ok((you, them)) = rx.try_recv() {
                    job.push(&you, &them);
                }
                job.transcribe(false);
                heard.store(job.heard_at.as_millis() as u64, Ordering::Relaxed);
            }
            (job.finish(), job.lost_writes)
        });

        loop {
            std::thread::sleep(TICK);
            // The computer's sound follows the output device: headphones
            // plugged in or AirPods connected mid-call.
            if output_checked.elapsed() >= OUTPUT_CHECK {
                output_checked = Instant::now();
                let now_output = splashdown_audio::devices::default_output();
                if now_output != output && them_problem.is_none() && listening.is_none() {
                    tracing::info!(
                        "the output device changed: recording the computer's sound again"
                    );
                    output = now_output;
                    them = match system_audio() {
                        Ok(again) => Some(again),
                        Err(e) => {
                            status.them_problem = Some(e);
                            None
                        }
                    };
                }
            }
            let mine = you.take();
            let theirs = match them.as_mut() {
                Some(them) => them.take(),
                // Without the other side, silence keeps it in step.
                None => vec![0.0; mine.len()],
            };
            let (mine, theirs) = step.push(&mine, &theirs);
            write_stereo(&mut raw, &mine, &theirs);
            recorded += mine.len();
            let _ = tx.send((mine, theirs));
            let recorded_for = Duration::from_secs_f64(recorded as f64 / RATE as f64);

            status.elapsed = recorded_for.as_secs_f64();
            status.you = you.level();
            status.them = them.as_ref().map_or(0.0, LiveAudio::level);

            if keep.swap(false, Ordering::Relaxed) {
                asked = None;
                status.warning = None;
                kept_at = recorded_for;
            }
            let heard_ms = heard.load(Ordering::Relaxed);
            status.heard = (heard_ms > 0).then(|| heard_ms as f64 / 1000.0);
            let heard_at = Duration::from_millis(heard_ms).max(kept_at);
            let quiet = recorded_for.saturating_sub(heard_at);
            let too_long = started.elapsed() >= LONGEST - GRACE;
            // Someone spoke after "Still in a meeting?": it is one.
            if asked.is_some() && !too_long && quiet < QUIET {
                asked = None;
                status.warning = None;
            }
            if asked.is_none() && ((settings.notes.stop_when_quiet() && quiet >= QUIET) || too_long)
            {
                asked = Some(Instant::now());
                status.warning = Some(if too_long {
                    "Recordings last 4 hours at most. Stopping in 1 minute.".into()
                } else if listening.is_some() {
                    "Still listening? Nothing has been said for 10 minutes. Stopping in 1 minute."
                        .into()
                } else {
                    "Still in a meeting? Nobody has spoken for 10 minutes. Stopping in 1 minute."
                        .into()
                });
                crate::notes::open(app);
            }
            show(app, Some(&status));

            let at = timestamp(recorded_for);
            if stop.load(Ordering::Relaxed) {
                ended = Some(format!("*Ended at {at}.*"));
            } else if you.lost() {
                ended = Some(match &listening {
                    Some(name) if !microphone_only => {
                        format!("*Stopped at {at}: {name}'s sound went away.*")
                    }
                    _ => format!("*Stopped at {at}: the microphone went away.*"),
                });
            } else if asked.is_some_and(|t| t.elapsed() >= GRACE) {
                ended = Some(if too_long {
                    format!("*Stopped at {at}, the longest a meeting records.*")
                } else {
                    format!("*Stopped at {at}: nobody had spoken for 10 minutes.*")
                });
            }
            if ended.is_some() {
                break;
            }
        }

        // The last of it.
        let mine = you.take();
        let theirs = match them.as_mut() {
            Some(them) => them.take(),
            None => vec![0.0; mine.len()],
        };
        let _ = step.push(&mine, &theirs);
        let (mine, theirs) = step.rest();
        write_stereo(&mut raw, &mine, &theirs);
        let _ = raw.flush();
        recorded += mine.len();
        let _ = tx.send((mine, theirs));
        drop(tx);
        status.finishing = true;
        status.warning = None;
        show(app, Some(&status));
        transcriber.join().unwrap_or_else(|_| {
            tracing::error!("transcribing the meeting panicked");
            (0, true)
        })
    });
    drop((you, them, raw));
    if let Some(ended) = &ended {
        let _ = store.add_paragraph(&note.path, ended);
    }
    if echoes > 0 {
        tell(
            app,
            "Some of what your speakers played was picked up by the microphone and left out. Headphones avoid that.".into(),
            false,
        );
    }
    if lost_writes {
        // The note couldn't take the turns (renamed, moved or deleted
        // meanwhile): the recording stays, to finish at the next launch.
        tell(
            app,
            "Part of the meeting couldn't be written into its note. Its recording is kept: Splashdown offers to finish it when it next starts.".into(),
            true,
        );
        return Ok(());
    }
    finish(
        app,
        &store,
        &note.path,
        &raw_path,
        &settings,
        listening.is_some(),
    );
    let _ = std::fs::remove_file(&raw_path);
    let _ = std::fs::remove_file(&meta_path);
    tracing::info!("meeting ended after {:.0} s", recorded as f64 / RATE as f64);
    Ok(())
}

/// Keeps You and Them in step. Each tick brings a slightly different
/// amount from each side; the surplus waits for the other side rather
/// than being padded with silence (which would put gaps in the recording
/// and stretch its timeline). A side that sends nothing for a while (the
/// computer's sound while nothing plays, on Windows) is filled with
/// silence once it's a second behind.
#[derive(Default)]
pub(super) struct InStep {
    pub(super) you: Vec<f32>,
    pub(super) them: Vec<f32>,
}

impl InStep {
    /// Takes a tick's audio; returns what both sides have, in step.
    pub(super) fn push(&mut self, you: &[f32], them: &[f32]) -> (Vec<f32>, Vec<f32>) {
        self.you.extend_from_slice(you);
        self.them.extend_from_slice(them);
        let ahead = self.you.len().max(self.them.len());
        let behind = ahead.saturating_sub(RATE);
        for side in [&mut self.you, &mut self.them] {
            if side.len() < behind {
                side.resize(behind, 0.0);
            }
        }
        let n = self.you.len().min(self.them.len());
        (
            self.you.drain(..n).collect(),
            self.them.drain(..n).collect(),
        )
    }

    /// Everything left, the shorter side padded to the longer.
    pub(super) fn rest(&mut self) -> (Vec<f32>, Vec<f32>) {
        let n = self.you.len().max(self.them.len());
        self.you.resize(n, 0.0);
        self.them.resize(n, 0.0);
        (
            std::mem::take(&mut self.you),
            std::mem::take(&mut self.them),
        )
    }
}

/// What the computer plays, as a live recording.
pub(super) fn system_audio() -> Result<LiveAudio, String> {
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::system_audio();
    #[cfg(windows)]
    return LiveAudio::system_output().map_err(|e| e.to_string());
    #[cfg(not(any(target_os = "macos", windows)))]
    Err("recording the computer's sound isn't supported here".into())
}

/// The spool: 16-bit stereo, You left and Them right. The shorter side is
/// padded so the two stay in step.
pub(super) fn write_stereo(out: &mut impl std::io::Write, left: &[f32], right: &[f32]) {
    let frames = left.len().max(right.len());
    let mut bytes = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        for side in [left, right] {
            let s = side.get(i).copied().unwrap_or(0.0).clamp(-1.0, 1.0);
            bytes.extend_from_slice(&((s * f32::from(i16::MAX)) as i16).to_le_bytes());
        }
    }
    if let Err(e) = out.write_all(&bytes) {
        tracing::warn!("couldn't write the meeting spool: {e}");
    }
}

/// The first meeting: a reminder that the people on the call should know.
/// The recording doesn't wait for it.
pub(super) fn remind(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        crate::prompt::ask(
            &app,
            "Recording a meeting",
            "Splashdown records your microphone and what your computer plays, and writes a transcript into your notes. Recording laws vary: let the people on the call know you're transcribing it.",
            &["Got it"],
        );
    });
}
