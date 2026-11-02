//! `splashdown-cli` — developer harness for the Splashdown pipeline (milestone M1),
//! and a remote control for the running app (`splashdown-cli toggle`, `splashdown-cli
//! listen`, … over its control socket; see `splashdown-control`).
//!
//! API keys come from the environment or a `.env.local` file in the current
//! directory (`OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `SPLASHDOWN_CUSTOM_API_KEY`).

mod eval;
mod manage;
mod opts;
mod record;
mod report;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use splashdown_control::Request;
use splashdown_core::{AudioJob, Mode, TextJob};
use splashdown_providers::{LocalModel, SpeechModel};

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
    /// Transcribe an audio or video file of any length, a piece at a time,
    /// and print it as Markdown, as Notes does (no AI pass).
    #[command(name = "transcribe-file")]
    TranscribeFile {
        file: PathBuf,
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
        /// Voice actions on: a request to do something ("open Spotify") is
        /// planned into an action and printed, not carried out.
        #[arg(long)]
        actions: bool,
        /// Selected text an action may use (an email's body).
        #[arg(long)]
        selection: Option<String>,
        #[command(flatten)]
        providers: ProviderOpts,
        #[command(flatten)]
        context: ContextOpts,
    },
    /// Answer a question as the Ask panel does (dictation key + A),
    /// streaming the answer. Notes aren't searched.
    Ask {
        question: String,
        /// Text it's about, as if selected.
        #[arg(long)]
        selection: Option<String>,
        #[command(flatten)]
        providers: ProviderOpts,
    },
    /// Run only the AI pass on text, as if it had been dictated.
    Clean {
        text: String,
        #[arg(long)]
        instruction: Option<String>,
        /// Transform this text with `text` as the instruction.
        #[arg(long)]
        selection: Option<String>,
        /// Show the AI's text as it streams in (on stderr), with the time
        /// each piece arrived.
        #[arg(long)]
        stream: bool,
        #[command(flatten)]
        providers: ProviderOpts,
        #[command(flatten)]
        context: ContextOpts,
    },
    /// List microphones.
    Devices,
    /// Manage downloadable models (local speech-to-text, the built-in text model).
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

// `record`'s paths are bigger on Windows, which is all that sets the
// variants apart; it's parsed once.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum MeetingCommand {
    /// Start recording a meeting in the running app.
    Start,
    /// Stop the meeting; the app finishes its note.
    Stop,
    /// The meeting in progress, as JSON (`null` when there's none).
    Status,
    /// Record a meeting for a while: the microphone (You, left channel)
    /// and what the computer plays (Them, right channel), as a 16 kHz
    /// stereo WAV for `meeting transcribe`.
    Record {
        out: PathBuf,
        /// How long to record.
        #[arg(long, default_value_t = 30)]
        seconds: u64,
        /// Microphone name (default: the system's).
        #[arg(long)]
        device: Option<String>,
    },
    /// Transcribe a recorded meeting, a stereo file with You (the
    /// microphone) on the left and Them (the computer's sound) on the
    /// right, and print the transcript as Markdown.
    Transcribe {
        file: PathBuf,
        #[command(flatten)]
        providers: ProviderOpts,
        #[command(flatten)]
        context: ContextOpts,
    },
}

