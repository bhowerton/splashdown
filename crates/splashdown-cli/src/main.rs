//! `splashdown-cli`: a developer harness for the Splashdown pipeline (and its evals),
//! and a remote control for the running app (`splashdown-cli toggle`, `splashdown-cli
//! listen`, … over its control socket; see `splashdown-control`). The app ships the
//! remote control alone, built with `--no-default-features`; the harness (`dev.rs`)
//! needs the `full` feature, on by default.
//!
//! API keys come from the environment or a `.env.local` file in the current
//! directory (`OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `SPLASHDOWN_CUSTOM_API_KEY`).

#[cfg(all(feature = "full", target_os = "macos"))]
mod computer;
#[cfg(feature = "full")]
mod dev;
#[cfg(feature = "full")]
mod eval;
mod import;
mod manage;
#[cfg(feature = "full")]
mod meeting;
mod notes;
#[cfg(feature = "full")]
mod opts;
#[cfg(feature = "full")]
mod record;
#[cfg(feature = "full")]
mod report;
#[cfg(feature = "full")]
mod speech;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use splashdown_control::Request;

use crate::import::*;
#[cfg(feature = "full")]
use crate::{
    meeting::*,
    opts::{ContextOpts, ProviderOpts, Setup},
    speech::*,
};
#[cfg(feature = "full")]
use splashdown_core::{AudioJob, Mode, TextJob};
#[cfg(feature = "full")]
use splashdown_providers::{LocalModel, SpeechModel};

