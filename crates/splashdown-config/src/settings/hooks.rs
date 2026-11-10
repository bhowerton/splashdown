//! `[[hooks]]` (Settings → Actions → When something happens): a script or
//! program to run, a web address to send to, or something to ask the AI,
//! when something happens in Splashdown. The app runs them
//! (`app/src-tauri/src/hooks.rs`); what each one gets is in docs/HOOKS.md.

use super::*;

/// When a hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookEvent {
    /// A dictation went into a text field.
    Dictation,
    /// A dictation is about to go into a text field: the program's output
    /// is inserted instead. A program, for one preset.
    BeforeInsert,
    /// A meeting's note is finished, with its summary.
    Meeting,
    /// A transcript note is finished: a file, a link, a watched folder, an
    /// app's audio or the microphone.
    Transcript,
    /// A note said on the iPhone was added.
    PhoneNote,
    /// A time of day came round (the hook's `schedule`): it gets the notes
    /// made since.
    Schedule,
    /// A calendar event is about to start (the hook's `before` says how
    /// many minutes ahead): it gets the event and the earlier notes about
    /// it. The calendar is read on a Mac only.
    BeforeMeeting,
}

impl HookEvent {
    /// The event's name in a webhook and in `SPLASHDOWN_EVENT`.
    pub fn name(self) -> &'static str {
        match self {
            HookEvent::Dictation => "dictation",
            HookEvent::BeforeInsert => "before-insert",
            HookEvent::Meeting => "meeting",
            HookEvent::Transcript => "transcript",
            HookEvent::PhoneNote => "phone-note",
            HookEvent::Schedule => "schedule",
            HookEvent::BeforeMeeting => "before-meeting",
        }
    }

    /// It comes from a dictation, so a preset can be chosen for it.
    pub fn has_preset(self) -> bool {
        matches!(self, HookEvent::Dictation | HookEvent::BeforeInsert)
    }

    /// It's about a note with a title of its own.
    pub fn has_note(self) -> bool {
        matches!(self, HookEvent::Meeting | HookEvent::Transcript)
    }

    /// It's about something with a title to go by: a note, or a calendar
    /// event.
    pub fn has_title(self) -> bool {
        self.has_note() || self == HookEvent::BeforeMeeting
    }
}

/// What a hook runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HookTarget {
    /// A script or program, given the text on its standard input and the
    /// details in environment variables, with these arguments, never
    /// through a shell.
    Program {
        path: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    /// A web address the details are posted to as JSON, signed.
    Web { url: String },
    /// Something the AI is asked to do with the text, in the user's own
    /// words ("draft a follow-up email"). What it writes is put somewhere;
    /// it can't do anything else.
    Ask {
        /// What it's called: in the list, in what ran, and as the heading
        /// its result goes under in a note.
        name: String,
        instruction: String,
        /// Where the result goes.
        put: PutIn,
        /// The note, file or web address for `put`, when it takes one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
}

/// Where what the AI wrote goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PutIn {
    /// A section of the meeting's or transcript's own note, under the
    /// hook's name.
    Note,
    /// The end of another note in the notes folder (`to`, made if it isn't
    /// there), under a heading with the date.
    OtherNote,
    /// The end of a file anywhere (`to`, a full path).
    File,
    Clipboard,
    /// A web address (`to`), as a webhook gets it, with the result.
    Web,
}

impl PutIn {
    /// It needs `to`.
    pub fn takes_a_place(self) -> bool {
        matches!(self, PutIn::OtherNote | PutIn::File | PutIn::Web)
    }
}

/// When a scheduled hook runs, and how far back the notes it gets go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// The time of day on this computer's clock, as `16:00`.
    pub at: String,
    /// The days it runs on; none is every day.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<Day>,
    #[serde(default, skip_serializing_if = "Covers::is_a_day")]
    pub covers: Covers,
}

impl Schedule {
    /// `at` as an hour and a minute, if it's a time of day.
    pub fn time(&self) -> Option<(u32, u32)> {
        let (hour, minute) = self.at.trim().split_once(':')?;
        let digits =
            |s: &str| !s.is_empty() && s.len() <= 2 && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(hour) || minute.len() != 2 || !digits(minute) {
            return None;
        }
        let (hour, minute) = (hour.parse().ok()?, minute.parse().ok()?);
        (hour < 24 && minute < 60).then_some((hour, minute))
    }

