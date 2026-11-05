//! Settings → Privacy → Coding agents → Set up: which agents have
//! Splashdown's MCP server, and setting one up. Both run `splashdown-mcp`
//! (next to the app), which does the work: `agents --json` and `install
//! <agent>` (`crates/splashdown-mcp/src/setup.rs`).

use std::path::PathBuf;
use std::process::Command;

/// `splashdown-mcp` next to the running app (inside the bundle, or in
/// `target/<profile>` during development).
fn server() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe.with_file_name(format!("splashdown-mcp{}", std::env::consts::EXE_SUFFIX));
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "splashdown-mcp isn't next to the app ({}); build it with `cargo build -p splashdown-mcp`",
            path.display()
        ))
    }
}

fn run(args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(server()?);
    command.args(args).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().map_err(|e| e.to_string())?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if output.status.success() {
        Ok(text(&output.stdout))
    } else {
        Err(text(&output.stderr))
    }
}

/// Each agent: found here, set up, and how to set it up (a command, a
/// link or lines to add).
#[tauri::command]
pub async fn coding_agents() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let text = run(&["agents", "--json"])?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sets `id` up (Claude Code and Codex through their own commands, Cursor
/// and VS Code through their install links); what happened, or what to do.
#[tauri::command]
pub async fn coding_agent_install(id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || run(&["install", &id]))
        .await
        .map_err(|e| e.to_string())?
}
