//! Setting coding agents up with this server: `splashdown-mcp agents`,
//! `install <agent>`, `uninstall <agent>` and `config <agent>`. Settings →
//! Privacy → Coding agents runs these too.
//!
//! Where an agent has a command for it (`claude mcp add`, `codex mcp add`,
//! `code --add-mcp`), that command does the work, so the agent's own
//! config files are only ever changed by the agent. Cursor gets its
//! install link, which asks before adding. Claude Desktop has neither, so
//! it gets the lines to add by hand. Configs are only read, to say whether
//! an agent is set up already.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// The name the server is registered under in every agent.
pub const NAME: &str = "splashdown";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Codex,
    Cursor,
    VsCode,
    ClaudeDesktop,
}

pub const AGENTS: [Agent; 5] = [
    Agent::ClaudeCode,
    Agent::Codex,
    Agent::Cursor,
    Agent::VsCode,
    Agent::ClaudeDesktop,
];

impl Agent {
    pub fn id(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "claude-code",
            Agent::Codex => "codex",
            Agent::Cursor => "cursor",
            Agent::VsCode => "vscode",
            Agent::ClaudeDesktop => "claude-desktop",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Cursor => "Cursor",
            Agent::VsCode => "VS Code",
            Agent::ClaudeDesktop => "Claude Desktop",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        let id = id.trim().to_lowercase().replace([' ', '_'], "-");
        AGENTS.into_iter().find(|a| {
            a.id() == id
                || matches!(
                    (a, id.as_str()),
                    (Agent::ClaudeCode, "claude")
                        | (Agent::VsCode, "code" | "vs-code" | "visual-studio-code")
                )
        })
    }

    /// The agent's own command line program, if it has one we use.
    fn program(self) -> Option<&'static str> {
        match self {
            Agent::ClaudeCode => Some("claude"),
            Agent::Codex => Some("codex"),
            Agent::VsCode => Some("code"),
            Agent::Cursor | Agent::ClaudeDesktop => None,
        }
    }
}

/// This program, as agents should run it: the copy inside the app.
pub fn server_path() -> PathBuf {
    std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .unwrap_or_else(|_| PathBuf::from("splashdown-mcp"))
}

// ------------------------------------------------------------ what to run

/// The command an agent's own program takes to add the server.
fn add_args(agent: Agent, server: &Path) -> Vec<String> {
    let server = server.display().to_string();
    match agent {
        Agent::ClaudeCode => vec![
            "mcp".into(),
            "add".into(),
            "--scope".into(),
            "user".into(),
            NAME.into(),
            "--".into(),
            server,
        ],
        Agent::Codex => vec!["mcp".into(), "add".into(), NAME.into(), "--".into(), server],
        Agent::VsCode => vec![
            "--add-mcp".into(),
            server_json(agent, Path::new(&server)).to_string(),
        ],
        Agent::Cursor | Agent::ClaudeDesktop => Vec::new(),
    }
}

fn remove_args(agent: Agent) -> Vec<String> {
    match agent {
        Agent::ClaudeCode => vec![
            "mcp".into(),
            "remove".into(),
            "--scope".into(),
            "user".into(),
            NAME.into(),
        ],
        Agent::Codex => vec!["mcp".into(), "remove".into(), NAME.into()],
        _ => Vec::new(),
    }
}

/// The server's entry, as each agent's config or link writes it.
fn server_json(agent: Agent, server: &Path) -> Value {
    let command = server.display().to_string();
    match agent {
        Agent::VsCode => json!({ "name": NAME, "type": "stdio", "command": command }),
        // Cursor's docs list `type` as required, though their examples
        // leave it out.
        Agent::Cursor => json!({ "type": "stdio", "command": command }),
        _ => json!({ "command": command }),
    }
}

