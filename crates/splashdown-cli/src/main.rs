//! `splashdown-cli` — developer harness for the Splashdown pipeline (milestone M1),
//! and a remote control for the running app (`splashdown-cli toggle`, `splashdown-cli
//! listen`, … over its control socket; see `splashdown-control`).
//!
//! API keys come from the environment or a `.env.local` file in the current
//! directory (`OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `SPLASHDOWN_CUSTOM_API_KEY`).

mod eval;
mod opts;
mod record;
mod report;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use splashdown_control::Request;
use splashdown_core::{AudioJob, Mode, TextJob};
use splashdown_providers::{LocalModel, WhisperModel};

use crate::opts::{ContextOpts, ProviderOpts, Setup};

#[derive(Parser)]
#[command(
    name = "splashdown-cli",
    version,
    about = "Splashdown pipeline harness, and a remote control for the running app"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Record from the microphone, then transcribe and clean up.
    Record {
        /// Press Enter once to switch to the instruction, again to finish.
        #[arg(long)]
        instruct: bool,
        /// Treat the speech as an instruction applied to this text.
        #[arg(long)]
        selection: Option<String>,
        /// Insert the transcript as-is (no AI pass).
        #[arg(long)]
        raw: bool,
        /// Input device name (see `splashdown-cli devices`).
        #[arg(long)]
        device: Option<String>,
        /// Save the recording as WAV (the instruction goes to `<name>-instruction.wav`).
        #[arg(long)]
        save: Option<PathBuf>,
        #[command(flatten)]
        providers: ProviderOpts,
        #[command(flatten)]
        context: ContextOpts,
    },
    /// Transcribe and clean up a WAV file.
    Transcribe {
        file: PathBuf,
        /// WAV file with a spoken instruction for the content.
        #[arg(long)]
        instruction_file: Option<PathBuf>,
        /// Treat the speech as an instruction applied to this text.
        #[arg(long)]
        selection: Option<String>,
        #[arg(long)]
        raw: bool,
        /// Whisper mode: raise quiet speech before transcribing it.
        #[arg(long)]
        whisper: bool,
        /// Remove background noise before transcribing (noise reduction).
        #[arg(long)]
        denoise: bool,
        #[command(flatten)]
        providers: ProviderOpts,
        #[command(flatten)]
        context: ContextOpts,
    },
    /// Interpret a spoken command (as said after the instruction key) and
    /// print the action the app would take.
    #[command(name = "command")]
    Interpret {
        /// The command, as said, e.g. "make that more formal".
        spoken: String,
        /// The last dictation the command may refer to.
        #[arg(long)]
        last: Option<String>,
        #[command(flatten)]
        providers: ProviderOpts,
        #[command(flatten)]
        context: ContextOpts,
    },
    /// Run only the AI pass on text, as if it had been dictated.
    Clean {
        text: String,
        #[arg(long)]
        instruction: Option<String>,
        /// Transform this text with `text` as the instruction.
        #[arg(long)]
        selection: Option<String>,
        #[command(flatten)]
        providers: ProviderOpts,
        #[command(flatten)]
        context: ContextOpts,
    },
    /// List microphones.
    Devices,
    /// Manage downloadable models (local Whisper, the built-in text model).
    Models {
        #[command(subcommand)]
        command: ModelsCommand,
    },
    /// Show the resolved configuration (never prints keys).
    Config {
        #[command(flatten)]
        providers: ProviderOpts,
    },
    /// Score providers and models against the eval sets in `eval/`.
    Eval {
        #[command(subcommand)]
        command: eval::EvalCommand,
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
}