    /// Whether it runs on the day that's `from_monday` days after a Monday.
    pub fn runs_on(&self, from_monday: u32) -> bool {
        self.days.is_empty() || self.days.iter().any(|d| d.from_monday() == from_monday)
    }
}

/// A day of the week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Day {
    /// How many days after Monday it is.
    pub fn from_monday(self) -> u32 {
        self as u32
    }

    pub fn name(self) -> &'static str {
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ][self as usize]
    }
}

/// How far back the notes a scheduled hook gets go, from the time it was
/// due.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Covers {
    /// The 24 hours before.
    #[default]
    Day,
    /// The 7 days before.
    Week,
}

impl Covers {
    pub fn days(self) -> u32 {
        match self {
            Covers::Day => 1,
            Covers::Week => 7,
        }
    }

    fn is_a_day(&self) -> bool {
        *self == Covers::Day
    }

    /// "the last day", "the last 7 days".
    pub fn words(self) -> &'static str {
        match self {
            Covers::Day => "the last day",
            Covers::Week => "the last 7 days",
        }
    }
}

/// How long before a calendar event starts its hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Before {
    #[serde(default = "Before::usual")]
    pub minutes: u32,
}

impl Before {
    /// The most minutes ahead a hook can run.
    pub const MOST: u32 = 120;

    fn usual() -> u32 {
        10
    }
}

impl Default for Before {
    fn default() -> Self {
        Before {
            minutes: Before::usual(),
        }
    }
}

/// One hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hook {
    pub when: HookEvent,
    #[serde(flatten)]
    pub run: HookTarget,
    /// Only dictations with this preset (its id); `None` is every preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Only meetings, transcripts and calendar events whose title has this
    /// in it, in any capitals ("1:1", "Acme"); `None` is all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_has: Option<String>,
    /// Off keeps it without running it.
    #[serde(default = "on", skip_serializing_if = "is_on")]
    pub on: bool,
    /// Runs without asking first: the user allowed it, or ran its Test.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trusted: bool,
    /// When it runs and what it reads, for a hook on a schedule. Last, as
    /// TOML wants a table after plain values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Schedule>,
    /// How long before the event it runs, for a hook before a meeting;
    /// `None` is 10 minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Before>,
}

fn on() -> bool {
    true
}

fn is_on(on: &bool) -> bool {
    *on
}

impl Hook {
    /// It runs for `event` in a dictation with `preset` (`None` outside
    /// dictations).
    pub fn applies(&self, event: HookEvent, preset: Option<&str>) -> bool {
        self.on
            && self.when == event
            && match (&self.preset, preset) {
                (None, _) => true,
                (Some(wanted), Some(preset)) => wanted == preset,
                (Some(_), None) => !event.has_preset(),
            }
    }

    /// `applies`, and the note's title (when the event has one) has what
    /// the hook asks for.
    pub fn applies_to(&self, event: HookEvent, preset: Option<&str>, title: Option<&str>) -> bool {
        if !self.applies(event, preset) {
            return false;
        }
        match self.title_has.as_deref().map(str::trim) {
            Some(wanted) if !wanted.is_empty() && event.has_title() => {
                title.is_some_and(|t| t.to_lowercase().contains(&wanted.to_lowercase()))
            }
            _ => true,
        }
    }

    /// How many minutes before a calendar event it runs.
    pub fn minutes_before(&self) -> u32 {
        self.before.unwrap_or_default().minutes
    }

