//! Eval harness: scores the AI pass, spoken commands, voice actions,
//! Compose and speech-to-text, so defaults and prompts are chosen from
//! data, not guesses. The cases are in eval/*.toml (each file's header
//! says its fields); results go in eval/README.md.
//!
//! This file has the `eval` subcommands and what they share (reading cases,
//! building a text-only pipeline, the summary line); each eval is in
//! `eval/`, named after its subcommand. To add one, add a file there, its
//! subcommand to `EvalCommand`, and its cases in eval/.

mod actions;
mod cleanup;
mod commands;
mod compose;
mod stt;
mod style;

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

use self::actions::*;
use self::cleanup::*;
use self::commands::*;
use self::compose::*;
use self::stt::*;
use self::style::*;

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
    /// Score Compose's writing style on eval/style.toml: drafts in the
    /// user's own voice (their messages on screen), against drafts without
    /// it, judged by a model, plus rules.
    Style {
        #[arg(long, default_value = "eval/style.toml")]
        cases: PathBuf,
        /// The OpenRouter model that judges which draft sounds more like
        /// the user: a stronger one than the models being scored, and not
        /// one of them.
        #[arg(long, default_value = "google/gemini-2.5-flash")]
        judge: String,
        #[arg(long)]
        filter: Option<String>,
        #[arg(short, long)]
        verbose: bool,
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[command(flatten)]
        providers: ProviderOpts,
    },
    /// Score Compose on eval/compose.toml: drafts written from what was
    /// said, and changes to them.
    Compose {
        #[arg(long, default_value = "eval/compose.toml")]
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
        EvalCommand::Compose {
            cases,
            filter,
            verbose,
            models,
            providers,
        } => {
            let file: ComposeFile = read_cases(&cases)?;
            let cases = filtered(&file.case, filter.as_deref(), |c| &c.id);
            for run in model_runs(&models, &providers) {
                compose(&cases, verbose, &run).await?;
            }
            Ok(())
        }
        EvalCommand::Style {
            cases,
            judge,
            filter,
            verbose,
            models,
            providers,
        } => {
            let file: StyleFile = read_cases(&cases)?;
            let cases = filtered(&file.case, filter.as_deref(), |c| &c.base.id);
            let judge_opts = ProviderOpts {
                ai: Some(splashdown_config::AiProvider::Openrouter),
                ai_model: Some(judge),
                config: providers.config.clone(),
                ..ProviderOpts::default()
            };
            let (judge_name, judge_pipeline) = text_pipeline(&judge_opts)?;
            let judge = judge_pipeline
                .answer_model()
                .context("the judge needs an OpenRouter key")?;
            eprintln!("judge: {judge_name}");
            for run in model_runs(&models, &providers) {
                style(&cases, verbose, &run, judge.as_ref()).await?;
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

// ---------------------------------------------------------------- shared by the evals

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
    let pipeline = Pipeline::new(std::sync::Arc::new(crate::opts::NoStt), Some(llm))
        .with_instruction_model(instruction_llm);
    Ok((name, pipeline))
}

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
            screen: None,
            misheard: Vec::new(),
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