/// A command line as the user would type it, quoting what needs it.
fn shell_line(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|a| quote(a)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@".contains(c));
    if plain {
        arg.to_string()
    } else if cfg!(windows) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// Cursor's one-click install link: the server's JSON, base64-encoded.
fn cursor_link(server: &Path) -> String {
    format!(
        "cursor://anysphere.cursor-deeplink/mcp/install?name={NAME}&config={}",
        percent(&base64(
            server_json(Agent::Cursor, server).to_string().as_bytes()
        ))
    )
}

/// VS Code's install link: the server's JSON, percent-encoded.
fn vscode_link(server: &Path) -> String {
    format!(
        "vscode:mcp/install?{}",
        percent(&server_json(Agent::VsCode, server).to_string())
    )
}

fn percent(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

// ------------------------------------------------------------ where things are

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Folders a program might be in that an app started from the Dock or
/// the Start menu doesn't have on its PATH.
fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home() {
        for d in [
            ".local/bin",
            ".claude/local",
            ".npm-global/bin",
            ".bun/bin",
            ".volta/bin",
            ".cargo/bin",
        ] {
            dirs.push(home.join(d));
        }
    }
    if cfg!(target_os = "macos") {
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
        dirs.push("/Applications/Visual Studio Code.app/Contents/Resources/app/bin".into());
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Programs/Microsoft VS Code/bin"));
        }
    }
    dirs
}

fn search_path() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for d in extra_dirs() {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs
}

/// `name` on the PATH or in the usual places.
fn find_program(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| format!("{name}.{ext}"))
            .collect()
    } else {
        vec![name.to_string()]
    };
    search_path()
        .into_iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

/// Runs an agent's program, with the usual places on its PATH too (an
/// npm-installed program needs to find `node`).
fn run(program: &Path, args: &[String]) -> std::io::Result<std::process::Output> {
    let path = std::env::join_paths(search_path()).unwrap_or_default();
    let mut command = Command::new(program);
    command
        .args(args)
        .env("PATH", path)
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.output()
}

/// The config file each agent keeps its MCP servers in (only read here).
fn config_file(agent: Agent) -> Option<PathBuf> {
    let home = home()?;
    let appdata = || std::env::var_os("APPDATA").map(PathBuf::from);
    Some(match agent {
        Agent::ClaudeCode => home.join(".claude.json"),
        Agent::Codex => home.join(".codex/config.toml"),
        Agent::Cursor => home.join(".cursor/mcp.json"),
        Agent::VsCode if cfg!(windows) => appdata()?.join("Code/User/mcp.json"),
        Agent::VsCode => home.join("Library/Application Support/Code/User/mcp.json"),
        Agent::ClaudeDesktop if cfg!(windows) => {
            appdata()?.join("Claude/claude_desktop_config.json")
        }
        Agent::ClaudeDesktop => {
            home.join("Library/Application Support/Claude/claude_desktop_config.json")
        }
    })
}

/// Whether the agent looks installed on this computer.
fn present(agent: Agent) -> bool {
    if let Some(program) = agent.program()
        && find_program(program).is_some()
    {
        return true;
    }
    let app_dir = |name: &str| {
        cfg!(target_os = "macos") && Path::new("/Applications").join(name).exists()
            || home().is_some_and(|h| h.join("Applications").join(name).exists())
    };
    match agent {
        Agent::Cursor => {
            app_dir("Cursor.app") || home().is_some_and(|h| h.join(".cursor").is_dir())
        }
        Agent::VsCode => app_dir("Visual Studio Code.app"),
        Agent::ClaudeDesktop => {
            app_dir("Claude.app")
                || config_file(agent).is_some_and(|f| f.parent().is_some_and(Path::is_dir))
        }
        Agent::ClaudeCode | Agent::Codex => false,
    }
}

/// The command the agent's config runs for this server, if it has one.
fn registered(agent: Agent) -> Option<String> {
    let text = std::fs::read_to_string(config_file(agent)?).ok()?;
    if agent == Agent::Codex {
        return codex_command(&text);
    }
    let json: Value = serde_json::from_str(&text).ok()?;
    let entry = match agent {
        Agent::VsCode => json.get("servers")?.get(NAME)?,
        // Claude Code keeps user-scope servers at the top level.
        _ => json.get("mcpServers")?.get(NAME)?,
    };
    Some(entry.get("command")?.as_str()?.to_string())
}

