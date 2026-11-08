//! Settings → Notes: where notes go, meetings, and agents' access.

use super::*;

/// Notes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotesSettings {
    /// Where notes are kept, one Markdown file each. Unset means
    /// `Documents/Splashdown Notes`.
    pub folder: Option<std::path::PathBuf>,
    /// Where a captured dictation (dictation key + N) is added.
    pub capture_to: CaptureTo,
    /// Daily notes' folder inside the notes folder. Unset means "Daily".
    pub daily_folder: Option<String>,
    /// Daily notes' file name, with YYYY, MM and DD. Unset means
    /// YYYY-MM-DD (Obsidian's default).
    pub daily_format: Option<String>,
    /// Keep a captured dictation's recording next to the note.
    pub keep_audio: bool,
    /// Let coding agents search and read notes (through `splashdown-mcp`).
    pub agents_read: bool,
    /// …and add to them.
    pub agents_write: bool,
    /// …including meetings and transcripts (other people's words). Unset
    /// means on, as before this choice existed; Settings saves it.
    pub agents_meetings: Option<bool>,
    /// Let Shortcuts actions (Mac) find, read and ask about notes. Unset
    /// means on: the user builds their own shortcuts. Notes hidden from
    /// agents stay hidden from them too.
    pub shortcuts_read: Option<bool>,
    /// Meeting notes' folder inside the notes folder. Unset means
    /// "Meetings".
    pub meeting_folder: Option<String>,
    /// Whether a meeting is summarized when it ends. Unset means always
    /// with an AI on this computer, and asking first with a cloud one.
    pub meeting_summary: Option<MeetingSummary>,
    /// Keep a meeting's recording with its note.
    pub keep_meeting_audio: bool,
    /// Stop a meeting after 10 minutes with nobody speaking. Unset means on.
    pub stop_when_quiet: Option<bool>,
    /// Show a small bar on screen while a meeting records. Unset means on
    /// on Windows, where the tray can't show the time, and off on macOS,
    /// where the menu bar does.
    pub meeting_pill: Option<bool>,
    /// Offer to record a meeting when a call app starts using the
    /// microphone. Unset means on.
    pub offer_on_calls: Option<bool>,
    /// Call apps not to offer for ("Zoom"), from the offer's "Don't offer
    /// for Zoom".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub offer_skip: Vec<String>,
    /// Questions asked with the ask key may use excerpts of matching notes.
    /// Unset means on.
    pub ask_uses_notes: Option<bool>,
    /// The first meeting's reminder about telling people has been shown.
    pub meeting_reminded: bool,
    /// Keep the Notes window above other apps' windows.
    pub keep_on_top: bool,
    /// Name a meeting (and list who's in it) from the calendar event on
    /// when it starts. macOS only.
    pub meeting_calendar: bool,
    /// Tell the other people in a meeting apart (Speaker 1, Speaker 2…)
    /// with the downloaded diarization model.
    pub meeting_speakers: bool,
    /// Know people the user named in a meeting again in later ones, by
    /// voice (with the downloaded voice model). Needs `meeting_speakers`.
    pub remember_speakers: bool,
    /// Ask finds notes by what they mean too, not only by their words,
    /// with the downloaded embedding model.
    pub ask_by_meaning: bool,
    /// Folders whose new audio and video files are transcribed into notes
    /// while Splashdown runs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub watched: Vec<WatchedFolder>,
}

/// A folder whose new audio and video files become transcript notes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchedFolder {
    pub path: std::path::PathBuf,
    /// The notes folder (inside the notes folder, "" for the top) the
    /// transcripts go into.
    pub into: String,
    /// Also save subtitles (`name.srt`) next to each file.
    pub subtitles: bool,
}

/// Whether to summarize a meeting when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MeetingSummary {
    Always,
    Ask,
    Never,
}

impl NotesSettings {
    pub fn meeting_folder(&self) -> &str {
        self.meeting_folder
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .unwrap_or("Meetings")
    }

    /// Whether coding agents may reach meetings and transcripts, when
    /// they may read notes at all.
    pub fn agents_meetings(&self) -> bool {
        self.agents_meetings.unwrap_or(true)
    }

    /// Whether Shortcuts actions may find, read and ask about notes.
    pub fn shortcuts_read(&self) -> bool {
        self.shortcuts_read.unwrap_or(true)
    }

    pub fn stop_when_quiet(&self) -> bool {
        self.stop_when_quiet.unwrap_or(true)
    }

    pub fn meeting_pill(&self) -> bool {
        self.meeting_pill.unwrap_or(cfg!(windows))
    }

    pub fn offer_on_calls(&self) -> bool {
        self.offer_on_calls.unwrap_or(true)
    }

    pub fn ask_uses_notes(&self) -> bool {
        self.ask_uses_notes.unwrap_or(true)
    }

    pub fn daily_folder(&self) -> &str {
        self.daily_folder
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .unwrap_or("Daily")
    }

    pub fn daily_format(&self) -> &str {
        self.daily_format
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .unwrap_or("YYYY-MM-DD")
    }
}

/// Where a captured dictation goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureTo {
    /// `Inbox.md` at the top of the notes folder.
    #[default]
    Inbox,
    /// Today's daily note.
    Daily,
    /// The note last opened in Notes.
    Last,
}
