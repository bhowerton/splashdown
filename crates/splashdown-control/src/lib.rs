//! The running app's control channel, for things other than the dictation
//! key: `splashdown-cli`, `splashdown://` links, and coding agents (through
//! `splashdown-mcp`).
//!
//! One JSON line per connection each way. The request is
//! `{"cmd":"toggle"|"start"|"stop"|"cancel"}`, `{"cmd":"listen","prompt":"…"}`,
//! `{"cmd":"note","text":"…"}`, or one of the requests for coding agents
//! (notes: `list_notes`, `search_notes`, `read_note`, `append_note`;
//! dictation history: `search_history`); the reply is `{"ok":true}`,
//! `{"ok":true,"text":"…"}` (`listen`: the cleaned-up answer; agents'
//! requests: JSON) or `{"ok":false,"error":"…"}`.
//!
//! macOS listens on a Unix socket in the config folder
//! (`Paths::control_socket`), mode 0600, and checks that every peer is the
//! same user. Windows uses the named pipe `\\.\pipe\splashdown-<user>-<session>`, which only
//! this user (and administrators) can write to.

mod server;

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use server::{Handler, serve};

/// The longest line either side accepts (a `listen` prompt, a long answer).
pub const MAX_LINE: u64 = 256 * 1024;

/// Windows: the app's named pipe, one per user and sign-in session.
/// Pipe names are shared by the whole machine, so with one name a second
/// person signed in at the same time (fast user switching, a shared PC)
/// couldn't open it, and their copy would take that for "already running"
/// and quit.
#[cfg(windows)]
pub fn pipe_name() -> String {
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    let user: String = std::env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut session = 0u32;
    // SAFETY: our own process id, and a valid place for the answer.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    format!(r"\\.\pipe\splashdown-{user}-{session}")
}

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    /// Start hands-free dictation into the focused field, or finish the
    /// recording in progress.
    Toggle,
    /// Start hands-free dictation (nothing happens if one is running).
    Start,
    /// Finish the recording in progress and insert it.
    Stop,
    /// Throw the recording in progress away.
    Cancel,
    /// Record a hands-free answer, showing `prompt` in the overlay, and
    /// reply with the cleaned-up text instead of inserting it.
    Listen {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        /// Where the answer goes, for the overlay once it's sent: "Sent to
        /// Shortcuts". Unset: "Answer sent".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
    /// With `text`, add it to the capture note. Without, start a
    /// hands-free recording that goes into a note, or finish the one in
    /// progress.
    Note {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// Notes for coding agents, when Settings allows it: the
    /// notes matching `query`, as JSON.
    #[serde(rename = "search_notes")]
    SearchNotes {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
        #[serde(default, skip_serializing_if = "NoteFilter::is_empty")]
        filter: NoteFilter,
    },
    /// The notes `filter` lets through, newest first, as JSON: a page of
    /// `limit` from `cursor`.
    #[serde(rename = "list_notes")]
    ListNotes {
        #[serde(default, skip_serializing_if = "NoteFilter::is_empty")]
        filter: NoteFilter,
        /// Each meeting's summary too.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        with_summary: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
    },
    /// A note's text, or one part of it, as JSON.
    #[serde(rename = "read_note")]
    ReadNote {
        path: String,
        #[serde(default, skip_serializing_if = "ReadPart::is_whole")]
        part: ReadPart,
    },
    /// Add `text` to the capture note, or to the note at `to`.
    #[serde(rename = "append_note")]
    AppendNote {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
    /// Dictation history for coding agents, when Settings allows it (a
    /// switch apart from the notes ones): the entries matching `query`,
    /// newest first, as JSON. `since` and `until` are the agent's words,
    /// as in [`NoteFilter`].
    #[serde(rename = "search_history")]
    SearchHistory {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<String>,
    },
    /// The settings file was changed outside the app (`splashdown-cli
    /// words import`): read it again.
    #[serde(rename = "reload_settings")]
    ReloadSettings,
    /// Start recording a meeting; answers with the note's path.
    #[serde(rename = "meeting_start")]
    MeetingStart,
    /// Stop the meeting and finish its note.
    #[serde(rename = "meeting_stop")]
    MeetingStop,
    /// The meeting in progress, as JSON (or `null`).
    #[serde(rename = "meeting_status")]
    MeetingStatus,
    /// The app's state as JSON: version, whether it's ready, recording,
    /// which providers it uses, and any meeting.
    Status,
    /// Show the Settings window. A second copy of the app sends this to
    /// the first, then quits.
    Settings,
}

impl Request {
    /// The command for a `splashdown://<name>` link or a CLI word.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "toggle" => Self::Toggle,
            "start" => Self::Start,
            "stop" => Self::Stop,
            "cancel" => Self::Cancel,
            "note" => Self::Note { text: None },
            _ => return None,
        })
    }

    /// Parses a request line.
    pub fn parse(line: &str) -> Result<Self, String> {
        serde_json::from_str(line.trim()).map_err(|e| format!("bad request: {e}"))
    }
}

