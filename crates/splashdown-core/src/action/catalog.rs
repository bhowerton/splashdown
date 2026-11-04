//! The fixed catalog of voice actions: each action's parameters (`Spec`),
//! the sites and media it may use, and `Action`, a planned action with its
//! values.

/// What kind of value a parameter takes.
#[derive(Debug, Clone, Copy)]
pub(super) enum Kind {
    Text,
    /// One of these words.
    OneOf(&'static [&'static str]),
    /// A local date and time, `YYYY-MM-DDTHH:MM`.
    Time,
    /// A whole number in this range.
    Number(i64, i64),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Param {
    pub(super) name: &'static str,
    pub(super) kind: Kind,
    pub(super) required: bool,
    pub(super) about: &'static str,
}

/// One action in the catalog.
#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub id: &'static str,
    pub(super) about: &'static str,
    pub(super) params: &'static [Param],
}

pub(super) const fn p(name: &'static str, kind: Kind, about: &'static str) -> Param {
    Param {
        name,
        kind,
        required: true,
        about,
    }
}

pub(super) const fn opt(name: &'static str, kind: Kind, about: &'static str) -> Param {
    Param {
        name,
        kind,
        required: false,
        about,
    }
}

pub const SITES: &[&str] = &[
    "web",
    "images",
    "news",
    "maps",
    "youtube",
    "wikipedia",
    "amazon",
];

/// Splashdown's own actions.
pub const OWN: &[&str] = &[
    "start_meeting",
    "stop_meeting",
    "new_note",
    "open_notes",
    "todays_note",
    "switch_preset",
    "whisper_mode_on",
    "whisper_mode_off",
    "hands_free",
    "retry_last_dictation",
    "open_settings",
    "show_commands",
];

pub const MEDIA: &[&str] = &[
    "play",
    "pause",
    "next_track",
    "previous_track",
    "mute",
    "unmute",
    "volume_up",
    "volume_down",
    "set_volume",
];

/// Every built-in action, in the order the model sees them.
pub const CATALOG: &[Spec] = &[
    Spec {
        id: "open_app",
        about: "open an app",
        params: &[p(
            "app",
            Kind::Text,
            "the app's name as it's usually written (\"Spotify\", \"Microsoft Word\")",
        )],
    },
    Spec {
        id: "switch_to_app",
        about: "bring an app that's already open to the front (\"go back to Slack\", \"switch to Chrome\")",
        params: &[p("app", Kind::Text, "the app's name")],
    },
    Spec {
        id: "search_web",
        about: "search the web, or a site with a search of its own",
        params: &[
            p(
                "query",
                Kind::Text,
                "what to search for, in the language it was said",
            ),
            opt(
                "site",
                Kind::OneOf(SITES),
                "the site named, when it's one of these (\"on Amazon\" is amazon, \"news about\" is news); web otherwise. The query leaves the site out",
            ),
        ],
    },
    Spec {
        id: "open_website",
        about: "open a website by name (\"open GitHub\", \"go to the BBC\")",
        params: &[p(
            "site",
            Kind::Text,
            "the site's name or domain as said (\"GitHub\", \"bbc.co.uk\")",
        )],
    },
    Spec {
        id: "splashdown",
        about: "something the dictation app itself does: start or stop recording a meeting, a new note, open Notes, today's note, switch to a preset (a saved style such as Email or Code), whisper mode on or off, hands-free dictation, retry the last dictation, open Settings, show what the user can say (\"what can I say?\", \"help\"). open_notes is the dictation app's own notes; an app called Notes is open_app",
        params: &[
            p("what", Kind::OneOf(OWN), "which one"),
            opt("preset", Kind::Text, "for switch_preset: the preset's name"),
        ],
    },
    Spec {
        id: "add_todo",
        about: "add a to-do to the user's notes (a task with no time; with a time it's a reminder)",
        params: &[p(
            "text",
            Kind::Text,
            "the to-do, short, in the language it was said",
        )],
    },
    Spec {
        id: "media",
        about: "control music, video or the sound: play, pause, next, previous, mute, volume",
        params: &[
            p(
                "what",
                Kind::OneOf(MEDIA),
                "which control (skipping a song is next_track; sound off is mute)",
            ),
            opt(
                "level",
                Kind::Number(0, 100),
                "for set_volume: the volume in percent",
            ),
        ],
    },
    Spec {
        id: "create_event",
        about: "add an event to the calendar (a meeting, appointment or plan at a time)",
        params: &[
            p(
                "title",
                Kind::Text,
                "short title, in the language it was said",
            ),
            p("start", Kind::Time, "when it starts"),
            opt("end", Kind::Time, "when it ends, if said"),
            opt("location", Kind::Text, "where, if said"),
        ],
    },
    Spec {
        id: "create_reminder",
        about: "a reminder for the user (\"remind me to…\"), with a time if one was said",
        params: &[
            p(
                "text",
                Kind::Text,
                "what to be reminded of, without \"remind me to\"",
            ),
            opt("due", Kind::Time, "when, if said"),
        ],
    },
    Spec {
        id: "draft_email",
        about: "open a new email in the mail app for the user to check and send; it's never sent for them. Only for email: a text, chat or WhatsApp message is none",
        params: &[
            opt("to", Kind::Text, "who to, as said (a name or an address)"),
            opt("subject", Kind::Text, "a short subject"),
            p(
                "body",
                Kind::Text,
                "the email's text, written out in full, in the language it was said",
            ),
        ],
    },
    Spec {
        id: "run_shortcut",
        about: "run one of the user's Shortcuts by name",
        params: &[
            p(
                "name",
                Kind::Text,
                "the Shortcut's name, from the list when there is one",
            ),
            opt("input", Kind::Text, "text to give the Shortcut, if any"),
        ],
    },
    Spec {
        id: "my_action",
        about: "run one of the actions the user set up themselves, named in the request (the list follows)",
        params: &[
            p("name", Kind::Text, "the action's name, from the list"),
            opt(
                "text",
                Kind::Text,
                "for an action that takes text: the text, as said",
            ),
        ],
    },
];

