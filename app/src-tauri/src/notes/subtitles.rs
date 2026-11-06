//! Subtitles for a transcript or meeting note. A file transcribed with a
//! local speech model keeps when each word was said; that's saved in the
//! app's data folder by the note's `splashdown-id`, not in the notes
//! folder, so the note stays as it was. Without it (cloud speech, meetings,
//! older notes), captions are estimated from the note's timed lines.

use super::*;
use splashdown_core::meeting::Turn;
use splashdown_core::subtitles::{self, Cue, Line, TimedWord};

/// What a transcription kept for its subtitles.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Timings {
    /// Every word with its time, when the speech model gave them all.
    #[serde(default)]
    pub words: Vec<TimedWord>,
    /// Where the speech of each of the note's lines starts and ends, in
    /// order: for estimating without words.
    #[serde(default)]
    pub spans: Vec<(u64, u64)>,
}

fn file(id: &str) -> Result<PathBuf, String> {
    // The id is ours ("1730000000-ab12"), but it's read from a file the
    // user can edit: nothing that leaves the folder.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("This note's id isn't one Splashdown wrote.".into());
    }
    Ok(engine::paths()?
        .config_dir
        .join("subtitles")
        .join(format!("{id}.json")))
}

/// Keeps `timings` for the note with `id`.
pub(super) fn save(id: &str, timings: &Timings) -> Result<(), String> {
    let path = file(id)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string(timings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

fn load(id: &str) -> Option<Timings> {
    let text = std::fs::read_to_string(file(id).ok()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Captions for the note `rel`, and whether their times are estimated.
pub(super) fn cues(store: &Store, rel: &str) -> Result<(Vec<Cue>, bool), String> {
    let note = store.read(rel).map_err(|e| e.to_string())?;
    let turns: Vec<Turn> = note.body.lines().filter_map(Turn::parse).collect();
    if turns.is_empty() {
        return Err("This note has no timed transcript to make subtitles from.".into());
    }
    let timings = store.id(rel).and_then(|id| load(&id)).unwrap_or_default();
    Ok(cues_from(&turns, &timings))
}

/// `cues`, from the note's lines and what was kept.
fn cues_from(turns: &[Turn], timings: &Timings) -> (Vec<Cue>, bool) {
    if !timings.words.is_empty() {
        return (subtitles::from_words(&timings.words), false);
    }
    // Spans line up with the lines only if the note's lines are the ones
    // transcribed (not edited since).
    let spans = (timings.spans.len() == turns.len()).then_some(&timings.spans);
    let lines: Vec<Line> = turns
        .iter()
        .enumerate()
        .map(|(i, turn)| {
            let span = spans.map(|s| s[i]);
            Line {
                start: span.map_or_else(|| turn.at(), |(start, _)| Duration::from_millis(start)),
                end: span.map(|(_, end)| Duration::from_millis(end)),
                who: turn.who,
                said: turn.said,
            }
        })
        .collect();
    (subtitles::from_lines(&lines, None), true)
}

/// The note `rel` in `folder`'s notes as SRT or WebVTT text, for a
/// shortcut (Get Note Text → Subtitles). The caller has checked it may
/// read the note.
#[cfg(target_os = "macos")]
pub(crate) fn text_for(folder: Option<PathBuf>, rel: &str, vtt: bool) -> Result<String, String> {
    let (store, _) = store_for(folder)?;
    let (cues, _) = cues(&store, rel)?;
    Ok(if vtt {
        subtitles::vtt(&cues)
    } else {
        subtitles::srt(&cues)
    })
}

/// Saves the note's subtitles next to the file it was transcribed from
/// (`talk.mp4` → `talk.srt`), unless subtitles are already there.
pub(super) fn save_beside(store: &Store, rel: &str, media: &Path) -> Result<(), String> {
    let target = media.with_extension("srt");
    if target.exists() {
        tracing::info!("{} has subtitles already; left alone", media.display());
        return Ok(());
    }
    let (cues, _) = cues(store, rel)?;
    if cues.is_empty() {
        return Ok(());
    }
    std::fs::write(&target, subtitles::srt(&cues)).map_err(|e| e.to_string())
}

/// What an export wrote.
#[derive(Debug, Serialize)]
pub struct Exported {
    pub file: String,
    /// Times are estimated, not from the words.
    pub approximate: bool,
}

/// Notes window → Export subtitles: asks where, and writes SRT or WebVTT
/// there. `None` when the user closed the dialog.
#[tauri::command]
pub async fn notes_export_subtitles(
    path: String,
    format: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Exported>, String> {
    let vtt = match format.as_str() {
        "srt" => false,
        "vtt" => true,
        _ => return Err(format!("Subtitles can't be saved as {format}.")),
    };
    let (store, _) = store(&state)?;
    let (cues, approximate) = cues(&store, &path)?;
    if cues.is_empty() {
        return Err("This note's transcript has nothing to show as subtitles.".into());
    }
    let text = if vtt {
        subtitles::vtt(&cues)
    } else {
        subtitles::srt(&cues)
    };
    let title = store
        .read(&path)
        .map(|n| markdown::title_of(&n.body))
        .unwrap_or_default();
    let name = format!(
        "{}.{}",
        safe_file_name(&title).unwrap_or_else(|| "Subtitles".into()),
        if vtt { "vtt" } else { "srt" }
    );
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Save subtitles")
            .set_file_name(&name)
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(target) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    std::fs::write(&target, text).map_err(|e| format!("Couldn't save the subtitles: {e}"))?;
    Ok(Some(Exported {
        file: target.display().to_string(),
        approximate,
    }))
}

/// `title` as a file name: without characters file systems refuse.
fn safe_file_name(title: &str) -> Option<String> {
    let name: String = title
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|".contains(c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turns(body: &'static str) -> Vec<Turn<'static>> {
        body.lines().filter_map(Turn::parse).collect()
    }

    #[test]
    fn kept_words_time_the_captions_exactly() {
        let timings = Timings {
            words: vec![
                TimedWord {
                    text: "Hello".into(),
                    start_ms: 500,
                    end_ms: 900,
                },
                TimedWord {
                    text: "there.".into(),
                    start_ms: 900,
                    end_ms: 1400,
                },
            ],
            spans: Vec::new(),
        };
        let (cues, approximate) = cues_from(&turns("**[0:00]** Hello there."), &timings);
        assert!(!approximate);
        assert_eq!(cues[0].start, Duration::from_millis(500));
        assert_eq!(cues[0].text, "Hello there.");
    }

    #[test]
    fn without_words_the_lines_and_their_spans_are_used() {
        let body = "# Call\n\n**[0:00] Them:** Hi there.\n\n**[0:30] You:** Hello.\n";
        let kept = Timings {
            words: Vec::new(),
            spans: vec![(1200, 2400), (31_000, 32_000)],
        };
        let (cues, approximate) = cues_from(&turns(body), &kept);
        assert!(approximate);
        assert_eq!(cues[0].start, Duration::from_millis(1200));
        assert_eq!(cues[0].text, "Them: Hi there.");
        assert_eq!(cues[1].start, Duration::from_millis(31_000));
        assert_eq!(cues[1].text, "You: Hello.");

        // A line added since: the spans don't fit, the stamps are used.
        let edited = format!("{body}\n**[0:40] You:** One more.\n");
        let edited: &'static str = Box::leak(edited.into_boxed_str());
        let (cues, _) = cues_from(&turns(edited), &kept);
        assert_eq!(cues[0].start, Duration::ZERO);
        assert_eq!(cues[1].start, Duration::from_secs(30));
    }

    #[test]
    fn a_title_becomes_a_file_name() {
        assert_eq!(
            safe_file_name("Q3: plan / review?").as_deref(),
            Some("Q3 plan review")
        );
        assert_eq!(safe_file_name("  "), None);
    }
}