#[derive(Parser)]
#[command(name = "splashdown-cli", version)]
#[cfg_attr(
    feature = "full",
    command(about = "Splashdown pipeline harness, and a remote control for the running app")
)]
#[cfg_attr(
    not(feature = "full"),
    command(about = "A remote control for the running Splashdown app, and its settings")
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The developer harness: the pipeline, evals, models, recording.
    #[cfg(feature = "full")]
    #[command(flatten)]
    Dev(dev::DevCommand),
    /// The running app's state: ready, recording, providers, meeting.
    /// Exits with 3 when the app isn't running.
    Status {
        /// Print it as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Read and change settings by dotted key (`general.language`), as in
    /// config.toml. A running app uses the change at once.
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    /// Search history (newest first): what was said or inserted. With no
    /// words, the latest entries.
    History {
        query: Vec<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Print the entries as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Your vocabulary, corrections and snippets.
    Words {
        #[command(subcommand)]
        command: WordsCommand,
    },
    /// Meeting recordings: You and Them on separate channels.
    Meeting {
        #[command(subcommand)]
        command: MeetingCommand,
    },
    /// Start hands-free dictation in the running app, or finish it.
    Toggle,
    /// Start hands-free dictation in the running app.
    Start,
    /// Finish the app's recording and insert the text.
    Stop,
    /// Discard the app's recording.
    Cancel,
    /// Record a hands-free answer in the running app and print it, instead
    /// of inserting it. Finish with the dictation key; Esc cancels.
    Listen {
        /// Shown in the app's overlay while it listens.
        #[arg(long)]
        prompt: Option<String>,
    },
    /// Add text to the running app's capture note (Inbox, or today's note).
    /// With no text, record a note hands-free: run it again, or tap the
    /// dictation key, to finish.
    Note {
        /// The text to add, e.g. `splashdown-cli note buy oat milk`.
        text: Vec<String>,
    },
    /// List, search and read your notes and meetings in the running app,
    /// as coding agents do (Settings → Privacy → Coding agents must allow
    /// it).
    Notes {
        #[command(subcommand)]
        command: notes::NotesCommand,
    },
}

#[derive(Subcommand)]
enum WordsCommand {
    /// Add vocabulary, corrections and snippets from another dictation
    /// app's file: a dictionary export (JSON with vocabulary and
    /// replacements), a settings file with custom_words, snippets as JSON
    /// ([{"name", "text"}]), or a word list (text or CSV; "misspelling,
    /// correction" lines become corrections). Nothing already there is
    /// changed.
    Import {
        file: PathBuf,
        /// Show what would be added without saving it.
        #[arg(long)]
        dry_run: bool,
        /// Config file (default: the app's config.toml).
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

// With the full build, `record`'s paths (bigger on Windows) make one
// variant large; it's parsed once.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum MeetingCommand {
    /// Start recording a meeting in the running app.
    Start,
    /// Stop the meeting; the app finishes its note.
    Stop,
    /// The meeting in progress, as JSON (`null` when there's none).
    Status,
    /// Recording and transcribing a meeting without the app.
    #[cfg(feature = "full")]
    #[command(flatten)]
    Dev(dev::MeetingDev),
}

#[derive(Subcommand)]
enum SettingsCommand {
    /// Every setting that's set, as `key = value`.
    List {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// One setting's value. Exits with 4 if it isn't set.
    Get {
        key: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Set a setting: `settings set general.language de`, or a TOML value
    /// (`true`, `0.5`, `["Tauri", "Kubernetes"]`). Exits with 4 for a
    /// setting Splashdown doesn't have.
    Set {
        key: String,
        value: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Put a setting back to its default.
    Unset {
        key: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Where the settings file is.
    Path,
}

/// The command line, or the usage error and exit 2. The copy inside the
/// app hasn't the developer commands, and says where they are.
fn parse() -> Cli {
    match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let unknown = e.kind() == clap::error::ErrorKind::InvalidSubcommand;
            let _ = e.print();
            if unknown && !cfg!(feature = "full") {
                eprintln!(
                    "\nThis is the splashdown-cli that comes with the app: it controls the running app and its settings. The pipeline, model and eval commands are in the full splashdown-cli, built from source (see CONTRIBUTING.md)."
                );
            }
            std::process::exit(e.exit_code());
        }
    }
}

/// Exit codes: 0 worked, 1 failed, 2 wrong usage, 3 the app isn't running
/// (for commands that need it), 4 no such setting or model.
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e:#}");
            // The app answered, but doesn't know the request yet.
            if format!("{e:#}").contains("unknown variant") {
                eprintln!(
                    "The running Splashdown is older than this splashdown-cli: quit it and open the new one."
                );
            }
            std::process::ExitCode::from(manage::exit_code(&e))
        }
    }
}

fn run() -> Result<()> {
    let cli = parse();
    // Remote control: no keys, models or runtime needed.
    let request = match &cli.command {
        Command::Toggle => Some(Request::Toggle),
        Command::Start => Some(Request::Start),
        Command::Stop => Some(Request::Stop),
        Command::Cancel => Some(Request::Cancel),
        Command::Listen { prompt } => Some(Request::Listen {
            prompt: prompt.clone(),
        }),
        Command::Note { text } => Some(Request::Note {
            text: Some(text.join(" ")).filter(|t| !t.trim().is_empty()),
        }),
        Command::Meeting {
            command: MeetingCommand::Start,
        } => Some(Request::MeetingStart),
        Command::Meeting {
            command: MeetingCommand::Stop,
        } => Some(Request::MeetingStop),
        Command::Meeting {
            command: MeetingCommand::Status,
        } => Some(Request::MeetingStatus),
        _ => None,
    };
    if let Some(request) = request {
        if let Some(text) = splashdown_control::send(&request)? {
            println!("{text}");
        }
        return Ok(());
    }

    // Status, settings and history need no keys or runtime either.
    match &cli.command {
        Command::Status { json } => return manage::status(*json),
        Command::Notes { command } => return notes::run(command),
        Command::History { query, limit, json } => {
            return manage::history(&query.join(" "), *limit, *json);
        }
        Command::Settings { command } => {
            return match command {
                SettingsCommand::List { config } => manage::settings_list(config.clone()),
                SettingsCommand::Get { key, config } => manage::settings_get(key, config.clone()),
                SettingsCommand::Set { key, value, config } => {
                    manage::settings_set(key, Some(value), config.clone())
                }
                SettingsCommand::Unset { key, config } => {
                    manage::settings_set(key, None, config.clone())
                }
                SettingsCommand::Path => {
                    println!("{}", manage::paths()?.config_file().display());
                    Ok(())
                }
            };
        }
        _ => {}
    }

    match cli.command {
        Command::Words {
            command:
                WordsCommand::Import {
                    file,
                    dry_run,
                    config,
                },
        } => import_words(&file, dry_run, config),
        #[cfg(feature = "full")]
        Command::Dev(command) => dev::run(command),
        #[cfg(feature = "full")]
        Command::Meeting {
            command: MeetingCommand::Dev(command),
        } => dev::meeting(command),
        Command::Toggle
        | Command::Start
        | Command::Stop
        | Command::Cancel
        | Command::Listen { .. }
        | Command::Note { .. }
        | Command::Notes { .. }
        | Command::Status { .. }
        | Command::History { .. }
        | Command::Settings { .. }
        | Command::Meeting {
            command: MeetingCommand::Start | MeetingCommand::Stop | MeetingCommand::Status,
        } => unreachable!("handled above"),
    }
}