#[derive(Subcommand)]
enum ModelsCommand {
    /// List models and whether they're downloaded.
    List,
    /// Download a model, e.g. `nemotron-en`. `apple` asks macOS for Apple's
    /// speech model for `--language` (from Apple).
    Download {
        id: String,
        /// For `apple`: the language (ISO 639-1), English by default.
        #[arg(long, default_value = "en")]
        language: String,
    },
    /// Use a downloaded model: a speech model for speech-to-text on this
    /// computer, or a text model for the AI.
    Use {
        id: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
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
                    println!("{}", opts::paths()?.config_file().display());
                    Ok(())
                }
            };
        }
        _ => {}
    }

    // A missing .env.local is fine; keys may come from the environment.
    let _ = dotenvy::from_filename(".env.local");
    // Settings and models saved under the app's former name.
    if let Some(paths) = splashdown_config::Paths::resolve() {
        paths.migrate_legacy();
        // The commands from here on may work with audio.
        splashdown_providers::factory::use_speech_detector(&paths.models_dir);
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
            let cleanup = splashdown_audio::Cleanup {
                denoise,
                quiet: whisper,
            };
            // Before loading models: no point (and a local one would still
            // be loading as the program ends).
            let speech = load_speech(&file, cleanup)?.context("no speech found in the file")?;
            let instruction_speech = match &instruction_file {
                Some(f) => load_speech(f, cleanup)?,
                None => None,
            };
            let setup = Setup::new(&providers)?;
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
        Command::TranscribeFile {
            file,
            whisper,
            denoise,
            providers,
            context,
        } => {
            let setup = Setup::new_speech_only(&providers)?;
            let cleanup = splashdown_audio::Cleanup {
                denoise,
                quiet: whisper,
            };
            transcribe_file(
                &rt,
                &file,
                setup.pipeline.stt().as_ref(),
                &context.hints(&setup.settings),
                cleanup,
            )?;
        }
        Command::Words {
            command:
                WordsCommand::Import {
                    file,
                    dry_run,
                    config,
                },
        } => import_words(&file, dry_run, config)?,
        Command::Meeting {
            command:
                MeetingCommand::Record {
                    out,
                    seconds,
                    device,
                },
        } => record_meeting(&out, seconds, device.as_deref())?,
        Command::Meeting {
            command:
                MeetingCommand::Transcribe {
                    file,
                    providers,
                    context,
                },
        } => {
            let setup = Setup::new_speech_only(&providers)?;
            transcribe_meeting(
                &rt,
                &file,
                setup.pipeline.stt().as_ref(),
                &context.hints(&setup.settings),
            )?;
        }
        Command::Interpret {
            spoken,
            last,
            actions,
            selection,
            providers,
            context,
        } => {
            let setup = Setup::new_text_only(&providers)?;
            let out = rt.block_on(setup.pipeline.command(
                &spoken,
                last.as_deref(),
                &context.prompt_context(&setup.settings),
                actions,
            ))?;
            eprintln!(
                "── timing      ai {:.2}s ({})",
                out.llm_time.as_secs_f32(),
                out.llm_name
            );
            println!("{:?}", out.command);
            if let splashdown_core::command::Command::Do(request) = &out.command {
                let ctx = splashdown_core::action::PlanContext {
                    selection,
                    ..eval::plan_context(chrono::Local::now().naive_local())
                };
                let plan = rt.block_on(setup.pipeline.plan_action(request, &ctx, None))?;
                eprintln!(
                    "── timing      plan {:.2}s ({})",
                    plan.llm_time.as_secs_f32(),
                    plan.llm_name
                );
                println!("{:?}", plan.plan);
            }
        }
        Command::Ask {
            question,
            selection,
            providers,
        } => {
            let setup = Setup::new_text_only(&providers)?;
            let model = setup
                .pipeline
                .answer_model()
                .context("asking needs an AI model (--ai)")?;
            let context = splashdown_core::ask::AskContext {
                selection,
                user_name: setup.settings.general.user_name.clone(),
                today: chrono::Local::now().format("%A, %-d %B %Y").to_string(),
                ..Default::default()
            };
            let request = splashdown_core::ask::request(&question, &context);
            let mut shown = 0;
            let mut show = |so_far: &str| {
                // Only what's new since the last piece.
                if let Some(new) = so_far.get(shown..) {
                    print!("{new}");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
                shown = so_far.len();
            };
            let completion = rt.block_on(model.complete_streaming(&request, &mut show))?;
            if shown == 0 {
                print!("{}", completion.text);
            }
            println!();
        }
        Command::Clean {
            text,
            instruction,
            selection,
            stream,
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
            let started = std::time::Instant::now();
            let mut show = |so_far: &str| {
                eprintln!("{:>6.2}s  {so_far}", started.elapsed().as_secs_f32());
            };
            let on_text = stream.then_some(&mut show as &mut splashdown_core::OnText<'_>);
            let outcome = rt.block_on(setup.pipeline.process_text_streaming(job, on_text))?;
            report::print(&outcome);
        }
        Command::Devices => {
            let inputs = splashdown_audio::devices::inputs();
            let auto = splashdown_audio::devices::choose(&[], &inputs);
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
                    println!("Speech to text (local, `--stt local`):");
                    for m in SpeechModel::ALL {
                        let s = state(m.is_downloaded(&models_dir));
                        println!(
                            "  {:<22} ~{:>4} MB  {s:<15} {} ({})",
                            m.id(),
                            m.approx_mb(),
                            m.label(),
                            m.license()
                        );
                    }
                    println!("Cleanup (built-in local model, `--ai local`):");
                    for m in LocalModel::ALL {
                        let s = state(m.is_downloaded(&models_dir));
                        println!("  {:<22} ~{:>4} MB  {s}", m.id(), m.approx_mb());
                    }
                    println!("\nModels folder: {}", models_dir.display());
                }
                ModelsCommand::Download { id, language } if id == "apple" => {
                    rt.block_on(splashdown_providers::apple::prepare_speech(Some(&language)))
                        .map_err(anyhow::Error::msg)?;
                    println!("Apple's speech model for {language} is ready.");
                }
                ModelsCommand::Download { id, .. } => {
                    if SpeechModel::from_id(&id).is_none() && LocalModel::from_id(&id).is_none() {
                        return Err(manage::CliError::NotFound(format!(
                            "unknown model \"{id}\" — see `splashdown-cli models list`"
                        ))
                        .into());
                    }
                    let path = rt.block_on(splashdown_providers::models::download_by_id(
                        &id,
                        &models_dir,
                        report::download_progress,
                    ))?;
                    eprintln!();
                    println!("{}", path.display());
                }
                ModelsCommand::Use { id, config } => manage::use_model(&id, config)?,
            }
        }
        Command::Config { providers } => opts::print_config(&providers)?,
        Command::Eval { command } => rt.block_on(eval::run(command))?,
        Command::Toggle
        | Command::Start
        | Command::Stop
        | Command::Cancel
        | Command::Listen { .. }
        | Command::Note { .. }
        | Command::Status { .. }
        | Command::History { .. }
        | Command::Settings { .. }
        | Command::Meeting {
            command: MeetingCommand::Start | MeetingCommand::Stop | MeetingCommand::Status,
        } => unreachable!("handled above"),
    }
    Ok(())
}