/// Claude Code's folders (its "local" scope, what `claude mcp add` does
/// without `--scope`) that have the server of their own.
fn project_entries(agent: Agent) -> Vec<String> {
    if agent != Agent::ClaudeCode {
        return Vec::new();
    }
    let Some(json) = config_file(agent)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return Vec::new();
    };
    let mut out: Vec<String> = json
        .get("projects")
        .and_then(Value::as_object)
        .map(|projects| {
            projects
                .iter()
                .filter(|(_, p)| p.get("mcpServers").and_then(|s| s.get(NAME)).is_some())
                .map(|(dir, _)| dir.clone())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// `command` under `[mcp_servers.splashdown]` in Codex's config.toml.
fn codex_command(toml: &str) -> Option<String> {
    let mut inside = false;
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[mcp_servers.{NAME}]");
            continue;
        }
        if inside
            && let Some(value) = line.strip_prefix("command").map(str::trim_start)
            && let Some(value) = value.strip_prefix('=')
        {
            let value = value.trim();
            // A basic string has escapes; a literal one ('…') has none.
            if let Some(v) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                return Some(v.replace("\\\\", "\\").replace("\\\"", "\""));
            }
            return Some(
                value
                    .strip_prefix('\'')
                    .and_then(|v| v.strip_suffix('\''))
                    .unwrap_or(value)
                    .to_string(),
            );
        }
    }
    None
}

// ------------------------------------------------------------ commands

/// What `agents` reports for one agent.
pub fn describe(agent: Agent, server: &Path) -> Value {
    let program = agent.program().and_then(find_program);
    let registered = registered(agent);
    let mut out = json!({
        "id": agent.id(),
        "name": agent.name(),
        "found": program.is_some() || present(agent),
        "set_up": registered.is_some(),
        "points_here": registered.as_deref().map(|c| Path::new(c) == server),
        "only_in": project_entries(agent),
        "config_file": config_file(agent).map(|p| p.display().to_string()),
    });
    match agent {
        Agent::ClaudeCode | Agent::Codex => {
            out["how"] = json!("command");
            out["command"] = json!(shell_line(
                agent.program().unwrap_or_default(),
                &add_args(agent, server)
            ));
        }
        Agent::VsCode => {
            out["how"] = json!(if program.is_some() { "command" } else { "link" });
            out["command"] = json!(shell_line("code", &add_args(agent, server)));
            out["link"] = json!(vscode_link(server));
        }
        Agent::Cursor => {
            out["how"] = json!("link");
            out["link"] = json!(cursor_link(server));
        }
        Agent::ClaudeDesktop => {
            out["how"] = json!("config");
            out["snippet"] = json!(
                serde_json::to_string_pretty(
                    &json!({ "mcpServers": { NAME: server_json(agent, server) } })
                )
                .unwrap_or_default()
            );
        }
    }
    out
}

/// `install <agent>`: what was done, or what the user should do.
pub fn install(agent: Agent, server: &Path) -> Result<String, String> {
    match agent {
        Agent::ClaudeCode | Agent::Codex | Agent::VsCode => {
            let name = agent.program().unwrap_or_default();
            let Some(program) = find_program(name) else {
                return if agent == Agent::VsCode {
                    open(&vscode_link(server), agent).map(|()| {
                        "VS Code was asked to add Splashdown; confirm it there.".to_string()
                    })
                } else {
                    Err(format!(
                        "{} isn't installed here (no `{name}` command found).\nIf it is, run this where `{name}` works:\n{}",
                        agent.name(),
                        shell_line(name, &add_args(agent, server))
                    ))
                };
            };
            // Set up before (perhaps from another copy of Splashdown): replaced.
            if agent != Agent::VsCode && registered(agent).is_some() {
                let _ = run(&program, &remove_args(agent));
            }
            let output = run(&program, &add_args(agent, server)).map_err(|e| e.to_string())?;
            if output.status.success() {
                Ok(format!(
                    "Added to {}. Start a new session there to use it.",
                    agent.name()
                ))
            } else {
                Err(format!(
                    "{} said no: {}",
                    agent.name(),
                    first_line(&output.stderr)
                        .or(first_line(&output.stdout))
                        .unwrap_or("it didn't say why")
                ))
            }
        }
        Agent::Cursor => open(&cursor_link(server), agent)
            .map(|()| "Cursor was asked to add Splashdown; confirm it there.".to_string()),
        Agent::ClaudeDesktop => Err(format!(
            "Claude Desktop is set up by hand: add this to {} (Settings → Developer → Edit Config), then restart it:\n{}",
            config_file(agent).map_or_else(
                || "claude_desktop_config.json".into(),
                |p| p.display().to_string()
            ),
            describe(agent, server)["snippet"]
                .as_str()
                .unwrap_or_default()
        )),
    }
}

