//! Starting the user's own scripts and programs (their voice actions and
//! hooks): directly, never through a shell that reads the arguments.

use std::path::Path;
use std::process::Command;

use splashdown_core::sync::Locked;

/// How to start `path` with `args`: directly, or through the program that
/// runs that kind of script. Never through a shell that reads the
/// arguments, except Windows batch files, which only cmd can run: their
/// arguments mustn't hold cmd's special characters.
pub(crate) fn command_for(path: &str, args: &[String]) -> Result<Command, String> {
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

/// How a program run with `run` ended.
#[derive(Debug)]
pub(crate) enum Ended {
    /// It finished in time: whether it succeeded, what it wrote, and the
    /// first line of its errors.
    Finished {
        ok: bool,
        output: String,
        error: String,
    },
    /// It ran past the limit and was stopped.
    TooLong,
}

/// How long what a program wrote is waited for once it has ended. It's all
/// there by then, unless something the program started and left running
/// still holds its output open.
const OUTPUT_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

/// Runs `path` with `args`, `input` on its standard input and `env` added
/// to its environment, for up to `limit`; stopped, with anything it
/// started, if it runs longer. Never waits past `limit` (and a moment for
/// its output), even when the program leaves something running that keeps
/// its output open.
pub(crate) fn run(
    path: &str,
    args: &[String],
    input: &str,
    env: &[(&str, String)],
    limit: std::time::Duration,
) -> Result<Ended, String> {
    use std::io::Write;
    use std::process::Stdio;

    if !Path::new(path).exists() {
        return Err(format!("{path} isn't there any more"));
    }
    let mut command = command_for(path, args)?;
    command
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A group of its own, so stopping it stops what it started too.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up for it.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("couldn't run {path}: {e}"))?;
    // Written and read beside it, so a program that writes a lot before
    // reading all its input doesn't wait on us forever. The writer isn't
    // waited for: a program needn't read its input.
    let mut stdin = child.stdin.take();
    let input = input.to_string();
    std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(input.as_bytes());
        }
    });
    let out = Reading::start(child.stdout.take());
    let err = Reading::start(child.stderr.take());
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > limit {
            stop(&mut child);
            return Ok(Ended::TooLong);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let until = std::time::Instant::now() + OUTPUT_GRACE;
    let output = out.text(until);
    let error = err
        .text(until)
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(Ended::Finished {
        ok: status.success(),
        output,
        error,
    })
}

/// Stops a program that ran too long, and what it started.
fn stop(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(group) = libc::pid_t::try_from(child.id())
        && group > 0
    {
        // SAFETY: `kill` takes any process id and signal and reports
        // failure through its return value. `-group` names the process
        // group `run` put the child in (`process_group(0)` makes the
        // child's id the group's), and the child hasn't been waited for,
        // so the id still belongs to it.
        unsafe { libc::kill(-group, libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // The whole tree: `kill` below ends only the program itself.
        let pid = child.id().to_string();
        let _ = Command::new("taskkill")
            .args(["/PID", pid.as_str(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// What a program writes to one of its pipes, read on a thread of its own
/// so it can be taken without waiting for the pipe to close.
struct Reading {
    bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    /// Closed when the pipe is.
    ended: std::sync::mpsc::Receiver<()>,
}

impl Reading {
    fn start(pipe: Option<impl std::io::Read + Send + 'static>) -> Self {
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (end, ended) = std::sync::mpsc::channel::<()>();
        if let Some(mut pipe) = pipe {
            let bytes = bytes.clone();
            std::thread::spawn(move || {
                let _end = end;
                let mut piece = [0u8; 8192];
                loop {
                    match pipe.read(&mut piece) {
                        Ok(0) => break,
                        Ok(n) => bytes.locked().extend_from_slice(&piece[..n]),
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
            });
        }
        Reading { bytes, ended }
    }

    /// What was written, waiting until `until` at most for the pipe to
    /// close.
    fn text(self, until: std::time::Instant) -> String {
        let wait = until.saturating_duration_since(std::time::Instant::now());
        // An error either way: the pipe closed (all of it is here), or
        // time is up (what's here will do).
        let _ = self.ended.recv_timeout(wait);
        String::from_utf8_lossy(&self.bytes.locked()).into_owned()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;

    fn script(name: &str, body: &str) -> String {
        let dir = std::env::temp_dir().join(format!("sd-programs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_program_gets_the_text_and_details_and_its_output_comes_back() {
        let path = script(
            "upper.sh",
            r#"printf '%s:' "$SPLASHDOWN_EVENT"; tr a-z A-Z"#,
        );
        let ended = run(
            &path,
            &[],
            "hello there",
            &[("SPLASHDOWN_EVENT", "dictation".into())],
            // Long enough for a machine busy with the other tests: five
            // seconds wasn't, now and then.
            Duration::from_secs(30),
        )
        .unwrap();
        match ended {
            Ended::Finished { ok, output, .. } => {
                assert!(ok);
                assert_eq!(output, "dictation:HELLO THERE");
            }
            Ended::TooLong => panic!("ran too long"),
        }
    }

    #[test]
    fn a_failing_program_says_why_and_a_slow_one_is_stopped() {
        let path = script("fail.sh", "echo 'no token set' >&2; exit 3");
        match run(&path, &[], "", &[], Duration::from_secs(30)).unwrap() {
            Ended::Finished { ok, error, .. } => {
                assert!(!ok);
                assert_eq!(error, "no token set");
            }
            Ended::TooLong => panic!("ran too long"),
        }
        let path = script("slow.sh", "sleep 5");
        let started = std::time::Instant::now();
        assert!(matches!(
            run(&path, &[], "", &[], Duration::from_millis(300)).unwrap(),
            Ended::TooLong
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(run("/no/such/program", &[], "", &[], Duration::from_secs(1)).is_err());
    }

    #[test]
    fn something_left_running_doesn_t_hold_the_caller() {
        // The script ends at once; what it started keeps its output open.
        let path = script("leaves.sh", "sleep 5 &\necho fixed");
        let started = std::time::Instant::now();
        match run(&path, &[], "hi", &[], Duration::from_secs(2)).unwrap() {
            Ended::Finished { ok, output, .. } => {
                assert!(ok);
                assert_eq!(output, "fixed\n");
            }
            Ended::TooLong => panic!("ran too long"),
        }
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn stopping_a_slow_program_stops_what_it_started() {
        let dir = std::env::temp_dir().join(format!("sd-programs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mark = dir.join("still-running");
        let _ = std::fs::remove_file(&mark);
        // Its child would leave a mark after the script was stopped.
        let path = script(
            "tree.sh",
            &format!("(sleep 1; touch '{}') &\nsleep 5", mark.display()),
        );
        assert!(matches!(
            run(&path, &[], "", &[], Duration::from_millis(200)).unwrap(),
            Ended::TooLong
        ));
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!mark.exists(), "the child outlived the script");
    }
}
