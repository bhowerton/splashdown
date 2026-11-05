//! A dictation into a note instead of the field (the note letter): which
//! note it goes to, adding it, and taking it back.

use super::*;

/// A dictation added to a note (dictation key + N), with what's needed to
/// take it back.
pub struct Captured {
    pub(super) folder: Option<PathBuf>,
    pub(super) appended: Appended,
    /// "Inbox", "2026-10-31", or the note's title.
    pub label: String,
}

impl Captured {
    /// The note's file, absolute.
    pub fn file(&self) -> Option<PathBuf> {
        let (store, _) = store_for(self.folder.clone()).ok()?;
        Some(store.root().join(&self.appended.path))
    }
}

pub(super) fn capture_target(settings: &NotesSettings) -> Target {
    match settings.capture_to {
        CaptureTo::Inbox => Target::Inbox,
        CaptureTo::Daily => Target::Daily {
            folder: settings.daily_folder().to_string(),
            format: settings.daily_format().to_string(),
        },
        CaptureTo::Last => Target::Last,
    }
}

/// Where a capture would go now, for the overlay ("Inbox").
pub fn capture_label(settings: &NotesSettings) -> String {
    match settings.capture_to {
        CaptureTo::Inbox => "Inbox".into(),
        CaptureTo::Daily => "today's note".into(),
        CaptureTo::Last => "the last note".into(),
    }
}

/// Adds `text` to the capture note (or `to`, a note's path), with a link
/// to `audio` (a WAV recording) when there is one.
pub fn capture(
    settings: &NotesSettings,
    to: Option<&str>,
    text: &str,
    audio: Option<&[u8]>,
) -> Result<Captured, String> {
    let (store, _) = store_for(settings.folder.clone())?;
    let now = chrono::Local::now();
    let mut text = text.trim().to_string();
    if let Some(bytes) = audio {
        match store.save_attachment(&now.format("%Y-%m-%d %H%M").to_string(), "wav", bytes) {
            // A plain Markdown link, so it works outside Obsidian too.
            Ok(rel) => text.push_str(&format!(" [Recording]({})", rel.replace(' ', "%20"))),
            Err(e) => tracing::warn!("couldn't keep the recording with the note: {e}"),
        }
    }
    let target = to.map_or_else(
        || capture_target(settings),
        |p| {
            // "Ideas" means the note Ideas.md.
            let p = p.trim();
            if Path::new(p)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            {
                Target::Note(p.to_string())
            } else {
                Target::Note(format!("{p}.md"))
            }
        },
    );
    let appended = store
        .append(&target, &text, now)
        .map_err(|e| format!("Couldn't add it to your notes: {e}"))?;
    let label = Path::new(&appended.path)
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    Ok(Captured {
        folder: settings.folder.clone(),
        appended,
        label,
    })
}

/// Adds a to-do (`- [ ] text`) to today's note, for voice actions.
pub fn add_todo(settings: &NotesSettings, text: &str) -> Result<Captured, String> {
    let (store, _) = store_for(settings.folder.clone())?;
    let target = Target::Daily {
        folder: settings.daily_folder().to_string(),
        format: settings.daily_format().to_string(),
    };
    let appended = store
        .append(
            &target,
            &format!("- [ ] {}", text.trim()),
            chrono::Local::now(),
        )
        .map_err(|e| format!("Couldn't add it to your notes: {e}"))?;
    let label = Path::new(&appended.path)
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    Ok(Captured {
        folder: settings.folder.clone(),
        appended,
        label,
    })
}

/// Takes a capture back, if the note hasn't changed since.
pub fn undo_capture(captured: &Captured) -> Result<bool, String> {
    let (store, _) = store_for(captured.folder.clone())?;
    store
        .undo_append(&captured.appended)
        .map_err(|e| e.to_string())
}
