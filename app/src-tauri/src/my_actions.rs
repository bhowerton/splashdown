//! The user's own voice actions (Settings → Actions → Your actions): open
//! a link, run a script or program, or press keys, each with a name to say.
//! `{text}` in a link or an argument is filled with what was said for it:
//! percent-encoded in a link, and passed as one argument, never through a
//! shell, to a program. Each asks before running, with exactly what will
//! happen, until the user chooses Always allow.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use splashdown_config::{ActionSettings, MyAction, MyActionKind, TEXT_SLOT};
use splashdown_core::action::Mine;
use tauri::{AppHandle, Manager};

use crate::commands::AppState;
use crate::voice_actions::Done;

/// How long a script may run before the pill says it's still running.
const WAIT: Duration = Duration::from_secs(15);

/// The actions as the planner sees them.
pub fn for_planner(settings: &ActionSettings) -> Vec<Mine> {
    settings
        .mine
        .iter()
        .map(|m| Mine {
            name: m.name.clone(),
            about: m.about.clone(),
            takes_text: m.takes_text(),
        })
        .collect()
}

/// Runs the action called `name` (as planned), asking first unless it's
/// trusted.
pub fn run(
    app: &AppHandle,
    settings: &ActionSettings,
    name: &str,
    text: Option<&str>,
) -> Result<Done, String> {
    let Some(action) = settings.mine.iter().find(|m| m.name == name) else {
        return Err(format!("No action called “{name}”"));
    };
    let text = text.map(str::trim).filter(|t| !t.is_empty());
    if action.takes_text() && text.is_none() {
        return Err(format!("“{name}” needs something said for it"));
    }
    let what = describe(action, text.unwrap_or_default())?;
    if !action.trusted {
        match crate::prompt::ask(
            app,
            &format!("Run “{name}”?"),
            &what,
            &["Run it", "Always allow", "Cancel"],
        ) {
            Some(0) => {}
            Some(1) => trust(app, name)?,
            _ => {
                return Ok(Done {
                    text: "Not run".into(),
                    undo: None,
                });
            }
        }
    }
    let text = text.unwrap_or_default();
    let said = match &action.does {
        MyActionKind::Link { url } => {
            splashdown_platform::open_url(&fill_url(url, text)).map_err(|e| e.to_string())?;
            format!("Opened “{name}”")
        }
        MyActionKind::Program { path, args } => run_program(name, path, &fill_args(args, text)?)?,
        MyActionKind::Keys { keys } => {
            let combo = splashdown_core::computer::parse_combo(keys)?;
            splashdown_platform::press_combo(&combo).map_err(|e| e.to_string())?;
            format!("Pressed {keys}")
        }
    };
    Ok(Done {
        text: said,
        undo: None,
    })
}

/// What will happen, in the confirmation panel.
fn describe(action: &MyAction, text: &str) -> Result<String, String> {
    Ok(match &action.does {
        MyActionKind::Link { url } => format!("Opens {}", fill_url(url, text)),
        MyActionKind::Program { path, args } => {
            let mut line = format!("Runs {path}");
            for arg in fill_args(args, text)? {
                line.push(' ');
                if arg.is_empty() || arg.contains(char::is_whitespace) {
                    line.push_str(&format!("“{arg}”"));
                } else {
                    line.push_str(&arg);
                }
            }
            line
        }
        MyActionKind::Keys { keys } => format!("Presses {keys} in the app in front"),
    })
}

/// The link with the text percent-encoded into `{text}`.
fn fill_url(url: &str, text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    url.replace(TEXT_SLOT, &encoded)
}

/// The arguments with the text in each `{text}`.
fn fill_args(args: &[String], text: &str) -> Result<Vec<String>, String> {
    // A line break could start a second command in a batch file, and
    // nothing said needs one.
    if text.contains(['\n', '\r', '\0']) {
        return Err("The text for it can't have line breaks".into());
    }
    Ok(args.iter().map(|a| a.replace(TEXT_SLOT, text)).collect())
}

/// How to start `path` with `args`: directly, or through the program that
/// runs that kind of script. Never through a shell that reads the
/// arguments, except Windows batch files, which only cmd can run: their
/// arguments mustn't hold cmd's special characters.
fn command_for(path: &str, args: &[String]) -> Result<Command, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mut command = match ext.as_str() {
        #[cfg(target_os = "macos")]
        "app" => {
            let mut c = Command::new("/usr/bin/open");
            c.arg("-a").arg(path).arg("--args");
            c
        }
        #[cfg(not(windows))]
        "sh" | "command" if !executable(path) => {
            let mut c = Command::new("/bin/sh");
            c.arg(path);
            c
        }
        #[cfg(windows)]
        "ps1" => {
            let mut c = Command::new("powershell.exe");
            c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path]);
            c
        }
        #[cfg(windows)]
        "bat" | "cmd" => {
            if args
                .iter()
                .any(|a| a.contains(['&', '|', '<', '>', '^', '%', '"', '!']))
            {
                return Err("The text has characters a batch file can't take safely".into());
            }
            let mut c = Command::new("cmd.exe");
            c.args(["/D", "/C"]).arg(path);
            c
        }
        _ => Command::new(path),
    };
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up for a script.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if matches!(ext.as_str(), "ps1" | "bat" | "cmd") {
            command.creation_flags(CREATE_NO_WINDOW);
        }
    }
    Ok(command)
}

