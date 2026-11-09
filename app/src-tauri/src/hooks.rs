//! Hooks (Settings → Actions → When something happens): when a dictation
//! goes into a field, a meeting or transcript is finished, or a note comes
//! from the iPhone, run the user's script or program with the text, or send
//! it to a web address as signed JSON. One kind runs before a dictation is
//! inserted and changes its text. What each gets is in docs/HOOKS.md.
//!
//! Hooks are only ever added in Settings, where saving one says exactly
//! what it will run; one found in `config.toml` without that (written by
//! hand, or by something else) asks before its first run. Programs are
//! started directly, never through a shell, with the text on their standard
//! input. Hooks after the fact run one at a time on a thread of their own,
//! so nothing waits for them.

use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::Serialize;
use splashdown_config::{Hook, HookEvent, HookTarget, Settings};
use splashdown_platform::{OverlayState, Tone};
use tauri::{AppHandle, Manager};

use crate::programs::Ended;
use crate::state::AppState;
use crate::ui::Ui;
use splashdown_core::sync::Locked;

/// How long a hook after the fact may run before it's stopped.
const AFTER_LIMIT: Duration = Duration::from_secs(60);
/// How long a dictation waits for a hook that changes its text. Past
/// that it's inserted as it was.
pub const BEFORE_LIMIT: Duration = Duration::from_secs(2);
/// How long a web address has to answer.
const WEB_LIMIT: Duration = Duration::from_secs(15);
/// How long a hook's failure shows.
const WARNING_FOR: Duration = Duration::from_secs(4);
/// What each hook gets, and examples.
const DOCS: &str = "https://github.com/bhowerton/splashdown/blob/main/docs/HOOKS.md";
/// The file in the settings folder that holds the webhook secret.
const SECRET_FILE: &str = "webhook-secret";

static UI: OnceLock<Ui> = OnceLock::new();
static QUEUE: OnceLock<Mutex<Sender<(Hook, Happened)>>> = OnceLock::new();

/// Called once at start-up with the status pill, to tell of failures.
pub fn init(ui: Ui) {
    let _ = UI.set(ui);
}

// ---------------------------------------------------------------- what happened

/// What happened, as a hook gets it.
#[derive(Debug, Clone, Serialize)]
pub struct Happened {
    pub event: &'static str,
    /// When it happened (said, for a note from the iPhone).
    pub at: String,
    /// The dictation, the note from the iPhone, or the whole note
    /// (Markdown) for a meeting or transcript.
    pub text: String,
    /// The app dictated into.
    pub app: Option<String>,
    /// The dictation's preset, by name.
    pub preset: Option<String>,
    pub note: Option<NoteRef>,
    /// Sent by Test in Settings.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub test: bool,
    /// The preset's id, to choose the hooks.
    #[serde(skip)]
    preset_id: Option<String>,
}

/// The note a meeting or transcript went into.
#[derive(Debug, Clone, Serialize)]
pub struct NoteRef {
    pub title: String,
    /// The note's file.
    pub path: PathBuf,
    /// Its summary, when it has one.
    pub summary: Option<String>,
}

impl Happened {
    fn new(event: HookEvent, text: String, at: DateTime<Local>) -> Self {
        Happened {
            event: event.name(),
            at: at.to_rfc3339(),
            text,
            app: None,
            preset: None,
            note: None,
            test: false,
            preset_id: None,
        }
    }

    /// A dictation into `app` with the preset `(id, name)`.
    pub fn dictation(
        event: HookEvent,
        text: &str,
        app: Option<&str>,
        preset: (&str, &str),
    ) -> Self {
        Happened {
            app: app.map(str::to_string),
            preset: Some(preset.1.to_string()),
            preset_id: Some(preset.0.to_string()),
            ..Happened::new(event, text.to_string(), Local::now())
        }
    }

    /// A note said on the iPhone at `at`.
    pub fn phone(text: &str, at: DateTime<Local>) -> Self {
        Happened::new(HookEvent::PhoneNote, text.to_string(), at)
    }

    /// The note at `rel` in the notes folder, finished.
    fn note(settings: &Settings, event: HookEvent, rel: &str) -> Result<Self, String> {
        let (store, _) = crate::notes::store_for(settings.notes.folder.clone())?;
        let note = store.read(rel).map_err(|e| e.to_string())?;
        let summary =
            crate::notes::markdown::section(&note.body, "Summary").filter(|s| !s.trim().is_empty());
        Ok(Happened {
            note: Some(NoteRef {
                title: crate::notes::markdown::title_of(&note.body),
                path: store.root().join(rel),
                summary,
            }),
            ..Happened::new(event, note.body, Local::now())
        })
    }

