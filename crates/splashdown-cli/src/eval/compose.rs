//! `eval compose`: drafts written from what was said, and changes to them
//! (eval/compose.toml).

use super::*;

#[derive(Deserialize)]
pub(super) struct ComposeFile {
    pub(super) case: Vec<ComposeCase>,
}

#[derive(Deserialize)]
pub(super) struct ComposeCase {
    pub(super) id: String,
    pub(super) asked: String,
    pub(super) replying_to: Option<String>,
    /// Text read from the window around the field (nothing selected).
    pub(super) window: Option<String>,
    pub(super) app: Option<String>,
    pub(super) user: Option<String>,
    pub(super) tone: Option<String>,
    /// One note's text; its first line is the title.
    pub(super) notes: Option<String>,
    /// A revision: the draft so far and what to change.
    pub(super) draft: Option<String>,
    pub(super) change: Option<String>,
    #[serde(default)]
    pub(super) has: Vec<String>,
    #[serde(default)]
    pub(super) has_any: Vec<String>,
    #[serde(default)]
    pub(super) not: Vec<String>,
    pub(super) lang: Option<String>,
    pub(super) max_words: Option<usize>,
    #[serde(default)]
    pub(super) placeholder: bool,
    #[serde(default)]
    pub(super) no_numbers: bool,
}

/// What's wrong with `draft` for `case`, if anything.
pub(super) fn compose_faults(case: &ComposeCase, draft: &str) -> Vec<String> {
    let lower = draft.to_lowercase();
    let mut faults = Vec::new();
    for want in &case.has {
        if !lower.contains(&want.to_lowercase()) {
            faults.push(format!("missing {want:?}"));
        }
    }
    if !case.has_any.is_empty()
        && !case
            .has_any
            .iter()
            .any(|w| lower.contains(&w.to_lowercase()))
    {
        faults.push(format!("none of {:?}", case.has_any));
    }
    for unwanted in &case.not {
        if lower.contains(&unwanted.to_lowercase()) {
            faults.push(format!("has {unwanted:?}"));
        }
    }
    if let Some(lang) = &case.lang {
        match splashdown_core::aloud::language_guess(draft) {
            Some(got) if got == lang => {}
            got => faults.push(format!(
                "language {}, wanted {lang}",
                got.unwrap_or("unclear")
            )),
        }
    }
    let words = draft.split_whitespace().count();
    if let Some(max) = case.max_words
        && words > max
    {
        faults.push(format!("{words} words, at most {max}"));
    }
    if case.placeholder && !(draft.contains('[') && draft.contains(']')) {
        faults.push("no [placeholder] for the missing fact".into());
    }
    if case.no_numbers && draft.chars().any(|c| c.is_ascii_digit()) {
        faults.push("a number nobody gave".into());
    }
    // Every case: dashes between words read as written by an AI.
    if draft.contains('—') || draft.contains(" – ") {
        faults.push("a dash between words".into());
    }
    faults
}

pub(super) async fn compose(
    cases: &[&ComposeCase],
    verbose: bool,
    providers: &ProviderOpts,
) -> Result<()> {
    use splashdown_core::ask::NoteExcerpt;
    use splashdown_core::compose::{self, ComposeContext};
    let (name, pipeline) = text_pipeline(providers)?;
    let model = pipeline
        .answer_model()
        .context("this eval needs an AI provider (--ai …)")?;
    let context = |case: &ComposeCase| ComposeContext {
        replying_to: case.replying_to.clone(),
        window_text: case.window.clone(),
        app_name: case.app.clone(),
        user_name: case.user.clone(),
        today: "Monday, 3 November 2026".into(),
        tone: case.tone.as_deref().and_then(compose::tone),
        match_style: true,
        notes: case
            .notes
            .iter()
            .map(|text| NoteExcerpt {
                title: text.lines().next().unwrap_or_default().to_string(),
                date: "2026-11-03".into(),
                text: text.clone(),
            })
            .collect(),
        ..ComposeContext::default()
    };
    // As the app does: instructions to an AI are left out of the material.
    let request = |case: &ComposeCase| {
        let (ctx, _) = context(case).defanged();
        match (&case.draft, &case.change) {
            (Some(draft), Some(change)) => compose::revise(&case.asked, draft, change, &ctx),
            _ => compose::request(&case.asked, &ctx),
        }
    };
    // Untimed warm-up so a local model's load time doesn't skew latency.
    if let Some(first) = cases.first() {
        let _ = model.complete(&request(first)).await;
    }
    eprintln!("\nCompose eval · {name}\n");
    let mut latencies = Vec::new();
    let (mut passed, mut errors) = (0, 0);
    let mut cost = splashdown_core::Usage::default();
    for case in cases {
        let started = Instant::now();
        let result = model.complete(&request(case)).await;
        latencies.push(started.elapsed());
        let (draft, faults) = match result {
            Ok(done) => {
                cost += done.usage;
                let draft = compose::clean(&done.text);
                let faults = compose_faults(case, &draft);
                (draft, faults)
            }
            Err(e) => {
                errors += 1;
                (String::new(), vec![format!("error: {e}")])
            }
        };
        let ok = faults.is_empty();
        passed += usize::from(ok);
        println!(
            "{} {:<26} {:>5.2}s  {}",
            if ok { "✓" } else { "✗" },
            case.id,
            started.elapsed().as_secs_f32(),
            faults.join("; ")
        );
        if (verbose || !ok) && !draft.is_empty() {
            for line in draft.lines() {
                println!("      │ {line}");
            }
        }
    }
    summary(&name, passed, cases.len(), errors, &mut latencies);
    println!("{name}: cost ${:.4}", cost.cost_usd);
    Ok(())
}
