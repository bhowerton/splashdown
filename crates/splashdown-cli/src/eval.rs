//! Eval harness: scores the AI pass and speech-to-text so model
//! defaults are chosen from data, not guesses.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::Deserialize;
use splashdown_config::Secrets;
use splashdown_core::action::{Action, Mine, Plan, PlanContext};
use splashdown_core::command::Command;
use splashdown_core::{Fallback, Mode, Pipeline, PromptContext, TextJob};

use crate::opts::{self, ProviderOpts};

#[derive(Subcommand)]
pub enum EvalCommand {
    /// Score the AI pass on transcripts in eval/cleanup.toml.
    Cleanup {
        #[arg(long, default_value = "eval/cleanup.toml")]
        cases: PathBuf,
        /// Only run cases whose id contains this.
        #[arg(long)]
        filter: Option<String>,
        /// Print every output, not just failures.
        #[arg(short, long)]
        verbose: bool,
        /// Compare several models of the same provider, comma-separated.
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[command(flatten)]
        providers: ProviderOpts,
    },
    /// Score spoken commands on eval/commands.toml, with voice actions
    /// off and then on.
    Commands {
        #[arg(long, default_value = "eval/commands.toml")]
        cases: PathBuf,
        #[arg(long)]
        filter: Option<String>,
        #[arg(short, long)]
        verbose: bool,
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[command(flatten)]
        providers: ProviderOpts,
    },
    /// Score voice actions on eval/actions.toml: each command is
    /// interpreted, then planned into an action.
    Actions {
        #[arg(long, default_value = "eval/actions.toml")]
        cases: PathBuf,
        #[arg(long)]
        filter: Option<String>,
        #[arg(short, long)]
        verbose: bool,
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[command(flatten)]
        providers: ProviderOpts,
    },
    /// Score speech-to-text (word error rate) on eval/stt.toml.
    Stt {
        #[arg(long, default_value = "eval/stt.toml")]
        cases: PathBuf,
        #[arg(long, default_value = "eval/audio")]
        audio_dir: PathBuf,
        #[arg(short, long)]
        verbose: bool,
        /// Let the model detect each clip's language instead of telling it
        /// (the app's "Detect automatically").
        #[arg(long)]
        detect_language: bool,
        #[command(flatten)]
        providers: ProviderOpts,
    },
    /// Synthesize eval/audio/*.wav from eval/stt.toml with macOS `say`.
    MakeAudio {
        #[arg(long, default_value = "eval/stt.toml")]
        cases: PathBuf,
        #[arg(long, default_value = "eval/audio")]
        audio_dir: PathBuf,
        #[arg(long, default_value = "Samantha")]
        voice: String,
    },
}

pub async fn run(cmd: EvalCommand) -> Result<()> {
    match cmd {
        EvalCommand::Cleanup {
            cases,
            filter,
            verbose,
            models,
            providers,
        } => cleanup(&cases, filter.as_deref(), verbose, &models, &providers).await,
        EvalCommand::Commands {
            cases,
            filter,
            verbose,
            models,
            providers,
        } => {
            let file: CommandFile = read_cases(&cases)?;
            let cases = filtered(&file.case, filter.as_deref(), |c| &c.id);
            for run in model_runs(&models, &providers) {
                commands(&cases, verbose, &run).await?;
            }
            Ok(())
        }
        EvalCommand::Actions {
            cases,
            filter,
            verbose,
            models,
            providers,
        } => {
            let file: ActionFile = read_cases(&cases)?;
            let cases = filtered(&file.case, filter.as_deref(), |c| &c.id);
            for run in model_runs(&models, &providers) {
                actions(&cases, verbose, &run).await?;
            }
            Ok(())
        }
        EvalCommand::Stt {
            cases,
            audio_dir,
            verbose,
            detect_language,
            providers,
        } => stt(&cases, &audio_dir, verbose, detect_language, &providers).await,
        EvalCommand::MakeAudio {
            cases,
            audio_dir,
            voice,
        } => make_audio(&cases, &audio_dir, &voice),
    }
}