    /// The details a program gets in its environment.
    fn env(&self) -> Vec<(&'static str, String)> {
        let mut env = vec![
            ("SPLASHDOWN_EVENT", self.event.to_string()),
            ("SPLASHDOWN_AT", self.at.clone()),
        ];
        if let Some(app) = &self.app {
            env.push(("SPLASHDOWN_APP", app.clone()));
        }
        if let Some(preset) = &self.preset {
            env.push(("SPLASHDOWN_PRESET", preset.clone()));
        }
        if let Some(note) = &self.note {
            env.push(("SPLASHDOWN_TITLE", note.title.clone()));
            env.push(("SPLASHDOWN_NOTE", note.path.to_string_lossy().into_owned()));
        }
        if self.test {
            env.push(("SPLASHDOWN_TEST", "1".into()));
        }
        env
    }
}

// ---------------------------------------------------------------- after the fact

/// Runs the hooks for what happened, one at a time in the background.
pub fn fire(app: &AppHandle, happened: Happened) {
    let Some(settings) = app.try_state::<AppState>().map(|s| s.settings()) else {
        return;
    };
    let Some(event) = event_named(happened.event) else {
        return;
    };
    let hooks: Vec<Hook> = settings
        .hooks
        .iter()
        .filter(|h| h.applies(event, happened.preset_id.as_deref()))
        .cloned()
        .collect();
    if hooks.is_empty() {
        return;
    }
    let queue = QUEUE.get_or_init(|| {
        let (send, receive) = channel::<(Hook, Happened)>();
        let app = app.clone();
        let started = std::thread::Builder::new()
            .name("hooks".into())
            .spawn(move || {
                for (hook, happened) in receive {
                    if let Err(e) = run_after(&app, &hook, &happened) {
                        tracing::warn!("hook {} ({}): {e}", name(&hook), happened.event);
                        warn(&format!("{}: {e}", name(&hook)));
                    }
                }
            });
        if let Err(e) = started {
            tracing::error!("couldn't start running hooks: {e}");
        }
        Mutex::new(send)
    });
    let queue = queue.locked();
    for hook in hooks {
        let _ = queue.send((hook, happened.clone()));
    }
}

/// A meeting's or transcript's note at `rel` is finished.
pub fn note_finished(app: &AppHandle, event: HookEvent, rel: &str) {
    let Some(settings) = app.try_state::<AppState>().map(|s| s.settings()) else {
        return;
    };
    if !settings.hooks.iter().any(|h| h.applies(event, None)) {
        return;
    }
    match Happened::note(&settings, event, rel) {
        Ok(happened) => fire(app, happened),
        Err(e) => tracing::warn!("a hook couldn't read {rel}: {e}"),
    }
}

fn run_after(app: &AppHandle, hook: &Hook, happened: &Happened) -> Result<(), String> {
    if !hook.trusted && !allowed(app, hook) {
        return Ok(());
    }
    match &hook.run {
        HookTarget::Program { path, args } => {
            match crate::programs::run(path, args, &happened.text, &happened.env(), AFTER_LIMIT)? {
                Ended::Finished { ok: true, .. } => Ok(()),
                Ended::Finished { error, .. } if !error.is_empty() => Err(error),
                Ended::Finished { .. } => Err("stopped with an error".into()),
                Ended::TooLong => Err(format!("stopped after {} seconds", AFTER_LIMIT.as_secs())),
            }
        }
        HookTarget::Web { url } => send(url, happened),
    }
}