/// Which notes a list or search covers. Every field is optional; dates
/// are the agent's words (`7d`, `2026-11-02`), checked by the app.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteFilter {
    /// A named range: `today`, `yesterday`, `this_week`, `last_week`…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// `meeting`, `transcript`, `capture` or `note`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// A folder, with its subfolders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// A tag, with the tags nested under it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl NoteFilter {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// What of a note `read_note` returns. The default is the whole note.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadPart {
    /// `all`, `summary`, `my_notes` or `transcript`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// The transcript from this time (`m:ss`, `h:mm:ss` or seconds)…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// …to this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// Only what this speaker said (`You`, `Them` or a name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
    /// Where to carry on, from the last reply's `next_cursor`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl ReadPart {
    pub fn is_whole(&self) -> bool {
        self == &Self::default()
    }
}

/// The answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: None,
            error: Some(message.into()),
        }
    }

    pub fn into_result(self) -> Result<Option<String>, String> {
        if self.ok {
            Ok(self.text)
        } else {
            Err(self
                .error
                .unwrap_or_else(|| "Splashdown couldn't do that".into()))
        }
    }
}

impl From<Result<Option<String>, String>> for Reply {
    fn from(result: Result<Option<String>, String>) -> Self {
        match result {
            Ok(text) => Self {
                ok: true,
                text,
                error: None,
            },
            Err(e) => Self::error(e),
        }
    }
}

/// `value` as one JSON line.
fn to_line(value: &impl Serialize) -> String {
    // Compact JSON never contains a raw newline.
    let mut line = serde_json::to_string(value).unwrap_or_else(|_| "{}".into());
    line.push('\n');
    line
}

/// Reads one line of at most `MAX_LINE` bytes. `None` at end of stream.
fn read_line(stream: impl Read) -> io::Result<Option<String>> {
    let mut reader = BufReader::new(stream.take(MAX_LINE + 1));
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Ok(None);
    }
    if n as u64 > MAX_LINE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(Some(line))
}

/// Where the running app listens.
pub fn endpoint() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        Some(PathBuf::from(pipe_name()))
    }
    #[cfg(not(windows))]
    {
        splashdown_config::Paths::resolve().map(|p| p.control_socket())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Splashdown isn't running — open the app first")]
    NotRunning,
    #[error("couldn't reach Splashdown: {0}")]
    Io(#[from] io::Error),
    #[error("Splashdown closed the connection without answering")]
    NoReply,
    #[error("Splashdown sent a reply this tool doesn't understand: {0}")]
    BadReply(String),
    /// The app answered with an error.
    #[error("{0}")]
    App(String),
}

/// Sends `request` to the running app and waits for its answer. For
/// `Listen`, that's when the user has finished speaking (minutes, maybe).
pub fn send(request: &Request) -> Result<Option<String>, ClientError> {
    let stream = connect()?;
    exchange(stream, request)?
        .into_result()
        .map_err(ClientError::App)
}

/// One request and its reply over an open connection.
fn exchange<S: Read + Write>(mut stream: S, request: &Request) -> Result<Reply, ClientError> {
    stream.write_all(to_line(request).as_bytes())?;
    stream.flush()?;
    let line = read_line(&mut stream)?.ok_or(ClientError::NoReply)?;
    serde_json::from_str(line.trim()).map_err(|e| ClientError::BadReply(e.to_string()))
}

#[cfg(unix)]
fn connect() -> Result<std::os::unix::net::UnixStream, ClientError> {
    let path = endpoint().ok_or(ClientError::NotRunning)?;
    std::os::unix::net::UnixStream::connect(&path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => ClientError::NotRunning,
        _ => ClientError::Io(e),
    })
}