    /// What's wrong with it before it's saved, if anything.
    pub fn problem(&self) -> Option<String> {
        if self.when == HookEvent::BeforeMeeting {
            if !cfg!(target_os = "macos") {
                return Some(
                    "A hook before a meeting reads your calendar, which Splashdown can only do on a Mac for now."
                        .into(),
                );
            }
            if !(1..=Before::MOST).contains(&self.minutes_before()) {
                return Some(format!(
                    "Choose how many minutes before the meeting it runs, from 1 to {}.",
                    Before::MOST
                ));
            }
        }
        if self.when == HookEvent::Schedule
            && self.schedule.as_ref().and_then(Schedule::time).is_none()
        {
            return Some("Choose the time it runs, such as 16:00.".into());
        }
        match &self.run {
            HookTarget::Program { path, .. } => {
                let path = path.trim();
                if path.is_empty() {
                    return Some("Choose a script or program to run.".into());
                }
                if !Path::new(path).is_absolute() {
                    return Some("Give the full path to the script or program.".into());
                }
            }
            HookTarget::Web { url } => {
                if let Some(problem) = web_problem(url.trim()) {
                    return Some(problem);
                }
            }
            HookTarget::Ask {
                name,
                instruction,
                put,
                to,
            } => {
                if self.when == HookEvent::BeforeInsert {
                    return Some(
                        "Changing the text needs a script or program: the AI would make every dictation wait. A preset's own instructions change its dictations."
                            .into(),
                    );
                }
                let name = name.trim();
                if name.is_empty() {
                    return Some("Give it a name.".into());
                }
                if name.contains(['\n', '\r']) || name.starts_with('#') {
                    return Some("The name is one line, without a # at the start.".into());
                }
                if instruction.trim().is_empty() {
                    return Some("Say what the AI should do with the text.".into());
                }
                let to = to.as_deref().map(str::trim).unwrap_or_default();
                match put {
                    PutIn::Note if !self.when.has_note() => {
                        return Some(
                            "Only a meeting or a transcript has a note of its own: choose another place for the result."
                                .into(),
                        );
                    }
                    PutIn::OtherNote => {
                        if to.is_empty() {
                            return Some("Name the note the result is added to.".into());
                        }
                        if Path::new(to).is_absolute()
                            || to.starts_with(['/', '\\'])
                            || to.split(['/', '\\']).any(|part| part == "..")
                        {
                            return Some(
                                "Name a note inside your notes folder, such as Commitments or Work/Decisions."
                                    .into(),
                            );
                        }
                    }
                    PutIn::File => {
                        if to.is_empty() {
                            return Some("Choose the file the result is added to.".into());
                        }
                        if !Path::new(to).is_absolute() {
                            return Some("Give the full path to the file.".into());
                        }
                    }
                    PutIn::Web => {
                        if let Some(problem) = web_problem(to) {
                            return Some(problem);
                        }
                    }
                    PutIn::Note | PutIn::Clipboard => {}
                }
            }
        }
        if self
            .title_has
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
            && !self.when.has_title()
        {
            return Some(
                "Only meetings, transcripts and calendar events have a title to go by.".into(),
            );
        }
        if self.when == HookEvent::BeforeInsert {
            if !matches!(self.run, HookTarget::Program { .. }) {
                return Some(
                    "Changing the text needs a script or program: a web address would make every dictation wait on the network."
                        .into(),
                );
            }
            if self.preset.is_none() {
                return Some("Choose the preset whose dictations it changes.".into());
            }
        }
        None
    }

    /// The same hook with what it runs: a change to that means asking again.
    pub fn runs_same_as(&self, other: &Hook) -> bool {
        self.run == other.run && self.when == other.when
    }
}