/// Asks before running a hook that wasn't set up in Settings; allowing it
/// means not asking again.
fn allowed(app: &AppHandle, hook: &Hook) -> bool {
    let event = match hook.when {
        HookEvent::Dictation => "after each dictation",
        HookEvent::BeforeInsert => "to change each dictation",
        HookEvent::Meeting => "after each meeting",
        HookEvent::Transcript => "after each transcript",
        HookEvent::PhoneNote => "for each note from your iPhone",
    };
    let answer = crate::prompt::ask(
        app,
        &format!("Run “{}” {event}?", name(hook)),
        &format!(
            "{}. It's in your settings, but wasn't added in Settings.",
            describe(hook)
        ),
        &["Allow", "Don't run it"],
    );
    if answer != Some(0) {
        return false;
    }
    let state = app.state::<AppState>();
    let saved = state.update(app, |s| {
        for h in &mut s.hooks {
            if h.runs_same_as(hook) && h.preset == hook.preset {
                h.trusted = true;
            }
        }
    });
    if let Err(e) = saved {
        tracing::warn!("couldn't remember allowing a hook: {e}");
    }
    crate::state::settings_changed_elsewhere(app);
    true
}

/// Shows a hook's failure for a moment.
fn warn(text: &str) {
    let Some(ui) = UI.get() else {
        return;
    };
    ui.show(OverlayState::Message {
        tone: Tone::Warning,
        text: text.to_string(),
    });
    let shown = ui.generation();
    std::thread::sleep(WARNING_FOR);
    ui.hide_if_unchanged(shown);
}

// ---------------------------------------------------------------- before inserting

/// A hook will change this preset's dictations before they're inserted (so
/// they aren't typed as they stream in).
pub fn changes_text(settings: &Settings, preset: &str) -> bool {
    settings
        .hooks
        .iter()
        .any(|h| h.applies(HookEvent::BeforeInsert, Some(preset)))
}

/// Runs the hooks that change a dictation, in order: the text to insert, or
/// why it's inserted as it was. Blocks for up to `BEFORE_LIMIT` each.
pub fn change(
    app: &AppHandle,
    settings: &Settings,
    mut happened: Happened,
) -> Result<String, String> {
    for hook in settings
        .hooks
        .iter()
        .filter(|h| h.applies(HookEvent::BeforeInsert, happened.preset_id.as_deref()))
    {
        if !hook.trusted && !allowed(app, hook) {
            continue;
        }
        happened.text = changed(hook, &happened)?;
    }
    Ok(happened.text)
}

fn changed(hook: &Hook, happened: &Happened) -> Result<String, String> {
    let HookTarget::Program { path, args } = &hook.run else {
        return Ok(happened.text.clone());
    };
    let who = name(hook);
    match crate::programs::run(path, args, &happened.text, &happened.env(), BEFORE_LIMIT)
        .map_err(|e| format!("{who}: {e}"))?
    {
        Ended::Finished {
            ok: true, output, ..
        } => {
            let output = output.trim_end_matches(['\n', '\r']);
            if output.trim().is_empty() {
                Err(format!("{who} gave back no text"))
            } else {
                Ok(output.to_string())
            }
        }
        Ended::Finished { error, .. } if !error.is_empty() => Err(format!("{who}: {error}")),
        Ended::Finished { .. } => Err(format!("{who} stopped with an error")),
        Ended::TooLong => Err(format!(
            "{who} took over {} seconds",
            BEFORE_LIMIT.as_secs()
        )),
    }
}

// ---------------------------------------------------------------- web addresses

/// Posts what happened to `url` as JSON, signed with the secret, trying
/// once more if the server or network failed.
fn send(url: &str, happened: &Happened) -> Result<(), String> {
    let body = serde_json::to_string(happened).map_err(|e| e.to_string())?;
    let secret = secret()?;
    let client = reqwest::Client::builder()
        .timeout(WEB_LIMIT)
        .user_agent(concat!("Splashdown/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2));
        }
        let at = chrono::Utc::now().timestamp().to_string();
        let request = client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("X-Splashdown-Event", happened.event)
            .header("X-Splashdown-Timestamp", &at)
            .header("X-Splashdown-Signature", signature(&secret, &at, &body))
            .body(body.clone())
            .send();
        match tauri::async_runtime::block_on(request) {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) if r.status().is_server_error() => {
                last = format!("the server said {}", r.status())
            }
            Ok(r) => return Err(format!("the server said {}", r.status())),
            Err(e) if e.is_timeout() => last = "no answer in time".into(),
            Err(e) => last = format!("couldn't reach it ({})", short_error(&e)),
        }
    }
    Err(last)
}

fn short_error(e: &reqwest::Error) -> String {
    use std::error::Error;
    let mut source: &dyn Error = e;
    while let Some(inner) = source.source() {
        source = inner;
    }
    source.to_string()
}

