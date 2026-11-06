//! `eval cleanup`: the AI pass on dictated text (eval/cleanup.toml).

use super::*;

#[derive(Deserialize)]
pub(super) struct CleanupFile {
    pub(super) case: Vec<CleanupCase>,
}

#[derive(Deserialize)]
pub(super) struct CleanupCase {
    pub(super) id: String,
    /// Dictated content (unused for transforms).
    #[serde(default)]
    pub(super) transcript: String,
    pub(super) instruction: Option<String>,
    pub(super) selection: Option<String>,
    pub(super) app: Option<String>,
    pub(super) preset: Option<String>,
    /// Text already in the field around the cursor (field context).
    pub(super) context: Option<String>,
    /// Earlier dictations into the same app, oldest first.
    #[serde(default)]
    pub(super) earlier: Vec<String>,
    /// Text read from the window around the field: its names and terms
    /// join the vocabulary, as in the app.
    pub(super) screen: Option<String>,
    /// Vocabulary words speech recognition wrote as something else before
    /// (`[word, heard]`), as learned from corrections.
    #[serde(default)]
    pub(super) misheard: Vec<[String; 2]>,
    /// A built-in transform's id: its instruction is used on `selection`.
    pub(super) transform: Option<String>,
    /// A language to translate into, with the app's translate instruction
    /// (⌃⌥T, a preset's "Translate into"): on `selection` if there is one,
    /// otherwise on the dictation.
    pub(super) translate: Option<String>,
    /// Case-insensitive substrings the output must contain.
    #[serde(default)]
    pub(super) expect: Vec<String>,
    /// Case-insensitive words/phrases the output must not contain.
    #[serde(default)]
    pub(super) reject: Vec<String>,
    /// Case-insensitive text it must not contain, punctuation included
    /// ("- [ ]", "(Dana)").
    #[serde(default)]
    pub(super) reject_text: Vec<String>,
    pub(super) min_lines: Option<usize>,
    pub(super) max_chars: Option<usize>,
}

impl CleanupCase {
    pub(super) fn job(&self) -> TextJob {
        let builtin = self.transform.as_deref().map(|id| {
            splashdown_config::builtin_transforms()
                .into_iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("no built-in transform {id:?}"))
                .instruction
        });
        let translate = self
            .translate
            .as_deref()
            .map(splashdown_config::translate_instruction);
        let instruction = builtin.or(translate).or_else(|| self.instruction.clone());
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
                vocabulary: ["Tauri", "OpenRouter", "Kubernetes"]
                    .into_iter()
                    .map(String::from)
                    .chain(
                        self.screen
                            .iter()
                            .flat_map(|s| splashdown_core::vocab::terms_on_screen(s, 30)),
                    )
                    .chain(self.misheard.iter().map(|[word, _]| word.clone()))
                    .collect(),
                misheard: self
                    .misheard
                    .iter()
                    .map(|[word, heard]| (word.clone(), heard.clone()))
                    .collect(),
                ..PromptContext::default()
            },
        }
    }

    pub(super) fn failures(&self, text: &str) -> Vec<String> {
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

pub(super) async fn cleanup(
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

pub(super) struct Row {
    pub(super) name: String,
    pub(super) passed: usize,
    pub(super) total: usize,
    pub(super) errors: usize,
    /// Failures the sanity guard caught, so the user got their raw words.
    pub(super) guarded: usize,
    pub(super) p50: f32,
    pub(super) p90: f32,
}

pub(super) async fn cleanup_one(
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
    let pipeline = Pipeline::new(std::sync::Arc::new(crate::opts::NoStt), Some(llm))
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