/// What's wrong with a web address to send to, if anything.
fn web_problem(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    if !rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/')) {
        return Some("Give a web address starting with https://.".into());
    }
    if url.chars().any(char::is_whitespace) {
        return Some("A web address can't have spaces.".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(when: HookEvent, preset: Option<&str>) -> Hook {
        Hook {
            when,
            run: HookTarget::Program {
                // A full path on this system.
                path: if cfg!(windows) {
                    r"C:\Tools\log-it.exe"
                } else {
                    "/usr/local/bin/log-it"
                }
                .into(),
                args: vec![],
            },
            preset: preset.map(str::to_string),
            title_has: None,
            on: true,
            trusted: false,
            schedule: None,
            before: None,
        }
    }

    fn ask(when: HookEvent, put: PutIn, to: Option<&str>) -> Hook {
        Hook {
            run: HookTarget::Ask {
                name: "Follow-up email".into(),
                instruction: "Draft a follow-up email to the people in this meeting.".into(),
                put,
                to: to.map(str::to_string),
            },
            ..program(when, None)
        }
    }

    #[test]
    fn a_hook_runs_for_its_event_and_preset() {
        let all = program(HookEvent::Dictation, None);
        assert!(all.applies(HookEvent::Dictation, Some("email")));
        assert!(!all.applies(HookEvent::Meeting, None));
        let email = program(HookEvent::Dictation, Some("email"));
        assert!(email.applies(HookEvent::Dictation, Some("email")));
        assert!(!email.applies(HookEvent::Dictation, Some("default")));
        let off = Hook {
            on: false,
            ..all.clone()
        };
        assert!(!off.applies(HookEvent::Dictation, Some("email")));
    }

    #[test]
    fn hooks_are_checked_before_saving() {
        assert_eq!(program(HookEvent::Meeting, None).problem(), None);
        let relative = Hook {
            run: HookTarget::Program {
                path: "log-it".into(),
                args: vec![],
            },
            ..program(HookEvent::Meeting, None)
        };
        assert!(relative.problem().is_some());
        let web = |url: &str| Hook {
            run: HookTarget::Web { url: url.into() },
            ..program(HookEvent::Meeting, None)
        };
        assert_eq!(
            web("https://hooks.zapier.com/hooks/catch/1/abc").problem(),
            None
        );
        assert_eq!(web("http://localhost:5678/webhook/x").problem(), None);
        assert!(web("hooks.zapier.com").problem().is_some());
        assert!(web("https://").problem().is_some());
        assert!(web("ftp://x").problem().is_some());
        // Changing the text: a program, for one preset.
        assert!(program(HookEvent::BeforeInsert, None).problem().is_some());
        assert_eq!(
            program(HookEvent::BeforeInsert, Some("code")).problem(),
            None
        );
        let web_before = Hook {
            when: HookEvent::BeforeInsert,
            preset: Some("code".into()),
            ..web("https://example.com/x")
        };
        assert!(web_before.problem().is_some());
    }

    #[test]
    fn a_hook_can_go_by_the_note_s_title() {
        let one_on_ones = Hook {
            title_has: Some("1:1".into()),
            ..ask(HookEvent::Meeting, PutIn::Note, None)
        };
        assert!(one_on_ones.applies_to(HookEvent::Meeting, None, Some("Dana 1:1")));
        assert!(!one_on_ones.applies_to(HookEvent::Meeting, None, Some("Weekly sync")));
        assert!(!one_on_ones.applies_to(HookEvent::Meeting, None, None));
        assert!(!one_on_ones.applies_to(HookEvent::Transcript, None, Some("Dana 1:1")));
        let acme = Hook {
            title_has: Some(" acme ".into()),
            ..ask(HookEvent::Meeting, PutIn::Note, None)
        };
        assert!(acme.applies_to(HookEvent::Meeting, None, Some("ACME kickoff")));
        let all = ask(HookEvent::Meeting, PutIn::Note, None);
        assert!(all.applies_to(HookEvent::Meeting, None, None));
        // Nothing else has a title to go by.
        let dictation = Hook {
            title_has: Some("1:1".into()),
            ..program(HookEvent::Dictation, None)
        };
        assert!(dictation.problem().is_some());
    }

    #[test]
    fn asking_the_ai_is_checked_before_saving() {
        assert_eq!(ask(HookEvent::Meeting, PutIn::Note, None).problem(), None);
        assert_eq!(
            ask(HookEvent::Transcript, PutIn::Clipboard, None).problem(),
            None
        );
        // Only a meeting or transcript has a note of its own.
        assert!(
            ask(HookEvent::PhoneNote, PutIn::Note, None)
                .problem()
                .is_some()
        );
        assert_eq!(
            ask(HookEvent::PhoneNote, PutIn::OtherNote, Some("To-dos")).problem(),
            None
        );
        assert_eq!(
            ask(
                HookEvent::Meeting,
                PutIn::OtherNote,
                Some("Work/Decisions.md")
            )
            .problem(),
            None
        );
        for outside in ["", "../Elsewhere", "/etc/passwd", "Work/../../x"] {
            assert!(
                ask(HookEvent::Meeting, PutIn::OtherNote, Some(outside))
                    .problem()
                    .is_some(),
                "{outside}"
            );
        }
        assert!(
            ask(HookEvent::Meeting, PutIn::File, Some("log.md"))
                .problem()
                .is_some()
        );
        let file = if cfg!(windows) {
            r"C:\Notes\log.md"
        } else {
            "/Users/me/log.md"
        };
        assert_eq!(
            ask(HookEvent::Meeting, PutIn::File, Some(file)).problem(),
            None
        );
        assert!(
            ask(HookEvent::Meeting, PutIn::Web, Some("example.com"))
                .problem()
                .is_some()
        );
        assert_eq!(
            ask(
                HookEvent::Meeting,
                PutIn::Web,
                Some("https://example.com/x")
            )
            .problem(),
            None
        );
        // A dictation doesn't wait for the AI.
        let before = Hook {
            preset: Some("code".into()),
            ..ask(HookEvent::BeforeInsert, PutIn::Clipboard, None)
        };
        assert!(before.problem().is_some());
        let unnamed = Hook {
            run: HookTarget::Ask {
                name: " ".into(),
                instruction: "Summarize.".into(),
                put: PutIn::Clipboard,
                to: None,
            },
            ..program(HookEvent::Meeting, None)
        };
        assert!(unnamed.problem().is_some());
    }

    #[test]
    fn a_scheduled_hook_needs_a_time() {
        let at = |at: &str, days: &[Day]| Hook {
            schedule: Some(Schedule {
                at: at.into(),
                days: days.to_vec(),
                covers: Covers::Week,
            }),
            ..ask(
                HookEvent::Schedule,
                PutIn::OtherNote,
                Some("Weekly reviews"),
            )
        };
        assert_eq!(at("16:00", &[Day::Fri]).problem(), None);
        assert_eq!(at(" 7:05 ", &[]).problem(), None);
        for wrong in ["", "4pm", "24:00", "16:60", "16", "16:5", "-1:00", "1:2:3"] {
            assert!(at(wrong, &[]).problem().is_some(), "{wrong}");
        }
        // Without a schedule at all.
        assert!(
            ask(HookEvent::Schedule, PutIn::Clipboard, None)
                .problem()
                .is_some()
        );
        // It has no note of its own to put the result in.
        let in_note = Hook {
            schedule: at("16:00", &[]).schedule,
            ..ask(HookEvent::Schedule, PutIn::Note, None)
        };
        assert!(in_note.problem().is_some());
        let fridays = at("16:00", &[Day::Fri]).schedule.unwrap();
        assert!(fridays.runs_on(4) && !fridays.runs_on(0));
        assert!(at("16:00", &[]).schedule.unwrap().runs_on(6));
        assert_eq!((Day::Mon.from_monday(), Day::Sun.from_monday()), (0, 6));
        assert_eq!(Day::Wed.name(), "Wednesday");
    }

    #[test]
    fn a_hook_before_a_meeting_says_how_long_before() {
        let brief = |minutes: Option<u32>| Hook {
            before: minutes.map(|minutes| Before { minutes }),
            ..ask(HookEvent::BeforeMeeting, PutIn::OtherNote, Some("Briefs"))
        };
        assert_eq!(brief(None).minutes_before(), 10);
        assert_eq!(brief(Some(5)).minutes_before(), 5);
        if cfg!(target_os = "macos") {
            assert_eq!(brief(None).problem(), None);
            assert_eq!(brief(Some(1)).problem(), None);
            assert_eq!(brief(Some(120)).problem(), None);
            for wrong in [0, 121, 10_000] {
                let problem = brief(Some(wrong)).problem();
                assert!(problem.is_some_and(|p| p.contains("minutes")), "{wrong}");
            }
            // An event has no note of its own to put the result in.
            let in_note = ask(HookEvent::BeforeMeeting, PutIn::Note, None);
            assert!(in_note.problem().is_some());
        } else {
            // The calendar is only read on a Mac.
            assert!(brief(None).problem().is_some_and(|p| p.contains("Mac")));
        }
        // It can go by the event's title, as a meeting's hook does.
        let one_on_ones = Hook {
            title_has: Some("1:1".into()),
            ..brief(None)
        };
        assert!(one_on_ones.applies_to(HookEvent::BeforeMeeting, None, Some("Dana 1:1")));
        assert!(!one_on_ones.applies_to(HookEvent::BeforeMeeting, None, Some("Weekly sync")));
        assert!(!one_on_ones.applies_to(HookEvent::Meeting, None, Some("Dana 1:1")));
        assert!(brief(None).applies_to(HookEvent::BeforeMeeting, None, Some("Weekly sync")));
        if cfg!(target_os = "macos") {
            assert_eq!(one_on_ones.problem(), None);
        }
        assert!(HookEvent::BeforeMeeting.has_title() && !HookEvent::BeforeMeeting.has_note());
        assert!(!HookEvent::Schedule.has_title());
    }

    #[test]
    fn hooks_read_and_write_as_toml() {
        let text = r#"
[[hooks]]
when = "meeting"
kind = "web"
url = "https://hooks.zapier.com/hooks/catch/1/abc"

[[hooks]]
when = "before-insert"
kind = "program"
path = "/Users/me/bin/fix-names"
args = ["--strict"]
preset = "code"
trusted = true

[[hooks]]
when = "meeting"
kind = "ask"
name = "My commitments"
instruction = "List everything I said I'd do."
put = "other-note"
to = "Commitments"
title_has = "1:1"

[[hooks]]
when = "schedule"
kind = "ask"
name = "Weekly review"
instruction = "Write a review of my week."
put = "other-note"
to = "Weekly reviews"
schedule = { at = "16:00", days = ["fri"], covers = "week" }

[[hooks]]
when = "schedule"
kind = "program"
path = "/Users/me/bin/daily-log"
schedule = { at = "8:30" }

[[hooks]]
when = "before-meeting"
kind = "ask"
name = "Brief"
instruction = "Write a brief for this meeting."
put = "other-note"
to = "Briefs"
title_has = "1:1"
before = { minutes = 5 }

[[hooks]]
when = "before-meeting"
kind = "program"
path = "/Users/me/bin/brief"
"#;
        let settings = Settings::from_toml(text).unwrap();
        assert_eq!(settings.hooks.len(), 7);
        assert_eq!(settings.hooks[5].when, HookEvent::BeforeMeeting);
        assert_eq!(settings.hooks[5].before, Some(Before { minutes: 5 }));
        assert_eq!(settings.hooks[5].title_has.as_deref(), Some("1:1"));
        // Without the table: ten minutes before.
        assert_eq!(settings.hooks[6].before, None);
        assert_eq!(settings.hooks[6].minutes_before(), 10);
        assert_eq!(
            Settings::from_toml("[[hooks]]\nwhen = \"before-meeting\"\nkind = \"web\"\nurl = \"https://example.com/x\"\nbefore = {}\n")
                .unwrap()
                .hooks[0]
                .minutes_before(),
            10
        );
        assert_eq!(
            settings.hooks[3].schedule,
            Some(Schedule {
                at: "16:00".into(),
                days: vec![Day::Fri],
                covers: Covers::Week,
            })
        );
        assert_eq!(settings.hooks[3].problem(), None);
        let daily = settings.hooks[4].schedule.clone().unwrap();
        assert_eq!((daily.time(), daily.covers), (Some((8, 30)), Covers::Day));
        assert!(daily.days.is_empty());
        assert_eq!(
            settings.hooks[2].run,
            HookTarget::Ask {
                name: "My commitments".into(),
                instruction: "List everything I said I'd do.".into(),
                put: PutIn::OtherNote,
                to: Some("Commitments".into()),
            }
        );
        assert_eq!(settings.hooks[2].title_has.as_deref(), Some("1:1"));
        assert_eq!(settings.hooks[2].problem(), None);
        assert_eq!(settings.hooks[0].when, HookEvent::Meeting);
        assert!(settings.hooks[0].on);
        assert!(!settings.hooks[0].trusted);
        assert_eq!(
            settings.hooks[1].run,
            HookTarget::Program {
                path: "/Users/me/bin/fix-names".into(),
                args: vec!["--strict".into()]
            }
        );
        let again = Settings::from_toml(&toml::to_string(&settings).unwrap()).unwrap();
        assert_eq!(again.hooks, settings.hooks);
    }
}