#[derive(Subcommand)]
enum ModelsCommand {
    /// List models and whether they're downloaded.
    List,
    /// Download a model, e.g. `base.en`.
    Download { id: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Remote control: no keys, models or runtime needed.
    let request = match &cli.command {
        Command::Toggle => Some(Request::Toggle),
        Command::Start => Some(Request::Start),
        Command::Stop => Some(Request::Stop),
        Command::Cancel => Some(Request::Cancel),
        Command::Listen { prompt } => Some(Request::Listen {
            prompt: prompt.clone(),
        }),
        _ => None,
    };
    if let Some(request) = request {
        if let Some(text) = splashdown_control::send(&request)? {
            println!("{text}");
        }
        return Ok(());
    }

    // A missing .env.local is fine; keys may come from the environment.
    let _ = dotenvy::from_filename(".env.local");
    // Settings and models saved under the app's former name.
    if let Some(paths) = splashdown_config::Paths::resolve() {
        paths.migrate_legacy();
    }
    let rt = tokio::runtime::Runtime::new()?;

    match cli.command {
        Command::Record {
            instruct,
            selection,
            raw,
            device,
            save,
            providers,
            context,
        } => {
            // Build providers first so a config problem shows before recording.
            let setup = Setup::new(&providers)?;
            let mode = mode_for(raw, selection.is_some(), instruct);
            let (speech, instruction_speech) =
                record::record(device.as_deref(), instruct, save.as_deref())?;
            let job = AudioJob {
                mode,
                speech,
                instruction_speech,
                selection,
                context: context.prompt_context(&setup.settings),
                hints: context.hints(&setup.settings),
                standing_instruction: None,
                prefix: None,
            };
            let outcome = rt.block_on(setup.pipeline.run(job))?;
            report::print(&outcome);
        }
        Command::Transcribe {
            file,
            instruction_file,
            selection,
            raw,
            whisper,
            denoise,
            providers,
            context,
        } => {
            let setup = Setup::new(&providers)?;
            let cleanup = splashdown_audio::Cleanup {
                denoise,
                quiet: whisper,
            };
            let speech = load_speech(&file, cleanup)?.context("no speech found in the file")?;
            let instruction_speech = match &instruction_file {
                Some(f) => load_speech(f, cleanup)?,
                None => None,
            };
            let mode = mode_for(raw, selection.is_some(), instruction_speech.is_some());
            let job = AudioJob {
                mode,
                speech,
                instruction_speech,
                selection,
                context: context.prompt_context(&setup.settings),
                hints: context.hints(&setup.settings),
                standing_instruction: None,
                prefix: None,
            };
            let outcome = rt.block_on(setup.pipeline.run(job))?;
            report::print(&outcome);
        }
        Command::Interpret {
            spoken,
            last,
            providers,
            context,
        } => {
            let setup = Setup::new_text_only(&providers)?;
            let out = rt.block_on(setup.pipeline.command(
                &spoken,
                last.as_deref(),
                &context.prompt_context(&setup.settings),
            ))?;
            eprintln!(
                "── timing      ai {:.2}s ({})",
                out.llm_time.as_secs_f32(),
                out.llm_name
            );
            println!("{:?}", out.command);
        }
        Command::Clean {
            text,
            instruction,
            selection,
            providers,
            context,
        } => {
            let setup = Setup::new_text_only(&providers)?;
            let job = match selection {
                Some(selection) => TextJob {
                    mode: Mode::Transform,
                    content: String::new(),
                    instruction: Some(text),
                    selection: Some(selection),
                    context: context.prompt_context(&setup.settings),
                },
                None => TextJob {
                    mode: if instruction.is_some() {
                        Mode::DictateInstruct
                    } else {
                        Mode::Dictate
                    },
                    content: text,
                    instruction,
                    selection: None,
                    context: context.prompt_context(&setup.settings),
                },
            };
            let outcome = rt.block_on(setup.pipeline.process_text(job))?;
            report::print(&outcome);
        }
        Command::Devices => {
            let inputs = splashdown_audio::devices::inputs();
            let auto = splashdown_audio::devices::choose(None, &inputs);
            for i in &inputs {
                let mut tags = Vec::new();
                if i.is_default {
                    tags.push("system default");
                }
                if i.built_in {
                    tags.push("built-in");
                }
                if i.bluetooth {
                    tags.push("bluetooth");
                }
                println!("{:<32} {}", i.name, tags.join(", "));
            }
            println!(
                "\nAutomatic would use: {}",
                auto.as_deref().unwrap_or("the system default")
            );
            // The recorder opens devices by cpal's names; they should match.
            let cpal_names = splashdown_audio::input_devices()?;
            for i in &inputs {
                if !cpal_names.contains(&i.name) {
                    println!("warning: {:?} isn't in cpal's device list", i.name);
                }
            }
        }
        Command::Models { command } => {
            let models_dir = opts::paths()?.models_dir;
            match command {
                ModelsCommand::List => {
                    let state = |downloaded| {
                        if downloaded {
                            "downloaded"
                        } else {
                            "not downloaded"
                        }
                    };
                    println!("Speech to text (local Whisper):");
                    for m in WhisperModel::ALL {
                        let s = state(m.is_downloaded(&models_dir));
                        println!("  {:<22} ~{:>4} MB  {s}", m.id(), m.approx_mb());
                    }
                    println!("Cleanup (built-in local model, `--ai local`):");
                    for m in LocalModel::ALL {
                        let s = state(m.is_downloaded(&models_dir));
                        println!("  {:<22} ~{:>4} MB  {s}", m.id(), m.approx_mb());
                    }
                    println!("\nModels folder: {}", models_dir.display());
                }
                ModelsCommand::Download { id } => {
                    if WhisperModel::from_id(&id).is_none() && LocalModel::from_id(&id).is_none() {
                        bail!("unknown model \"{id}\" — see `splashdown-cli models list`");
                    }
                    let path = rt.block_on(splashdown_providers::models::download_by_id(
                        &id,
                        &models_dir,
                        report::download_progress,
                    ))?;
                    eprintln!();
                    println!("{}", path.display());
                }
            }
        }
        Command::Config { providers } => opts::print_config(&providers)?,
        Command::Eval { command } => rt.block_on(eval::run(command))?,
        Command::Toggle
        | Command::Start
        | Command::Stop
        | Command::Cancel
        | Command::Listen { .. } => unreachable!("handled above"),
    }
    Ok(())
}

fn mode_for(raw: bool, has_selection: bool, has_instruction: bool) -> Mode {
    match (raw, has_selection, has_instruction) {
        (true, _, _) => Mode::Raw,
        (_, true, _) => Mode::Transform,
        (_, _, true) => Mode::DictateInstruct,
        _ => Mode::Dictate,
    }
}

fn load_speech(
    path: &std::path::Path,
    cleanup: splashdown_audio::Cleanup,
) -> Result<Option<splashdown_core::AudioClip>> {
    let clip =
        splashdown_audio::wav::read(path).with_context(|| format!("reading {}", path.display()))?;
    let start = std::time::Instant::now();
    let prepared = splashdown_audio::prepare_with(&clip, cleanup);
    // For comparing the cost of --denoise and --whisper.
    eprintln!(
        "prepared {:.1} s of audio in {} ms",
        clip.duration().as_secs_f32(),
        start.elapsed().as_millis()
    );
    Ok(prepared)
}
