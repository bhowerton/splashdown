//! Subtitles for a transcript or meeting note. A file transcribed with a
//! local speech model keeps when each word was said; that's saved in the
//! settings folder by the note's `splashdown-id`, not in the notes
//! folder, so the note stays as it was. Without it (cloud speech, meetings,
//! older notes, or a transcript edited since), captions are estimated from
//! the note's timed lines.

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
    crate::data::for_note("subtitles", id)
}

/// Keeps `timings` for the note with `id`.
pub(super) fn save(id: &str, timings: &Timings) -> Result<(), String> {
    crate::data::write(&file(id)?, timings)
}

fn load(id: &str) -> Option<Timings> {
    Some(crate::data::read(&file(id).ok()?))
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

/// `cues`, from the note's lines and what was kept: the kept words while
/// the note still says them (an edit, such as a misheard name put right,
/// would otherwise be lost from the captions).
fn cues_from(turns: &[Turn], timings: &Timings) -> (Vec<Cue>, bool) {
    let letters = |text: &str| -> String {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let unedited = || {
        let words: String = timings.words.iter().map(|w| letters(&w.text)).collect();
        let note: String = turns.iter().map(|t| letters(t.said)).collect();
        words == note
    };
    if !timings.words.is_empty() && unedited() {
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
pub(crate) fn text_for(
    folder: Option<PathBuf>,
    rel: &str,
    format: subtitles::Format,
) -> Result<String, String> {
    let (store, _) = store_for(folder)?;
    let (cues, _) = cues(&store, rel)?;
    Ok(format.render(&cues))
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
    let Some(format) = subtitles::Format::named(&format) else {
        return Err(format!("Subtitles can't be saved as {format}."));
    };
    let (store, _) = store(&state)?;
    let (cues, approximate) = cues(&store, &path)?;
    if cues.is_empty() {
        return Err("This note's transcript has nothing to show as subtitles.".into());
    }
    let text = format.render(&cues);
    let title = store
        .read(&path)
        .map(|n| markdown::title_of(&n.body))
        .unwrap_or_default();
    let name = format!(
        "{}.{}",
        Some(super::markdown::file_name_part(&title))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Subtitles".into()),
        format.extension()
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

        // Edited since: the note's words, at estimated times.
        let (cues, approximate) = cues_from(&turns("**[0:00]** Hello, Dana."), &timings);
        assert!(approximate);
        assert_eq!(cues[0].text, "Hello, Dana.");
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
}
