//! Eval harness: scores the AI pass and speech-to-text so model
//! defaults are chosen from data, not guesses.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::Deserialize;
use splashdown_config::Secrets;
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
    /// Score speech-to-text (word error rate) on eval/stt.toml.
    Stt {
        #[arg(long, default_value = "eval/stt.toml")]
        cases: PathBuf,
        #[arg(long, default_value = "eval/audio")]
        audio_dir: PathBuf,
        #[arg(short, long)]
        verbose: bool,
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
        EvalCommand::Stt {
            cases,
            audio_dir,
            verbose,
            providers,
        } => stt(&cases, &audio_dir, verbose, &providers).await,
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
                (outcome.text, fails)
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

// ---------------------------------------------------------------- stt

#[derive(Deserialize)]
struct SttFile {
    case: Vec<SttCase>,
}

#[derive(Deserialize)]
struct SttCase {
    id: String,
    text: String,
}

async fn stt(path: &Path, audio_dir: &Path, verbose: bool, providers: &ProviderOpts) -> Result<()> {
    let file: SttFile = toml::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    let settings = opts::settings(providers)?;
    let stt = opts::build_stt(&settings, &Secrets::from_env())?;
    let name = stt.name();
    let hints = splashdown_core::SttHints {
        vocabulary: settings.general.vocabulary.clone(),
        language: Some("en".into()),
        ..Default::default()
    };
    eprintln!("Speech-to-text eval · {name} · {} cases\n", file.case.len());

    let mut latencies = Vec::new();
    let mut total_wer = 0.0;
    let mut errors = 0;
    for case in &file.case {
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
                println!(
                    "{:<26} {:>5.2}s  error: {e}",
                    case.id,
                    elapsed.as_secs_f32()
                );
            }
        }
    }
    let n = file.case.len().max(1) as f64;
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
                voice,
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
    text.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
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