/// `uninstall <agent>`.
pub fn uninstall(agent: Agent) -> Result<String, String> {
    match agent {
        Agent::ClaudeCode | Agent::Codex => {
            let name = agent.program().unwrap_or_default();
            let program = find_program(name).ok_or_else(|| format!("Couldn't find `{name}`."))?;
            let output = run(&program, &remove_args(agent)).map_err(|e| e.to_string())?;
            if output.status.success() {
                Ok(format!("Removed from {}.", agent.name()))
            } else {
                Err(format!(
                    "{} said no: {}",
                    agent.name(),
                    first_line(&output.stderr)
                        .or(first_line(&output.stdout))
                        .unwrap_or("it didn't say why")
                ))
            }
        }
        _ => Err(format!(
            "Remove “{NAME}” from {}'s MCP servers there{}.",
            agent.name(),
            config_file(agent).map_or_else(String::new, |p| format!(" (or from {})", p.display()))
        )),
    }
}

/// What a program said went wrong: its first line that isn't a warning.
fn first_line(bytes: &[u8]) -> Option<&str> {
    let lines: Vec<&str> = std::str::from_utf8(bytes)
        .ok()?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let warning = |l: &&str| l.to_ascii_lowercase().starts_with("warning");
    lines
        .iter()
        .find(|l| !warning(l))
        .or(lines.first())
        .copied()
}

/// Opens an agent's install link in the agent.
fn open(url: &str, agent: Agent) -> Result<(), String> {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(windows) {
        // rundll32, not `cmd /c start`: cmd would read the link's & and %.
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .status()
    } else {
        Command::new("xdg-open").arg(url).status()
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        // The link itself is long and says nothing to the user.
        _ => Err(format!("Couldn't open {}. Is it installed?", agent.name())),
    }
}