/// Prints the file's transcript as it goes: a title, then a paragraph per
/// piece starting with its time. Timings go to stderr.
fn transcribe_file(
    rt: &tokio::runtime::Runtime,
    path: &std::path::Path,
    stt: &dyn splashdown_core::SpeechToText,
    hints: &splashdown_core::SttHints,
    cleanup: splashdown_audio::Cleanup,
) -> Result<()> {
    use splashdown_audio::file::{AudioFile, timestamp};

    let mut file = AudioFile::open(path).with_context(|| format!("reading {}", path.display()))?;
    let length = file.duration();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    println!("# {stem}");
    let start = std::time::Instant::now();
    let mut audio = std::time::Duration::ZERO;
    while let Some(piece) = file.next_piece()? {
        audio = piece.start + piece.clip.duration();
        let Some(clip) = splashdown_audio::prepare_with(&piece.clip, cleanup) else {
            continue;
        };
        let text = rt.block_on(stt.transcribe(&clip, hints))?.text;
        if !text.trim().is_empty() {
            println!("\n**[{}]** {}", timestamp(piece.start), text.trim());
        }
    }
    eprintln!(
        "transcribed {:.0} s of audio (the file says {}) in {:.1} s with {}",
        audio.as_secs_f64(),
        length.map_or("nothing".into(), |d| format!("{:.0} s", d.as_secs_f64())),
        start.elapsed().as_secs_f64(),
        stt.name()
    );
    Ok(())
}

/// Merges another app's words into the config file and tells the running
/// app to read it again.
fn import_words(file: &std::path::Path, dry_run: bool, config: Option<PathBuf>) -> Result<()> {
    use splashdown_config::import;

    let imported = import::read(file).with_context(|| format!("importing {}", file.display()))?;
    let own_config = config.is_none();
    let path = match config {
        Some(p) => p,
        None => opts::paths()?.config_file(),
    };
    let mut settings = splashdown_config::Settings::load(&path)?;
    let merged = import::merge(&mut settings, &imported);
    let added = merged.vocabulary + merged.replacements + merged.snippets;
    println!(
        "{}: {} vocabulary word(s), {} correction(s), {} snippet(s) to add",
        imported.kind, merged.vocabulary, merged.replacements, merged.snippets
    );
    if merged.duplicates > 0 {
        println!("{} already there", merged.duplicates);
    }
    if merged.conflicts > 0 {
        println!(
            "{} kept as you have them (the file says something different)",
            merged.conflicts
        );
    }
    if imported.skipped > 0 {
        println!("{} line(s) or entries couldn't be used", imported.skipped);
    }
    if dry_run || added == 0 {
        if dry_run {
            println!("Nothing saved (--dry-run).");
        }
        return Ok(());
    }
    settings.save(&path).map_err(anyhow::Error::msg)?;
    if !own_config {
        println!("Saved to {}.", path.display());
        return Ok(());
    }
    match splashdown_control::send(&Request::ReloadSettings) {
        Ok(_) => println!("Saved; Splashdown is using them now."),
        Err(_) => println!("Saved; Splashdown will use them when it starts."),
    }
    Ok(())
}