// ---------------------------------------------------------------- cleanup

#[derive(Deserialize)]
struct CleanupFile {
    case: Vec<CleanupCase>,
}

#[derive(Deserialize)]
struct CleanupCase {
    id: String,
    /// Dictated content (unused for transforms).
    #[serde(default)]
    transcript: String,
    instruction: Option<String>,
    selection: Option<String>,
    app: Option<String>,
    preset: Option<String>,
    /// Text already in the field around the cursor (field context).
    context: Option<String>,
    /// Earlier dictations into the same app, oldest first.
    #[serde(default)]
    earlier: Vec<String>,
    /// A built-in transform's id: its instruction is used on `selection`.
    transform: Option<String>,
    /// Case-insensitive substrings the output must contain.
    #[serde(default)]
    expect: Vec<String>,
    /// Case-insensitive words/phrases the output must not contain.
    #[serde(default)]
    reject: Vec<String>,
    /// Case-insensitive text it must not contain, punctuation included
    /// ("- [ ]", "(Dana)").
    #[serde(default)]
    reject_text: Vec<String>,
    min_lines: Option<usize>,
    max_chars: Option<usize>,
}

impl CleanupCase {
    fn job(&self) -> TextJob {
        let builtin = self.transform.as_deref().map(|id| {
            splashdown_config::builtin_transforms()
                .into_iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("no built-in transform {id:?}"))
                .instruction
        });
        let instruction = builtin.or_else(|| self.instruction.clone());
        let mode = match (&self.selection, &instruction) {
            (Some(_), _) => Mode::Transform,
            (None, Some(_)) => Mode::DictateInstruct,
            (None, None) => Mode::Dictate,
        };
        TextJob {
            mode,
            content: self.transcript.clone(),
            instruction,
            selection: self.selection.clone(),
            context: PromptContext {
                preset_instructions: self.preset.clone(),
                app_name: self.app.clone(),
                field_text: self.context.clone(),
                earlier: self.earlier.clone(),
                user_name: Some("Alex".into()),
                vocabulary: vec!["Tauri".into(), "OpenRouter".into(), "Kubernetes".into()],
                ..PromptContext::default()
            },
        }
    }

    fn failures(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let lower = text.to_lowercase();
        for e in &self.expect {
            if !lower.contains(&e.to_lowercase()) {
                out.push(format!("missing {e:?}"));
            }
        }
        let output_words = words(text);
        for r in &self.reject {
            let phrase = words(r);
            if !phrase.is_empty()
                && output_words
                    .windows(phrase.len())
                    .any(|w| w == phrase.as_slice())
            {
                out.push(format!("contains {r:?}"));
            }
        }
        for r in &self.reject_text {
            if lower.contains(&r.to_lowercase()) {
                out.push(format!("contains {r:?}"));
            }
        }
        if let Some(n) = self.min_lines {
            let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
            if lines < n {
                out.push(format!("{lines} lines, wanted ≥{n}"));
            }
        }
        if let Some(n) = self.max_chars
            && text.chars().count() > n
        {
            out.push(format!("{} chars, wanted ≤{n}", text.chars().count()));
        }
        out
    }
}