/// `sha256=` and the HMAC-SHA256 of `"{timestamp}.{body}"` with the
/// secret, in hex: what `X-Splashdown-Signature` holds.
fn signature(secret: &str, at: &str, body: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC takes a key of any length");
    mac.update(at.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    format!("sha256={}", hex(&mac.finalize().into_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn secret_file() -> Result<PathBuf, String> {
    Ok(crate::engine::paths()?.config_dir.join(SECRET_FILE))
}

/// The secret webhooks are signed with, made the first time it's needed.
fn secret() -> Result<String, String> {
    let file = secret_file()?;
    if let Ok(secret) = std::fs::read_to_string(&file)
        && !secret.trim().is_empty()
    {
        return Ok(secret.trim().to_string());
    }
    new_secret(&file)
}

fn new_secret(file: &std::path::Path) -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    let secret = format!("sds_{}", hex(&bytes));
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(file, &secret).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
    }
    Ok(secret)
}

// ---------------------------------------------------------------- words

fn event_named(name: &str) -> Option<HookEvent> {
    [
        HookEvent::Dictation,
        HookEvent::BeforeInsert,
        HookEvent::Meeting,
        HookEvent::Transcript,
        HookEvent::PhoneNote,
    ]
    .into_iter()
    .find(|e| e.name() == name)
}

/// What a hook is called: its program's file name, or its web address's
/// host.
fn name(hook: &Hook) -> String {
    match &hook.run {
        HookTarget::Program { path, .. } => std::path::Path::new(path)
            .file_name()
            .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned()),
        HookTarget::Web { url } => url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split(['/', '?', '#']).next())
            .unwrap_or(url)
            .to_string(),
    }
}

/// What a hook does, in a sentence.
fn describe(hook: &Hook) -> String {
    match &hook.run {
        HookTarget::Program { path, args } => {
            let mut line = format!("Runs {path}");
            for arg in args {
                line.push(' ');
                if arg.is_empty() || arg.contains(char::is_whitespace) {
                    line.push_str(&format!("“{arg}”"));
                } else {
                    line.push_str(arg);
                }
            }
            line
        }
        HookTarget::Web { url } => format!("Sends the text to {url}"),
    }
}

// ---------------------------------------------------------------- Settings

/// What's wrong with a hook before it's saved, if anything.
#[tauri::command]
pub fn hooks_problem(hook: Hook) -> Option<String> {
    hook.problem()
}

/// Test: runs the hook now with sample text for its event; what happened.
#[tauri::command]
pub async fn hooks_test(hook: Hook) -> Result<String, String> {
    if let Some(problem) = hook.problem() {
        return Err(problem);
    }
    tauri::async_runtime::spawn_blocking(move || test(&hook))
        .await
        .map_err(|e| e.to_string())?
}

fn test(hook: &Hook) -> Result<String, String> {
    let happened = sample(hook.when);
    match (&hook.run, hook.when) {
        (HookTarget::Program { .. }, HookEvent::BeforeInsert) => {
            let text = changed(hook, &happened)?;
            Ok(format!("It gave back: “{}”", text.trim()))
        }
        (HookTarget::Program { path, args }, _) => {
            match crate::programs::run(path, args, &happened.text, &happened.env(), AFTER_LIMIT)? {
                Ended::Finished { ok: true, .. } => Ok("It ran without errors.".into()),
                Ended::Finished { error, .. } if !error.is_empty() => Err(error),
                Ended::Finished { .. } => Err("It stopped with an error.".into()),
                Ended::TooLong => Err(format!(
                    "It ran over {} seconds and was stopped.",
                    AFTER_LIMIT.as_secs()
                )),
            }
        }
        (HookTarget::Web { url }, _) => {
            send(url, &happened).map(|()| "Sent, and the server took it.".into())
        }
    }
}

/// Sample text for Test.
fn sample(event: HookEvent) -> Happened {
    let mut happened = match event {
        HookEvent::Dictation | HookEvent::BeforeInsert => Happened::dictation(
            event,
            "This is a test from Splashdown.",
            Some("Notes"),
            ("default", "Default"),
        ),
        HookEvent::PhoneNote => Happened::phone("This is a test from Splashdown.", Local::now()),
        HookEvent::Meeting | HookEvent::Transcript => {
            let title = if event == HookEvent::Meeting {
                "Test meeting"
            } else {
                "Test transcript"
            };
            Happened {
                note: Some(NoteRef {
                    title: title.into(),
                    path: PathBuf::new(),
                    summary: Some("A test from Splashdown.".into()),
                }),
                ..Happened::new(
                    event,
                    format!(
                        "# {title}\n\n## Summary\n\nA test from Splashdown.\n\n## Transcript\n\n**[0:00] You:** This is a test.\n"
                    ),
                    Local::now(),
                )
            }
        }
    };
    happened.test = true;
    happened
}