#[cfg(windows)]
fn connect() -> Result<std::fs::File, ClientError> {
    /// `ERROR_PIPE_BUSY`: every pipe instance is taken for a moment.
    const PIPE_BUSY: i32 = 231;
    let mut tries = 0;
    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe_name())
        {
            Ok(f) => return Ok(f),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(ClientError::NotRunning),
            Err(e) if e.raw_os_error() == Some(PIPE_BUSY) && tries < 40 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(ClientError::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse() {
        assert_eq!(Request::parse(r#"{"cmd":"toggle"}"#), Ok(Request::Toggle));
        assert_eq!(Request::parse(" {\"cmd\":\"stop\"}\n"), Ok(Request::Stop));
        assert_eq!(
            Request::parse(r#"{"cmd":"settings"}"#),
            Ok(Request::Settings)
        );
        assert_eq!(
            Request::parse(r#"{"cmd":"listen"}"#),
            Ok(Request::Listen {
                prompt: None,
                to: None
            })
        );
        assert_eq!(
            Request::parse(r#"{"cmd":"listen","prompt":"Which one?"}"#),
            Ok(Request::Listen {
                prompt: Some("Which one?".into()),
                to: None
            })
        );
        assert_eq!(
            Request::parse(r#"{"cmd":"listen","to":"Shortcuts"}"#),
            Ok(Request::Listen {
                prompt: None,
                to: Some("Shortcuts".into())
            })
        );
        assert!(Request::parse(r#"{"cmd":"format-disk"}"#).is_err());
        assert!(Request::parse(r#"{"command":"toggle"}"#).is_err());
        assert!(Request::parse("toggle").is_err());
    }

    #[test]
    fn notes_requests_parse() {
        assert_eq!(
            Request::parse(r#"{"cmd":"note","text":"buy milk"}"#),
            Ok(Request::Note {
                text: Some("buy milk".into())
            })
        );
        assert_eq!(
            Request::from_name("note"),
            Some(Request::Note { text: None })
        );
        assert_eq!(
            Request::parse(r#"{"cmd":"search_notes","query":"launch"}"#),
            Ok(Request::SearchNotes {
                query: "launch".into(),
                limit: None,
                filter: NoteFilter::default(),
            })
        );
        assert_eq!(
            Request::parse(
                r#"{"cmd":"list_notes","filter":{"when":"this_week","kind":"meeting"},"with_summary":true}"#
            ),
            Ok(Request::ListNotes {
                filter: NoteFilter {
                    when: Some("this_week".into()),
                    kind: Some("meeting".into()),
                    ..NoteFilter::default()
                },
                with_summary: true,
                limit: None,
                cursor: None,
            })
        );
        // An older splashdown-mcp's read_note is still the whole note.
        assert_eq!(
            Request::parse(r#"{"cmd":"read_note","path":"a.md"}"#),
            Ok(Request::ReadNote {
                path: "a.md".into(),
                part: ReadPart::default(),
            })
        );
        assert_eq!(
            to_line(&Request::ReadNote {
                path: "a.md".into(),
                part: ReadPart {
                    section: Some("summary".into()),
                    ..ReadPart::default()
                },
            }),
            "{\"cmd\":\"read_note\",\"path\":\"a.md\",\"part\":{\"section\":\"summary\"}}\n"
        );
        assert_eq!(
            to_line(&Request::AppendNote {
                text: "hi".into(),
                to: None
            }),
            "{\"cmd\":\"append_note\",\"text\":\"hi\"}\n"
        );
    }

    #[test]
    fn history_requests_parse() {
        assert_eq!(
            Request::parse(r#"{"cmd":"search_history","query":""}"#),
            Ok(Request::SearchHistory {
                query: String::new(),
                limit: None,
                since: None,
                until: None,
            })
        );
        assert_eq!(
            Request::parse(
                r#"{"cmd":"search_history","query":"invoice","limit":5,"since":"7d","until":"2026-11-02"}"#
            ),
            Ok(Request::SearchHistory {
                query: "invoice".into(),
                limit: Some(5),
                since: Some("7d".into()),
                until: Some("2026-11-02".into()),
            })
        );
        assert_eq!(
            to_line(&Request::SearchHistory {
                query: "invoice".into(),
                limit: None,
                since: Some("yesterday".into()),
                until: None,
            }),
            "{\"cmd\":\"search_history\",\"query\":\"invoice\",\"since\":\"yesterday\"}\n"
        );
        // The query may be empty but not left out, as in search_notes.
        assert!(Request::parse(r#"{"cmd":"search_history"}"#).is_err());
    }

    #[test]
    fn requests_and_replies_are_single_lines() {
        let line = to_line(&Request::Listen {
            prompt: Some("two\nlines".into()),
            to: None,
        });
        assert_eq!(line, "{\"cmd\":\"listen\",\"prompt\":\"two\\nlines\"}\n");
        assert_eq!(to_line(&Request::Cancel), "{\"cmd\":\"cancel\"}\n");
        assert_eq!(to_line(&Reply::from(Ok(None))), "{\"ok\":true}\n");
        assert_eq!(
            to_line(&Reply::from(Ok(Some("hi".into())))),
            "{\"ok\":true,\"text\":\"hi\"}\n"
        );
        assert_eq!(
            to_line(&Reply::error("Not recording")),
            "{\"ok\":false,\"error\":\"Not recording\"}\n"
        );
    }

    #[test]
    fn replies_turn_into_results() {
        let parse = |s: &str| serde_json::from_str::<Reply>(s).unwrap().into_result();
        assert_eq!(parse(r#"{"ok":true}"#), Ok(None));
        assert_eq!(parse(r#"{"ok":true,"text":"yes"}"#), Ok(Some("yes".into())));
        assert_eq!(parse(r#"{"ok":false,"error":"nope"}"#), Err("nope".into()));
        assert!(parse(r#"{"ok":false}"#).is_err());
    }

    #[test]
    fn link_names() {
        assert_eq!(Request::from_name("toggle"), Some(Request::Toggle));
        assert_eq!(Request::from_name("cancel"), Some(Request::Cancel));
        assert_eq!(Request::from_name("listen"), None, "links can't listen");
    }

    #[test]
    fn long_lines_are_refused() {
        let long = vec![b'x'; MAX_LINE as usize + 10];
        assert!(read_line(&long[..]).is_err());
        assert_eq!(read_line(&b""[..]).unwrap(), None);
        assert_eq!(read_line(&b"a\nb"[..]).unwrap().as_deref(), Some("a\n"));
    }

    #[cfg(unix)]
    #[test]
    fn round_trip_over_a_socket() {
        use std::os::unix::net::UnixStream;
        let (client, server) = UnixStream::pair().unwrap();
        let t = std::thread::spawn(move || {
            server::handle_connection(server, &|r: Request| match r {
                Request::Listen { prompt, .. } => Reply::from(Ok(prompt)),
                _ => Reply::error("Not recording"),
            });
        });
        let reply = exchange(
            &client,
            &Request::Listen {
                prompt: Some("echo".into()),
                to: None,
            },
        )
        .unwrap();
        assert_eq!(reply.into_result(), Ok(Some("echo".into())));
        t.join().unwrap();

        // The peer check passes for ourselves.
        let (a, _b) = UnixStream::pair().unwrap();
        assert!(server::same_user(&a));
    }
}