async fn cleanup(
    path: &Path,
    filter: Option<&str>,
    verbose: bool,
    models: &[String],
    providers: &ProviderOpts,
) -> Result<()> {
    let file: CleanupFile = toml::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    let cases: Vec<_> = file
        .case
        .iter()
        .filter(|c| filter.is_none_or(|f| c.id.contains(f)))
        .collect();

    let runs: Vec<ProviderOpts> = if models.is_empty() {
        vec![providers.clone()]
    } else {
        models
            .iter()
            .map(|m| ProviderOpts {
                ai_model: Some(m.clone()),
                ..providers.clone()
            })
            .collect()
    };

    let mut rows = Vec::new();
    for run in &runs {
        rows.push(cleanup_one(&cases, verbose, run).await?);
    }
    if rows.len() > 1 {
        println!(
            "\n{:<48} {:>7} {:>8} {:>6} {:>7} {:>7}",
            "model", "passed", "guarded", "errors", "p50", "p90"
        );
        for r in &rows {
            println!(
                "{:<48} {:>6.0}% {:>8} {:>6} {:>6.2}s {:>6.2}s",
                r.name,
                r.passed as f64 * 100.0 / r.total.max(1) as f64,
                r.guarded,
                r.errors,
                r.p50,
                r.p90
            );
        }
    }
    Ok(())
}

struct Row {
    name: String,
    passed: usize,
    total: usize,
    errors: usize,
    /// Failures the sanity guard caught, so the user got their raw words.
    guarded: usize,
    p50: f32,
    p90: f32,
}

async fn cleanup_one(
    cases: &[&CleanupCase],
    verbose: bool,
    providers: &ProviderOpts,
) -> Result<Row> {
    let settings = opts::settings(providers)?;
    let secrets = Secrets::from_env();
    let Some(llm) = opts::build_llm(&settings, &secrets)? else {
        bail!("the cleanup eval needs an AI provider (--ai …)");
    };
    let instruction_llm = opts::build_instruction_llm(&settings, &secrets)?;
    let name = match &instruction_llm {
        Some(cloud) => format!("{} + instructions: {}", llm.name(), cloud.name()),
        None => llm.name(),
    };
    let pipeline = Pipeline::new(std::sync::Arc::new(NoopStt), Some(llm))
        .with_instruction_model(instruction_llm);
    eprintln!("\nCleanup eval · {name} · {} cases\n", cases.len());

    // Untimed warm-up so a local model's load time doesn't skew latency.
    if let Some(first) = cases.first() {
        let _ = pipeline.process_text(first.job()).await;
    }

    let mut latencies = Vec::new();
    let mut passed = 0;
    let mut errors = 0;
    let mut guarded = 0;
    let mut cost = splashdown_core::Usage::default();
    for case in cases {
        let started = Instant::now();
        let result = pipeline.process_text(case.job()).await;
        let elapsed = started.elapsed();
        let (text, mut fails) = match result {
            Ok(outcome) => {
                cost += outcome.usage;
                let mut fails = Vec::new();
                match &outcome.fallback {
                    Some(Fallback::Guard(r)) => {
                        guarded += 1;
                        fails.push(format!("guard: {} (raw text inserted)", r.describe()));
                    }
                    Some(Fallback::ModelError(e)) => {
                        errors += 1;
                        fails.push(format!("error: {e}"));
                    }
                    None => {}
                }
                // What the app would put in the note.
                let text = match &case.transform {
                    Some(id) => splashdown_config::tidy_answer(id, &outcome.text),
                    None => outcome.text,
                };
                (text, fails)
            }
            Err(e) => {
                errors += 1;
                (String::new(), vec![format!("error: {e}")])
            }
        };
        if fails.is_empty() {
            fails = case.failures(&text);
        }
        latencies.push(elapsed);

        let ok = fails.is_empty();
        passed += usize::from(ok);
        println!(
            "{} {:<34} {:>5.2}s  {}",
            if ok { "✓" } else { "✗" },
            case.id,
            elapsed.as_secs_f32(),
            fails.join("; ")
        );
        if verbose || !ok {
            for line in text.lines() {
                println!("      │ {line}");
            }
        }
    }

    summary(&name, passed, cases.len(), errors, &mut latencies);
    println!("{name}: guard fell back to raw text on {guarded} case(s)");
    println!(
        "{name}: cost ${:.4} (estimated{})",
        cost.cost_usd,
        if cost.unpriced {
            "; some calls unpriced"
        } else {
            ""
        }
    );
    Ok(Row {
        name,
        passed,
        total: cases.len(),
        errors,
        guarded,
        p50: percentile(&latencies, 0.5),
        p90: percentile(&latencies, 0.9),
    })
}

