//! The developer harness: the pipeline, evals, models, recording and
//! transcription. Only in the full build (the default features); the
//! control-only build that ships with the app (`--no-default-features`)
//! has just the commands that talk to the running app or its settings.

use super::*;

#[derive(Subcommand)]
pub(crate) enum DevCommand {
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
        /// Also write subtitles (SubRip) to this file.
        #[arg(long)]
        srt: Option<PathBuf>,
        /// Also write subtitles (WebVTT) to this file.
        #[arg(long)]
        vtt: Option<PathBuf>,
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
    /// Run an instruction on a whole text file (a meeting's note, a
    /// transcript), as a hook that asks the AI does, and print what it
    /// writes. A text too long for the model is read in parts.
    Instruct {
        /// What the AI should do, e.g. "List what I said I'd do."
        instruction: String,
        /// The text: a Markdown or plain-text file.
        #[arg(long)]
        file: PathBuf,
        /// What the text is, for the prompt.
        #[arg(
            long,
            default_value = "the note of a meeting they were in, with its transcript"
        )]
        what: String,
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
    /// Read text aloud with Phonon 31M (`models download phonon-en`
    /// first), and print how long it took.
    #[cfg(feature = "natural-voice")]
    Say {
        text: String,
        /// One of the voices (`--voice list` lists them).
        #[arg(long)]
        voice: Option<String>,
    },
    /// Waits, then prints the text Compose would read from the window
    /// around the focused field (for testing how well an app exposes it).
    #[command(hide = true)]
    WindowText {
        /// Seconds to wait, to click into a field first.
        #[arg(long, default_value_t = 5)]
        after: u64,
    },
    /// Show the resolved configuration (never prints keys).
    Config {
        #[command(flatten)]
        providers: ProviderOpts,
    },
    /// Have Claude do a task on screen, with Splashdown's checks (an
    /// exploration; macOS only). Needs an Anthropic API key and Screen
    /// Recording and Accessibility for the terminal.
    #[cfg(target_os = "macos")]
    Computer(computer::ComputerArgs),
    /// Score providers and models against the eval sets in `eval/`.
    Eval {
        #[command(subcommand)]
        command: eval::EvalCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum ModelsCommand {
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

// `record`'s paths are bigger on Windows, which is all that sets the
// variants apart; it's parsed once.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub(crate) enum MeetingDev {
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

/// What the pipeline commands share: keys from `.env.local`, settings
/// under the app's former name moved, the speech detector, a runtime.
fn runtime() -> Result<tokio::runtime::Runtime> {
    // A missing .env.local is fine; keys may come from the environment.
    let _ = dotenvy::from_filename(".env.local");
    // Settings and models saved under the app's former name.
    if let Some(paths) = splashdown_config::Paths::resolve() {
        paths.migrate_legacy();
        // The commands from here on may work with audio.
        splashdown_providers::factory::use_speech_detector(&paths.models_dir);
    }
    Ok(tokio::runtime::Runtime::new()?)
}

pub(crate) fn run(command: DevCommand) -> Result<()> {
    let rt = runtime()?;
    match command {
        DevCommand::Record {
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
        DevCommand::Transcribe {
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
        DevCommand::TranscribeFile {
            file,
            whisper,
            denoise,
            srt,
            vtt,
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
                Subtitles { srt, vtt },
            )?;
        }
        DevCommand::Interpret {
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
                    ..splashdown_core::action::PlanContext::at(chrono::Local::now().naive_local())
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
        DevCommand::Ask {
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
                today: splashdown_core::prompt::today(chrono::Local::now().date_naive()),
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
        DevCommand::Instruct {
            instruction,
            file,
            what,
            providers,
        } => {
            let setup = Setup::new_text_only(&providers)?;
            let model = setup
                .pipeline
                .answer_model()
                .context("an instruction needs an AI model (--ai)")?;
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("couldn't read {}", file.display()))?;
            let about = splashdown_core::instruct::About {
                what,
                title: None,
                user_name: setup.settings.general.user_name.clone(),
                today: splashdown_core::prompt::today(chrono::Local::now().date_naive()),
            };
            let started = std::time::Instant::now();
            let done = rt
                .block_on(splashdown_core::instruct::run(
                    model.as_ref(),
                    &instruction,
                    &text,
                    &about,
                ))
                .map_err(|failed| anyhow::anyhow!(failed.error))?;
            eprintln!(
                "── {} · about {} tokens of text, read in {} part(s) · {:.1}s",
                model.name(),
                splashdown_core::prompt::estimated_tokens(&text),
                done.parts,
                started.elapsed().as_secs_f32()
            );
            if done.text.is_empty() {
                println!("(nothing to do)");
            } else {
                println!("{}", done.text);
            }
        }
        DevCommand::Clean {
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
        DevCommand::Devices => {
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
        DevCommand::Models { command } => {
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
                    let id = splashdown_providers::NaturalVoice::ID;
                    let s = state(splashdown_providers::NaturalVoice::is_downloaded(
                        &models_dir,
                    ));
                    println!("Text to speech (Phonon 31M, English):");
                    let mb = splashdown_providers::NaturalVoice::APPROX_MB;
                    println!("  {id:<22} ~{mb:>4} MB  {s}");
                    println!("\nModels folder: {}", models_dir.display());
                }
                ModelsCommand::Download { id, language } if id == "apple" => {
                    rt.block_on(splashdown_providers::apple::prepare_speech(Some(&language)))
                        .map_err(anyhow::Error::msg)?;
                    println!("Apple's speech model for {language} is ready.");
                }
                ModelsCommand::Download { id, .. } => {
                    if SpeechModel::from_id(&id).is_none()
                        && LocalModel::from_id(&id).is_none()
                        && id != splashdown_providers::NaturalVoice::ID
                    {
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
        #[cfg(feature = "natural-voice")]
        DevCommand::Say { text, voice } => say(&text, voice.as_deref())?,
        DevCommand::WindowText { after } => {
            eprintln!("Click into a text field; reading in {after} s…");
            std::thread::sleep(std::time::Duration::from_secs(after));
            let Some(focus) = splashdown_platform::Focus::capture() else {
                anyhow::bail!(
                    "couldn't see the focused field (does this terminal have Accessibility permission?)"
                );
            };
            eprintln!(
                "field: {} characters of its own · selection: {:?}",
                focus.field_text().map_or(0, |t| t.chars().count()),
                focus.selection()
            );
            let started = std::time::Instant::now();
            let text = focus.surrounding_text(
                splashdown_core::compose::WINDOW_MIN_CHARS,
                splashdown_core::compose::WINDOW_MAX_CHARS,
            );
            eprintln!(
                "{} · {} ms · {} characters",
                focus.app.name,
                started.elapsed().as_millis(),
                text.as_deref().map_or(0, |t| t.chars().count())
            );
            println!("{}", text.unwrap_or_default());
        }
        DevCommand::Config { providers } => opts::print_config(&providers)?,
        DevCommand::Eval { command } => rt.block_on(eval::run(command))?,
        #[cfg(target_os = "macos")]
        DevCommand::Computer(args) => computer::run(&rt, args)?,
    }
    Ok(())
}

pub(crate) fn meeting(command: MeetingDev) -> Result<()> {
    let rt = runtime()?;
    match command {
        MeetingDev::Record {
            out,
            seconds,
            device,
        } => record_meeting(&out, seconds, device.as_deref())?,
        MeetingDev::Transcribe {
            file,
            providers,
            context,
        } => {
            let setup = Setup::new_speech_only(&providers)?;
            transcribe_meeting(
                &rt,
                &file,
                setup.pipeline.stt().as_ref(),
                &context.hints(&setup.settings),
            )?;
        }
    }
    Ok(())
}