#[cfg(not(windows))]
fn executable(path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

/// Runs it, waiting a little for it to finish; what the pill says.
fn run_program(name: &str, path: &str, args: &[String]) -> Result<String, String> {
    if !Path::new(path).exists() {
        return Err(format!("“{name}”: {path} isn't there any more"));
    }
    let mut child = command_for(path, args)?
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run “{name}”: {e}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if status.success() {
                return Ok(format!("Ran “{name}”"));
            }
            let mut said = String::new();
            if let Some(mut err) = child.stderr.take() {
                use std::io::Read;
                let _ = err.read_to_string(&mut said);
            }
            let first = said.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            tracing::info!("{name} exited with {status}");
            return Err(if first.is_empty() {
                format!("“{name}” stopped with an error")
            } else {
                format!("“{name}”: {}", first.trim())
            });
        }
        if started.elapsed() > WAIT {
            // Left to finish on its own.
            return Ok(format!("“{name}” is still running"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// "Always allow": it runs without asking from now on.
fn trust(app: &AppHandle, name: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state.settings();
    for action in &mut settings.actions.mine {
        if action.name == name {
            action.trusted = true;
        }
    }
    settings.save(&crate::engine::paths()?.config_file())?;
    crate::commands::show_settings(app, &state, &settings);
    *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
    crate::commands::settings_changed_elsewhere(app);
    Ok(())
}

// ---------------------------------------------------------------- Settings

/// What's wrong with an action before it's saved, if anything.
#[tauri::command]
pub fn my_action_problem(action: MyAction) -> Option<String> {
    problem(&action)
}

fn problem(action: &MyAction) -> Option<String> {
    if action.name.trim().is_empty() {
        return Some("Give it a name to say.".into());
    }
    match &action.does {
        MyActionKind::Link { url } => {
            let filled = fill_url(url.trim(), "x");
            match reqwest::Url::parse(&filled) {
                Ok(u) if !u.scheme().is_empty() => None,
                _ => Some(
                    "That isn't a link (it should start with https:// or another app's scheme)."
                        .into(),
                ),
            }
        }
        MyActionKind::Program { path, .. } => {
            (!Path::new(path.trim()).exists()).then(|| format!("There's nothing at {path}."))
        }
        MyActionKind::Keys { keys } => splashdown_core::computer::parse_combo(keys.trim())
            .err()
            .map(|e| format!("Keys like cmd+shift+4 or right ({e}).")),
    }
}

/// A native file picker for a script or program. `None` if cancelled.
#[tauri::command]
pub async fn my_action_pick_program(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a script or program")
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str) -> MyAction {
        MyAction {
            name: "Issue".into(),
            about: String::new(),
            does: MyActionKind::Link { url: url.into() },
            trusted: false,
        }
    }

    #[test]
    fn text_is_encoded_into_links() {
        assert_eq!(
            fill_url("https://linear.app/acme/issue/{text}", "ENG 42/a&b"),
            "https://linear.app/acme/issue/ENG%2042%2Fa%26b"
        );
        assert_eq!(fill_url("https://x.com/", "ignored"), "https://x.com/");
        assert!(link("https://x/{text}").takes_text());
        assert!(!link("https://x/").takes_text());
    }

    #[test]
    fn text_is_one_argument() {
        let args = vec!["--env".to_string(), "{text}".to_string()];
        assert_eq!(
            fill_args(&args, "staging; rm -rf ~").unwrap(),
            vec!["--env", "staging; rm -rf ~"]
        );
        assert!(fill_args(&args, "a\nb").is_err());
    }

    #[test]
    fn says_what_it_will_do() {
        let action = MyAction {
            name: "Deploy".into(),
            about: String::new(),
            does: MyActionKind::Program {
                path: "/usr/local/bin/deploy".into(),
                args: vec!["--to".into(), "{text}".into()],
            },
            trusted: false,
        };
        assert_eq!(
            describe(&action, "the staging site").unwrap(),
            "Runs /usr/local/bin/deploy --to “the staging site”"
        );
    }

    #[test]
    fn checks_before_saving() {
        assert!(problem(&link("https://linear.app/acme/issue/{text}")).is_none());
        assert!(problem(&link("raycast://extensions/x?arguments={text}")).is_none());
        assert!(problem(&link("linear issue")).is_some());
        let mut keys = link("");
        keys.does = MyActionKind::Keys {
            keys: "cmd+shift+4".into(),
        };
        assert!(problem(&keys).is_none());
        keys.does = MyActionKind::Keys {
            keys: "cmd+".into(),
        };
        assert!(problem(&keys).is_some());
        keys.name = " ".into();
        assert!(problem(&keys).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn batch_files_refuse_special_characters() {
        assert!(command_for("C:\\x.bat", &["a & b".into()]).is_err());
        assert!(command_for("C:\\x.bat", &["a b".into()]).is_ok());
    }
}