/// `splashdown-mcp agents|install|uninstall|config …`: the exit code, or
/// `None` when the arguments aren't one of these.
pub fn main(args: &[String]) -> Option<i32> {
    let server = server_path();
    let json = args.iter().any(|a| a == "--json");
    let words: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let agent = |i: usize| -> Result<Agent, String> {
        let id = words.get(i).copied().unwrap_or_default();
        Agent::from_id(id).ok_or_else(|| {
            format!(
                "Which agent? One of: {}",
                AGENTS.iter().map(|a| a.id()).collect::<Vec<_>>().join(", ")
            )
        })
    };
    let result: Result<String, String> = match words.first().copied()? {
        "agents" | "status" => {
            let all: Vec<Value> = AGENTS.iter().map(|a| describe(*a, &server)).collect();
            if json {
                Ok(json!({ "server": server.display().to_string(), "agents": all }).to_string())
            } else {
                let mut lines = vec![format!("Splashdown's MCP server: {}", server.display())];
                for a in &all {
                    let only: Vec<&str> = a["only_in"]
                        .as_array()
                        .map(|d| d.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    let state = match (a["set_up"].as_bool(), a["points_here"].as_bool()) {
                        (Some(true), Some(false)) => {
                            "set up, with another copy of Splashdown".into()
                        }
                        (Some(true), _) => "set up".into(),
                        _ if !only.is_empty() => format!(
                            "set up for {} only (install sets it up everywhere)",
                            only.join(", ")
                        ),
                        _ if a["found"] == true => "not set up".into(),
                        _ => "not found".into(),
                    };
                    lines.push(format!(
                        "  {:<15} {state}",
                        a["name"].as_str().unwrap_or_default()
                    ));
                }
                lines.push("Set one up with: splashdown-mcp install <claude-code|codex|cursor|vscode|claude-desktop>".into());
                Ok(lines.join("\n"))
            }
        }
        "install" => agent(1).and_then(|a| install(a, &server)),
        "uninstall" | "remove" => agent(1).and_then(uninstall),
        "config" => agent(1).map(|a| {
            let d = describe(a, &server);
            if json {
                d.to_string()
            } else {
                ["command", "link", "snippet"]
                    .iter()
                    .filter_map(|k| d[k].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }),
        "help" | "-h" => Ok(HELP.to_string()),
        _ => return None,
    };
    match result {
        Ok(text) => {
            println!("{text}");
            Some(0)
        }
        Err(text) => {
            eprintln!("{text}");
            Some(1)
        }
    }
}

pub const HELP: &str = "splashdown-mcp: Splashdown's MCP server for coding agents.

Run with no arguments, it's the server (agents start it themselves).
  splashdown-mcp agents [--json]        which agents are set up with it
  splashdown-mcp install <agent>        set one up: claude-code, codex, cursor, vscode, claude-desktop
  splashdown-mcp uninstall <agent>      take it out again
  splashdown-mcp config <agent> [--json]  what to add by hand

The notes tools need Settings → Privacy → Coding agents in Splashdown.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_are_named_every_way() {
        assert_eq!(Agent::from_id("claude-code"), Some(Agent::ClaudeCode));
        assert_eq!(Agent::from_id("Claude Code"), Some(Agent::ClaudeCode));
        assert_eq!(Agent::from_id("claude"), Some(Agent::ClaudeCode));
        assert_eq!(Agent::from_id("VS Code"), Some(Agent::VsCode));
        assert_eq!(Agent::from_id("claude_desktop"), Some(Agent::ClaudeDesktop));
        assert_eq!(Agent::from_id("emacs"), None);
    }

    #[test]
    fn commands_and_links_carry_the_path() {
        let server =
            Path::new("/Applications/My Apps/Splashdown.app/Contents/MacOS/splashdown-mcp");
        let claude = shell_line("claude", &add_args(Agent::ClaudeCode, server));
        if cfg!(windows) {
            assert!(claude.ends_with(
                "-- \"/Applications/My Apps/Splashdown.app/Contents/MacOS/splashdown-mcp\""
            ));
        } else {
            assert_eq!(
                claude,
                "claude mcp add --scope user splashdown -- '/Applications/My Apps/Splashdown.app/Contents/MacOS/splashdown-mcp'"
            );
        }
        let cursor = cursor_link(server);
        let config = cursor.split("config=").nth(1).unwrap();
        assert!(
            cursor.starts_with(
                "cursor://anysphere.cursor-deeplink/mcp/install?name=splashdown&config="
            )
        );
        assert!(
            !config.contains('+') && !config.contains('/'),
            "base64 is percent-encoded"
        );
        let vscode = vscode_link(server);
        assert!(vscode.starts_with("vscode:mcp/install?%7B"));
        assert!(vscode.contains("%22name%22%3A%22splashdown%22") && vscode.contains("My%20Apps"));
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(br#"{"command":"/x"}"#), "eyJjb21tYW5kIjoiL3gifQ==");
    }

    #[test]
    fn codex_config_is_read() {
        let toml = "[other]\ncommand = \"x\"\n\n[mcp_servers.splashdown]\ncommand = \"/A/splashdown-mcp\"\nargs = []\n";
        assert_eq!(codex_command(toml).as_deref(), Some("/A/splashdown-mcp"));
        assert_eq!(
            codex_command("[mcp_servers.other]\ncommand = \"y\"\n"),
            None
        );
        assert_eq!(
            codex_command("[mcp_servers.splashdown]\ncommand = 'C:\\Apps\\splashdown-mcp.exe'\n")
                .as_deref(),
            Some("C:\\Apps\\splashdown-mcp.exe")
        );
        assert_eq!(
            codex_command(
                "[mcp_servers.splashdown]\ncommand = \"C:\\\\Apps\\\\splashdown-mcp.exe\"\n"
            )
            .as_deref(),
            Some("C:\\Apps\\splashdown-mcp.exe")
        );
    }

    #[test]
    fn errors_skip_warnings() {
        assert_eq!(
            first_line(b"WARNING: no aliases\nError: bad name\n"),
            Some("Error: bad name")
        );
        assert_eq!(
            first_line(b"\nwarning: only this\n"),
            Some("warning: only this")
        );
        assert_eq!(first_line(b""), None);
    }

    #[test]
    fn unknown_words_are_the_server() {
        assert_eq!(main(&[]), None);
        assert_eq!(main(&["--stdio".to_string()]), None);
        assert_eq!(main(&["serve".to_string()]), None);
    }
}
