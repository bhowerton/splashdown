//! Starting the user's own scripts and programs (their voice actions and
//! hooks): directly, never through a shell that reads the arguments.

use std::path::Path;
use std::process::Command;

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

/// Runs `path` with `args`, `input` on its standard input and `env` added
/// to its environment, for up to `limit`; stopped if it runs longer.
pub(crate) fn run(
    path: &str,
    args: &[String],
    input: &str,
    env: &[(&str, String)],
    limit: std::time::Duration,
) -> Result<Ended, String> {
    use std::io::{Read, Write};
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
    // reading all its input doesn't wait on us forever.
    let mut stdin = child.stdin.take();
    let input = input.to_string();
    let writer = std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            // A program that doesn't read its input is fine.
            let _ = stdin.write_all(input.as_bytes());
        }
    });
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let mut bytes = Vec::new();
                let _ = pipe.read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            text
        })
    };
    let out = reader(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = reader(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Ended::TooLong);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let _ = writer.join();
    let output = out.join().unwrap_or_default();
    let error = err
        .join()
        .unwrap_or_default()
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
            Duration::from_secs(5),
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
        match run(&path, &[], "", &[], Duration::from_secs(5)).unwrap() {
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
}