// ---------------------------------------------------------------- commands

fn read_cases<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(toml::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )?)
}

fn filtered<'a, T>(cases: &'a [T], filter: Option<&str>, id: impl Fn(&T) -> &String) -> Vec<&'a T> {
    cases
        .iter()
        .filter(|c| filter.is_none_or(|f| id(c).contains(f)))
        .collect()
}

fn model_runs(models: &[String], providers: &ProviderOpts) -> Vec<ProviderOpts> {
    if models.is_empty() {
        return vec![providers.clone()];
    }
    models
        .iter()
        .map(|m| ProviderOpts {
            ai_model: Some(m.clone()),
            ..providers.clone()
        })
        .collect()
}

/// The pipeline for text-only evals, and its model's name.
fn text_pipeline(providers: &ProviderOpts) -> Result<(String, Pipeline)> {
    let settings = opts::settings(providers)?;
    let secrets = Secrets::from_env();
    let Some(llm) = opts::build_llm(&settings, &secrets)? else {
        bail!("this eval needs an AI provider (--ai …)");
    };
    let instruction_llm = opts::build_instruction_llm(&settings, &secrets)?;
    let name = match &instruction_llm {
        Some(cloud) => cloud.name(),
        None => llm.name(),
    };
    let pipeline = Pipeline::new(std::sync::Arc::new(NoopStt), Some(llm))
        .with_instruction_model(instruction_llm);
    Ok((name, pipeline))
}

/// One acceptable answer, or several.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMore {
    One(String),
    More(Vec<String>),
}

