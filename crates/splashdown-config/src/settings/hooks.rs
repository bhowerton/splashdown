//! `[[hooks]]` (Settings → Actions → When something happens): a script or
//! program to run, or a web address to send to, when something happens in
//! Splashdown. The app runs them (`app/src-tauri/src/hooks.rs`); what each
//! one gets is in docs/HOOKS.md.

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
        }
    }

    /// It comes from a dictation, so a preset can be chosen for it.
    pub fn has_preset(self) -> bool {
        matches!(self, HookEvent::Dictation | HookEvent::BeforeInsert)
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
    /// Off keeps it without running it.
    #[serde(default = "on", skip_serializing_if = "is_on")]
    pub on: bool,
    /// Runs without asking first: the user allowed it, or ran its Test.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trusted: bool,
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

    /// What's wrong with it before it's saved, if anything.
    pub fn problem(&self) -> Option<String> {
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
                let url = url.trim();
                let rest = url
                    .strip_prefix("https://")
                    .or_else(|| url.strip_prefix("http://"));
                if !rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/')) {
                    return Some("Give a web address starting with https://.".into());
                }
                if url.chars().any(char::is_whitespace) {
                    return Some("A web address can't have spaces.".into());
                }
            }
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
            on: true,
            trusted: false,
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
"#;
        let settings = Settings::from_toml(text).unwrap();
        assert_eq!(settings.hooks.len(), 2);
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