/// A planned action, checked and ready to carry out. Times are local,
/// `YYYY-MM-DDTHH:MM`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    OpenApp {
        app: String,
    },
    SwitchToApp {
        app: String,
    },
    SearchWeb {
        query: String,
        site: String,
    },
    OpenWebsite {
        site: String,
    },
    Splashdown {
        what: String,
        preset: Option<String>,
    },
    AddTodo {
        text: String,
    },
    Media {
        what: String,
        level: Option<u8>,
    },
    CreateEvent {
        title: String,
        start: String,
        end: Option<String>,
        location: Option<String>,
    },
    CreateReminder {
        text: String,
        due: Option<String>,
    },
    DraftEmail {
        to: Option<String>,
        subject: Option<String>,
        body: String,
    },
    RunShortcut {
        name: String,
        input: Option<String>,
    },
    /// One of the user's own actions, by its name (as the user wrote it).
    MyAction {
        name: String,
        text: Option<String>,
    },
}

impl Action {
    /// The catalog id.
    pub fn id(&self) -> &'static str {
        match self {
            Action::OpenApp { .. } => "open_app",
            Action::SwitchToApp { .. } => "switch_to_app",
            Action::SearchWeb { .. } => "search_web",
            Action::OpenWebsite { .. } => "open_website",
            Action::Splashdown { .. } => "splashdown",
            Action::AddTodo { .. } => "add_todo",
            Action::Media { .. } => "media",
            Action::CreateEvent { .. } => "create_event",
            Action::CreateReminder { .. } => "create_reminder",
            Action::DraftEmail { .. } => "draft_email",
            Action::RunShortcut { .. } => "run_shortcut",
            Action::MyAction { .. } => "my_action",
        }
    }

    /// The parameters that were given, by name.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        let mut add = |name: &'static str, value: Option<&String>| {
            if let Some(v) = value {
                out.push((name, v.clone()));
            }
        };
        match self {
            Action::OpenApp { app } | Action::SwitchToApp { app } => add("app", Some(app)),
            Action::SearchWeb { query, site } => {
                add("query", Some(query));
                add("site", Some(site));
            }
            Action::OpenWebsite { site } => add("site", Some(site)),
            Action::Splashdown { what, preset } => {
                add("what", Some(what));
                add("preset", preset.as_ref());
            }
            Action::AddTodo { text } => add("text", Some(text)),
            Action::Media { what, level } => {
                add("what", Some(what));
                add("level", level.map(|l| l.to_string()).as_ref());
            }
            Action::CreateEvent {
                title,
                start,
                end,
                location,
            } => {
                add("title", Some(title));
                add("start", Some(start));
                add("end", end.as_ref());
                add("location", location.as_ref());
            }
            Action::CreateReminder { text, due } => {
                add("text", Some(text));
                add("due", due.as_ref());
            }
            Action::DraftEmail { to, subject, body } => {
                add("to", to.as_ref());
                add("subject", subject.as_ref());
                add("body", Some(body));
            }
            Action::RunShortcut { name, input } => {
                add("name", Some(name));
                add("input", input.as_ref());
            }
            Action::MyAction { name, text } => {
                add("name", Some(name));
                add("text", text.as_ref());
            }
        }
        out
    }
}

pub(super) fn enabled_specs(enabled: Option<&[&str]>) -> Vec<&'static Spec> {
    CATALOG
        .iter()
        .filter(|s| enabled.is_none_or(|e| e.contains(&s.id)))
        .collect()
}