impl OneOrMore {
    fn list(&self) -> Vec<&str> {
        match self {
            OneOrMore::One(s) => vec![s.as_str()],
            OneOrMore::More(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Deserialize)]
struct CommandFile {
    case: Vec<CommandCase>,
}

#[derive(Deserialize)]
struct CommandCase {
    id: String,
    said: String,
    last: Option<String>,
    app: Option<String>,
    expect: OneOrMore,
    /// Case-insensitive substrings the command's text must contain.
    #[serde(default)]
    text: Vec<String>,
}

fn command_id(command: &Command) -> &'static str {
    match command {
        Command::ReplaceLast(_) => "replace_last",
        Command::DeleteLastWord => "delete_last_word",
        Command::DeleteLastSentence => "delete_last_sentence",
        Command::SelectLast => "select_last",
        Command::BoldLast => "bold_last",
        Command::ItalicLast => "italic_last",
        Command::PressEnter => "press_enter",
        Command::UndoLast => "undo_last",
        Command::Write(_) => "write",
        Command::Ask(_) => "ask",
        Command::Read(_) => "read",
        Command::Do(_) => "do",
        Command::Unclear => "unclear",
    }
}

fn command_text(command: &Command) -> &str {
    match command {
        Command::ReplaceLast(t)
        | Command::Write(t)
        | Command::Ask(t)
        | Command::Read(t)
        | Command::Do(t) => t,
        _ => "",
    }
}

async fn commands(cases: &[&CommandCase], verbose: bool, providers: &ProviderOpts) -> Result<()> {
    let (name, pipeline) = text_pipeline(providers)?;
    // Untimed warm-up so a local model's load time doesn't skew latency.
    if let Some(first) = cases.first() {
        let _ = pipeline
            .command(&first.said, None, &PromptContext::default(), false)
            .await;
    }
    for actions in [false, true] {
        let label = if actions { "actions on" } else { "actions off" };
        eprintln!("\nCommands eval · {name} · {label}\n");
        let mut latencies = Vec::new();
        let (mut passed, mut total, mut errors, mut false_actions) = (0, 0, 0, 0);
        let mut cost = splashdown_core::Usage::default();
        for case in cases {
            let expect: Vec<&str> = case
                .expect
                .list()
                .into_iter()
                .filter(|e| actions || *e != "do")
                .collect();
            if expect.is_empty() {
                continue;
            }
            total += 1;
            let ctx = PromptContext {
                app_name: case.app.clone(),
                ..PromptContext::default()
            };
            let started = Instant::now();
            let result = pipeline
                .command(&case.said, case.last.as_deref(), &ctx, actions)
                .await;
            latencies.push(started.elapsed());
            let (got, text, mut fails) = match result {
                Ok(out) => {
                    cost += out.usage;
                    let id = command_id(&out.command);
                    (id, command_text(&out.command).to_string(), Vec::new())
                }
                Err(e) => {
                    errors += 1;
                    ("error", String::new(), vec![format!("error: {e}")])
                }
            };
            if fails.is_empty() {
                if !expect.contains(&got) {
                    fails.push(format!("{got}, wanted {}", expect.join(" or ")));
                    if got == "do" {
                        false_actions += 1;
                        fails.push("FALSE ACTION".into());
                    }
                } else {
                    let lower = text.to_lowercase();
                    for t in &case.text {
                        if !lower.contains(&t.to_lowercase()) {
                            fails.push(format!("text missing {t:?}"));
                        }
                    }
                }
            }
            let ok = fails.is_empty();
            passed += usize::from(ok);
            println!(
                "{} {:<28} {:>5.2}s  {}",
                if ok { "✓" } else { "✗" },
                case.id,
                started.elapsed().as_secs_f32(),
                fails.join("; ")
            );
            if (verbose || !ok) && !text.is_empty() {
                println!("      │ {got}: {text}");
            }
        }
        summary(
            &format!("{name} · {label}"),
            passed,
            total,
            errors,
            &mut latencies,
        );
        println!(
            "{name} · {label}: {false_actions} false action(s) · cost ${:.4}",
            cost.cost_usd
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- actions

/// What the planner knows of now: fixed in the eval so dates can be
/// checked exactly.
pub fn plan_context(now: chrono::NaiveDateTime) -> PlanContext {
    PlanContext {
        now: now.format("%A %Y-%m-%d %H:%M").to_string(),
        coming_days: (1..=7)
            .map(|d| {
                (now.date() + chrono::Days::new(d))
                    .format("%A %Y-%m-%d")
                    .to_string()
            })
            .collect(),
        ..PlanContext::default()
    }
}

#[derive(Deserialize)]
struct ActionFile {
    case: Vec<ActionCase>,
}

#[derive(Deserialize)]
struct ActionCase {
    id: String,
    said: String,
    selection: Option<String>,
    action: OneOrMore,
    /// Parameter → what it must contain; times exactly; "-" for absent.
    #[serde(default)]
    params: std::collections::BTreeMap<String, String>,
}

const TIME_PARAMS: &[&str] = &["start", "end", "due"];

fn param_failures(case: &ActionCase, action: &Action) -> Vec<String> {
    let fields = action.fields();
    let mut out = Vec::new();
    for (name, want) in &case.params {
        let got = fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str());
        let ok = match (want.as_str(), got) {
            ("-", got) => got.is_none(),
            (_, None) => false,
            (want, Some(got)) if TIME_PARAMS.contains(&name.as_str()) => want == got,
            (want, Some(got)) => {
                splashdown_core::action::fold(got).contains(&splashdown_core::action::fold(want))
            }
        };
        if !ok {
            out.push(format!("{name} = {got:?}, wanted {want:?}"));
        }
    }
    out
}

async fn actions(cases: &[&ActionCase], verbose: bool, providers: &ProviderOpts) -> Result<()> {
    let (name, pipeline) = text_pipeline(providers)?;
    let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 2)
        .and_then(|d| d.and_hms_opt(10, 0, 0))
        .context("the eval's date")?;
    let base = PlanContext {
        shortcuts: ["Morning routine", "Log water", "Start focus", "Wipe Disk"]
            .map(String::from)
            .to_vec(),
        presets: ["Email", "Code", "Formal"].map(String::from).to_vec(),
        mine: [
            ("Issue", "opens an issue in Linear", true),
            ("Deploy staging", "deploys the website to staging", false),
            ("Next slide", "", false),
            ("Format Drive", "", false),
        ]
        .map(|(name, about, takes_text)| Mine {
            name: name.into(),
            about: about.into(),
            takes_text,
        })
        .to_vec(),
        ..plan_context(now)
    };
    eprintln!("\nActions eval · {name} · {} cases\n", cases.len());
    if let Some(first) = cases.first() {
        let _ = pipeline
            .command(&first.said, None, &PromptContext::default(), true)
            .await;
    }
    let mut latencies = Vec::new();
    let (mut interpret_times, mut plan_times) = (Vec::new(), Vec::new());
    let (mut passed, mut errors, mut false_actions, mut interpreted, mut refused) = (0, 0, 0, 0, 0);
    let mut cost = splashdown_core::Usage::default();
    for case in cases {
        let expect = case.action.list();
        let nothing_ok = expect.contains(&"none");
        let started = Instant::now();
        let mut fails = Vec::new();
        let mut shown = String::new();
        match pipeline
            .command(&case.said, None, &PromptContext::default(), true)
            .await
        {
            Err(e) => {
                errors += 1;
                fails.push(format!("error: {e}"));
            }
            Ok(out) => {
                cost += out.usage;
                interpret_times.push(out.llm_time);
                match out.command {
                    Command::Do(request) => {
                        interpreted += 1;
                        let ctx = PlanContext {
                            selection: case.selection.clone(),
                            ..base.clone()
                        };
                        match pipeline.plan_action(&request, &ctx, None).await {
                            Err(e) => {
                                errors += 1;
                                fails.push(format!("error: {e}"));
                            }
                            Ok(plan) => {
                                cost += plan.usage;
                                plan_times.push(plan.llm_time);
                                match plan.plan {
                                    Plan::Nothing(reason) => {
                                        shown = format!("none: {reason}");
                                        if !nothing_ok {
                                            fails.push(format!(
                                                "none, wanted {}",
                                                expect.join(" or ")
                                            ));
                                        }
                                    }
                                    Plan::Invalid(why) => {
                                        refused += 1;
                                        shown = format!("invalid: {why}");
                                        if !nothing_ok {
                                            fails.push(format!("invalid: {why}"));
                                        }
                                    }
                                    Plan::Do(action) => {
                                        shown = format!("{action:?}");
                                        if expect.contains(&action.id()) {
                                            fails.extend(param_failures(case, &action));
                                        } else {
                                            false_actions += 1;
                                            fails.push(format!(
                                                "{}, wanted {} · FALSE ACTION",
                                                action.id(),
                                                expect.join(" or ")
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    other => {
                        shown = format!("interpreted as {}", command_id(&other));
                        if !nothing_ok {
                            fails.push(format!("{}, not do", command_id(&other)));
                        }
                    }
                }
            }
        }
        latencies.push(started.elapsed());
        let ok = fails.is_empty();
        passed += usize::from(ok);
        println!(
            "{} {:<24} {:>5.2}s  {}",
            if ok { "✓" } else { "✗" },
            case.id,
            started.elapsed().as_secs_f32(),
            fails.join("; ")
        );
        if (verbose || !ok) && !shown.is_empty() {
            println!("      │ {shown}");
        }
    }
    summary(&name, passed, cases.len(), errors, &mut latencies);
    interpret_times.sort();
    plan_times.sort();
    println!(
        "{name}: {false_actions} false action(s) · {refused} refused by the checks · {interpreted} of {} interpreted as do · interpret p50 {:.2}s · plan p50 {:.2}s · cost ${:.4}",
        cases.len(),
        percentile(&interpret_times, 0.5),
        percentile(&plan_times, 0.5),
        cost.cost_usd
    );
    Ok(())
}

// ---------------------------------------------------------------- stt

#[derive(Deserialize)]
struct SttFile {
    case: Vec<SttCase>,
}

#[derive(Deserialize)]
struct SttCase {
    id: String,
    text: String,
    /// The language spoken (ISO 639-1). Unset means English.
    language: Option<String>,
    /// The `say` voice for `make-audio`, for a case in another language.
    voice: Option<String>,
}

async fn stt(
    path: &Path,
    audio_dir: &Path,
    verbose: bool,
    detect_language: bool,
    providers: &ProviderOpts,
) -> Result<()> {
    let file: SttFile = toml::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    let settings = opts::settings(providers)?;
    let stt = opts::build_stt(&settings, &Secrets::from_env())?;
    let name = stt.name();
    eprintln!("Speech-to-text eval · {name} · {} cases\n", file.case.len());

    let mut latencies = Vec::new();
    let mut total_wer = 0.0;
    let mut errors = 0;
    // Error rate per language, for sets with more than one.
    let mut by_language: std::collections::BTreeMap<String, (f64, usize)> = Default::default();
    for case in &file.case {
        let language = case.language.clone().unwrap_or_else(|| "en".into());
        let hints = splashdown_core::SttHints {
            vocabulary: settings.general.vocabulary.clone(),
            language: (!detect_language).then(|| language.clone()),
            ..Default::default()
        };
        let wav = audio_dir.join(format!("{}.wav", case.id));
        let clip = splashdown_audio::wav::read(&wav).with_context(|| {
            format!(
                "reading {} (run `splashdown-cli eval make-audio`)",
                wav.display()
            )
        })?;
        let Some(clip) = splashdown_audio::prepare(&clip) else {
            bail!("no speech found in {}", wav.display());
        };
        let started = Instant::now();
        let result = stt.transcribe(&clip, &hints).await;
        let elapsed = started.elapsed();
        latencies.push(elapsed);
        match result {
            Ok(t) => {
                let rate = wer(&case.text, &t.text);
                total_wer += rate;
                let entry = by_language.entry(language.clone()).or_default();
                entry.0 += rate;
                entry.1 += 1;
                println!(
                    "{:<26} {:>5.2}s  WER {:>5.1}%",
                    case.id,
                    elapsed.as_secs_f32(),
                    rate * 100.0
                );
                if verbose || rate > 0.0 {
                    println!("      │ heard: {}", t.text);
                }
            }
            Err(e) => {
                errors += 1;
                total_wer += 1.0;
                let entry = by_language.entry(language.clone()).or_default();
                entry.0 += 1.0;
                entry.1 += 1;
                println!(
                    "{:<26} {:>5.2}s  error: {e}",
                    case.id,
                    elapsed.as_secs_f32()
                );
            }
        }
    }
    let n = file.case.len().max(1) as f64;
    if by_language.len() > 1 {
        let rates: Vec<String> = by_language
            .iter()
            .map(|(lang, (sum, count))| format!("{lang} {:.0}%", sum / *count as f64 * 100.0))
            .collect();
        println!("\nby language: {}", rates.join(" · "));
    }
    latencies.sort();
    println!(
        "\n{name}: mean WER {:.1}% · {errors} errors · latency p50 {:.2}s p90 {:.2}s",
        total_wer / n * 100.0,
        percentile(&latencies, 0.5),
        percentile(&latencies, 0.9)
    );
    Ok(())
}

fn make_audio(path: &Path, audio_dir: &Path, voice: &str) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("make-audio uses macOS `say`; copy eval/audio from a Mac instead");
    }
    let file: SttFile = toml::from_str(&std::fs::read_to_string(path)?)?;
    std::fs::create_dir_all(audio_dir)?;
    for case in &file.case {
        let out = audio_dir.join(format!("{}.wav", case.id));
        let status = std::process::Command::new("say")
            .args([
                "-v",
                case.voice.as_deref().unwrap_or(voice),
                "--file-format=WAVE",
                "--data-format=LEI16@16000",
                "-o",
            ])
            .arg(&out)
            .arg(&case.text)
            .status()?;
        if !status.success() {
            bail!("`say` failed for {}", case.id);
        }
        println!("{}", out.display());
    }
    Ok(())
}

// ---------------------------------------------------------------- helpers

fn summary(name: &str, passed: usize, total: usize, errors: usize, latencies: &mut [Duration]) {
    latencies.sort();
    println!(
        "\n{name}: {passed}/{total} passed ({:.0}%) · {errors} errors · latency p50 {:.2}s p90 {:.2}s",
        passed as f64 * 100.0 / total.max(1) as f64,
        percentile(latencies, 0.5),
        percentile(latencies, 0.9)
    );
}

/// `p`-th percentile in seconds of already-sorted durations.
fn percentile(sorted: &[Duration], p: f64) -> f32 {
    sorted
        .get(((sorted.len() as f64 - 1.0).max(0.0) * p).round() as usize)
        .map_or(0.0, |d| d.as_secs_f32())
}

/// Lowercased words with punctuation stripped (apostrophes kept).
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || is_unspaced(c))) {
        let mut rest = String::new();
        for c in word.chars() {
            if is_unspaced(c) {
                if !rest.is_empty() {
                    out.push(std::mem::take(&mut rest).to_lowercase());
                }
                out.push(c.to_string());
            } else {
                rest.push(c);
            }
        }
        if !rest.is_empty() {
            out.push(rest.to_lowercase());
        }
    }
    out
}

/// Chinese and Japanese aren't written with spaces between words: each of
/// their characters counts as a word (so the rate is a character error
/// rate there, the usual measure).
fn is_unspaced(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}')
}

/// Word error rate: word-level edit distance / reference length.
fn wer(reference: &str, hypothesis: &str) -> f64 {
    let r = words(reference);
    let h = words(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            let sub = prev[j] + usize::from(rw != hw);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len() as f64
}

struct NoopStt;

#[async_trait::async_trait]
impl splashdown_core::SpeechToText for NoopStt {
    fn name(&self) -> String {
        "none".into()
    }
    async fn transcribe(
        &self,
        _: &splashdown_core::AudioClip,
        _: &splashdown_core::SttHints,
    ) -> splashdown_core::ProviderResult<splashdown_core::Transcript> {
        Err(splashdown_core::ProviderError::Unavailable(
            "text-only".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wer_basics() {
        assert_eq!(wer("hello world", "Hello, world!"), 0.0);
        assert!((wer("a b c d", "a x c d") - 0.25).abs() < 1e-9);
        assert!((wer("a b", "a b c") - 0.5).abs() < 1e-9);
        assert_eq!(wer("", ""), 0.0);
        // One character of five wrong.
        assert!((wer("我们下周发", "我们上周发") - 0.2).abs() < 1e-9);
        assert_eq!(wer("新しいバージョン", "新しい バージョン。"), 0.0);
    }

    #[test]
    fn reject_matches_whole_words() {
        let case = CleanupCase {
            id: "t".into(),
            transcript: String::new(),
            instruction: None,
            selection: None,
            app: None,
            preset: None,
            context: None,
            earlier: Vec::new(),
            transform: None,
            expect: vec!["friday".into()],
            reject: vec!["um".into(), "you know".into()],
            reject_text: Vec::new(),
            min_lines: None,
            max_chars: None,
        };
        assert!(case.failures("Bring an umbrella on Friday.").is_empty());
        assert_eq!(case.failures("Um, on Friday."), vec!["contains \"um\""]);
        assert_eq!(
            case.failures("You know, Monday."),
            vec![
                "missing \"friday\"".to_string(),
                "contains \"you know\"".to_string()
            ]
        );
    }
}