/// What the computer plays, as a live recording.
fn system_audio() -> Result<splashdown_audio::live::LiveAudio> {
    #[cfg(target_os = "macos")]
    return splashdown_apple_shim::system_audio().map_err(anyhow::Error::msg);
    #[cfg(windows)]
    return Ok(splashdown_audio::live::LiveAudio::system_output()?);
    #[cfg(not(any(target_os = "macos", windows)))]
    anyhow::bail!("recording the computer's sound isn't supported here");
}

/// Records both sides of a meeting into a stereo WAV.
fn record_meeting(out: &std::path::Path, seconds: u64, device: Option<&str>) -> Result<()> {
    use splashdown_audio::live::LiveAudio;

    let mut you = LiveAudio::microphone(device)?;
    let mut them = system_audio()?;
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let end = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut loudest = (0.0f32, 0.0f32);
    while std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(200));
        left.extend(you.take());
        right.extend(them.take());
        loudest = (loudest.0.max(you.level()), loudest.1.max(them.level()));
        eprint!(
            "\rrecording {:>3} s · You {:.3} · Them {:.3}   ",
            left.len() / 16_000,
            you.level(),
            them.level()
        );
    }
    eprintln!();
    eprintln!(
        "audio delivered: You {:.1} s, Them {:.1} s",
        left.len() as f64 / 16_000.0,
        right.len() as f64 / 16_000.0
    );
    let frames = left.len().max(right.len());
    left.resize(frames, 0.0);
    right.resize(frames, 0.0);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: splashdown_audio::TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(out, spec)?;
    for (l, r) in left.iter().zip(&right) {
        for s in [l, r] {
            writer.write_sample((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)?;
        }
    }
    writer.finalize()?;
    eprintln!(
        "wrote {:.1} s to {} (loudest: You {:.3}, Them {:.3}{})",
        frames as f64 / 16_000.0,
        out.display(),
        loudest.0,
        loudest.1,
        if loudest.1 == 0.0 {
            "; Them was silent: nothing played, or recording the computer's sound isn't allowed"
        } else {
            ""
        }
    );
    Ok(())
}

/// Transcribes both sides of a meeting recording a piece at a time, oldest
/// first, and prints each paragraph once it's final.
fn transcribe_meeting(
    rt: &tokio::runtime::Runtime,
    path: &std::path::Path,
    stt: &dyn splashdown_core::SpeechToText,
    hints: &splashdown_core::SttHints,
) -> Result<()> {
    use splashdown_audio::file::{AudioFile, Piece, timestamp};
    use splashdown_core::meeting::{Meeting, Paragraph, Segment, Side};

    let open = |channel| {
        AudioFile::open_turns(path, channel).with_context(|| format!("reading {}", path.display()))
    };
    let mut sides = [(Side::You, open(0)?), (Side::Them, open(1)?)];
    let mut next: [Option<Piece>; 2] = [sides[0].1.next_piece()?, sides[1].1.next_piece()?];
    let print = |paragraphs: Vec<Paragraph>| {
        for p in paragraphs {
            println!(
                "\n**[{}] {}:** {}",
                timestamp(p.start),
                p.side.label(),
                p.text
            );
        }
    };
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    println!("# {stem}");
    let start = std::time::Instant::now();
    let mut meeting = Meeting::new();
    // The side whose next piece starts first goes next, so the transcript
    // moves forward evenly.
    while let Some(i) = (0..2)
        .filter(|&i| next[i].is_some())
        .min_by_key(|&i| next[i].as_ref().map(|p| p.start))
    {
        let piece = next[i].take().expect("chosen because it's there");
        let end = piece.start + piece.clip.duration();
        let text = match splashdown_audio::prepare(&piece.clip) {
            Some(clip) => rt.block_on(stt.transcribe(&clip, hints))?.text,
            None => String::new(),
        };
        meeting.push(Segment {
            side: sides[i].0,
            start: piece.start,
            end,
            text,
        });
        next[i] = sides[i].1.next_piece()?;
        if next[i].is_none() {
            // This side is over: nothing more will come from it.
            meeting.reached(sides[i].0, std::time::Duration::MAX);
        }
        print(meeting.ready());
    }
    let echoes = meeting.echoes;
    print(meeting.finish());
    eprintln!(
        "transcribed in {:.1} s with {}; {echoes} piece(s) dropped as echo",
        start.elapsed().as_secs_f64(),
        stt.name()
    );
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