/// Settings → What each one gets: the hooks reference.
#[tauri::command]
pub fn hooks_docs_page() -> Result<(), String> {
    splashdown_platform::open_url(DOCS).map_err(|e| e.to_string())
}

/// The secret webhooks are signed with, to give to the receiving end.
#[tauri::command]
pub fn hooks_secret() -> Result<String, String> {
    secret()
}

/// A new secret, after the old one was shared by mistake.
#[tauri::command]
pub fn hooks_new_secret() -> Result<String, String> {
    new_secret(&secret_file()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhooks_are_signed_as_documented() {
        // openssl: printf '1700000000.{"event":"test"}' | openssl dgst -sha256 -hmac sds_example
        assert_eq!(
            signature("sds_example", "1700000000", r#"{"event":"test"}"#),
            "sha256=1b2905a82cfcfd0b94ab961e72682041ebcd30560989f075185a3195ef5331a8"
        );
    }

    #[test]
    fn what_a_hook_gets() {
        let happened = Happened::dictation(
            HookEvent::Dictation,
            "Ship it",
            Some("Slack"),
            ("chat", "Chat"),
        );
        let json: serde_json::Value = serde_json::to_value(&happened).unwrap();
        assert_eq!(json["event"], "dictation");
        assert_eq!(json["text"], "Ship it");
        assert_eq!(json["app"], "Slack");
        assert_eq!(json["preset"], "Chat");
        assert!(json.get("preset_id").is_none());
        assert!(json.get("test").is_none());
        let env = happened.env();
        assert!(env.contains(&("SPLASHDOWN_EVENT", "dictation".into())));
        assert!(env.contains(&("SPLASHDOWN_APP", "Slack".into())));
        let test = sample(HookEvent::Meeting);
        assert!(test.text.contains("## Summary"));
        assert!(test.env().contains(&("SPLASHDOWN_TEST", "1".into())));
        assert_eq!(serde_json::to_value(&test).unwrap()["test"], true);
    }

    #[test]
    fn hooks_are_named_by_program_or_host() {
        let hook = |run| Hook {
            when: HookEvent::Meeting,
            run,
            preset: None,
            on: true,
            trusted: false,
        };
        assert_eq!(
            name(&hook(HookTarget::Program {
                path: "/Users/me/bin/to-slack.sh".into(),
                args: vec![]
            })),
            "to-slack.sh"
        );
        assert_eq!(
            name(&hook(HookTarget::Web {
                url: "https://hooks.zapier.com/hooks/catch/1/abc".into()
            })),
            "hooks.zapier.com"
        );
        for event in [
            HookEvent::Dictation,
            HookEvent::BeforeInsert,
            HookEvent::Meeting,
            HookEvent::Transcript,
            HookEvent::PhoneNote,
        ] {
            assert_eq!(event_named(event.name()), Some(event));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_hook_changes_the_text_or_says_why_not() {
        let dir = std::env::temp_dir().join(format!("sd-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            Hook {
                when: HookEvent::BeforeInsert,
                run: HookTarget::Program {
                    path: path.to_string_lossy().into_owned(),
                    args: vec![],
                },
                preset: Some("code".into()),
                on: true,
                trusted: true,
            }
        };
        let happened =
            Happened::dictation(HookEvent::BeforeInsert, "use react", None, ("code", "Code"));
        let fix = script("fix.sh", "sed 's/react/React/'");
        assert_eq!(changed(&fix, &happened).unwrap(), "use React");
        let empty = script("empty.sh", "cat >/dev/null");
        assert!(changed(&empty, &happened).unwrap_err().contains("no text"));
        let slow = script("slow.sh", "sleep 5");
        assert!(changed(&slow, &happened).unwrap_err().contains("took over"));
        let settings = Settings {
            hooks: vec![fix],
            ..Settings::default()
        };
        assert!(changes_text(&settings, "code"));
        assert!(!changes_text(&settings, "email"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
