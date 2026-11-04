//! `eval style`: drafts in the user's own voice (their messages on screen,
//! and the style learned from how they edited earlier drafts), against
//! drafts without it, judged by another model (eval/style.toml).

use super::*;

#[derive(Deserialize)]
pub(super) struct StyleFile {
    pub(super) case: Vec<StyleCase>,
}

/// A Compose case (its checks included) with the user's own messages for
/// the judge, and checks on how it's written.
#[derive(Deserialize)]
pub(super) struct StyleCase {
    #[serde(flatten)]
    pub(super) base: ComposeCase,
    /// The user's own messages in `window`, as the judge's samples.
    #[serde(default)]
    pub(super) voice: Vec<String>,
    pub(super) min_words: Option<usize>,
    /// Starts with a small letter (true) or a capital (false).
    pub(super) starts_lower: Option<bool>,
    /// Has an emoji (true) or none (false).
    pub(super) emoji: Option<bool>,
    /// Earlier drafts and what the user sent instead (`[draft, sent]`):
    /// a style profile is written from them, as the app does, and Compose
    /// follows it.
    #[serde(default)]
    pub(super) edits: Vec<[String; 2]>,
    /// A profile as the user wrote it, when there are no `edits`.
    pub(super) profile: Option<String>,
}

/// The profile `case` drafts with: written from its edits by `model`, as
/// the app does, or given.
async fn profile_for(case: &StyleCase, model: &dyn splashdown_core::TextModel) -> Option<String> {
    use splashdown_core::style;
    if case.edits.is_empty() {
        return case.profile.clone();
    }
    let edits: Vec<style::Edit> = case
        .edits
        .iter()
        .map(|[draft, sent]| style::Edit {
            draft: draft.clone(),
            sent: sent.clone(),
        })
        .collect();
    let kind = splashdown_config::style_kind(&splashdown_config::AppContext {
        app_id: "",
        app_name: case.base.app.as_deref().unwrap_or(""),
        title: None,
        url: None,
    });
    let request = style::profile_request(kind, "", &edits);
    let done = model.complete(&request).await.ok()?;
    Some(style::clean_profile(&done.text)).filter(|p| !p.is_empty())
}

pub(super) fn style_faults(case: &StyleCase, draft: &str) -> Vec<String> {
    let mut faults = compose_faults(&case.base, draft);
    let words = draft.split_whitespace().count();
    if let Some(min) = case.min_words
        && words < min
    {
        faults.push(format!("{words} words, at least {min}"));
    }
    if let (Some(lower), Some(first)) =
        (case.starts_lower, draft.chars().find(|c| c.is_alphabetic()))
        && first.is_lowercase() != lower
    {
        faults.push(
            if lower {
                "starts with a capital"
            } else {
                "starts with a small letter"
            }
            .into(),
        );
    }
    let has_emoji = draft
        .chars()
        .any(|c| matches!(u32::from(c), 0x1F300..=0x1FAFF | 0x2600..=0x27BF));
    match case.emoji {
        Some(true) if !has_emoji => faults.push("no emoji".into()),
        Some(false) if has_emoji => faults.push("an emoji".into()),
        _ => {}
    }
    faults
}

pub(super) const STYLE_JUDGE: &str = "The samples are messages one person wrote. Drafts A and B say roughly the same thing. Which one sounds more like that person wrote it: their kind of greeting and sign-off (or none), length, how formal, capital letters, punctuation, exclamation marks, emoji and usual words? Judge only how it's written, not which says it better. Answer with one letter: A or B.";

/// Which draft the judge says sounds more like `voice`: Some(true) for
/// `a`.
pub(super) async fn judged_first(
    model: &dyn splashdown_core::TextModel,
    voice: &[String],
    a: &str,
    b: &str,
) -> Option<bool> {
    let request = splashdown_core::TextRequest {
        system: STYLE_JUDGE.into(),
        user: format!(
            "<samples>\n{}\n</samples>\n<a>\n{a}\n</a>\n<b>\n{b}\n</b>",
            voice.join("\n---\n")
        ),
        temperature: 0.0,
        max_tokens: 5,
        json_schema: None,
    };
    let answer = model.complete(&request).await.ok()?.text;
    match answer.trim().chars().next()?.to_ascii_uppercase() {
        'A' => Some(true),
        'B' => Some(false),
        _ => None,
    }
}

pub(super) async fn style(
    cases: &[&StyleCase],
    verbose: bool,
    providers: &ProviderOpts,
    judge: &dyn splashdown_core::TextModel,
) -> Result<()> {
    use splashdown_core::compose::{self, ComposeContext};
    let (name, pipeline) = text_pipeline(providers)?;
    let model = pipeline
        .answer_model()
        .context("this eval needs an AI provider (--ai …)")?;
    let request = |case: &StyleCase, match_style: bool, style: Option<String>| {
        let ctx = ComposeContext {
            replying_to: case.base.replying_to.clone(),
            window_text: case.base.window.clone(),
            app_name: case.base.app.clone(),
            user_name: case.base.user.clone(),
            today: "Monday, 3 November 2026".into(),
            tone: case.base.tone.as_deref().and_then(compose::tone),
            match_style,
            style: style.filter(|_| match_style),
            ..ComposeContext::default()
        };
        compose::request(&case.base.asked, &ctx.defanged().0)
    };
    if let Some(first) = cases.first() {
        let _ = model.complete(&request(first, true, None)).await;
    }
    eprintln!("\nStyle eval · {name}\n");
    let mut latencies = Vec::new();
    let (mut passed, mut errors) = (0, 0);
    // The judge's points for the styled draft (1 per order it won) and how
    // many it could have had.
    let (mut points, mut possible) = (0, 0);
    // The rules, passed by the drafts without the voice: the baseline.
    let mut plain_passed = 0;
    for case in cases {
        let profile = profile_for(case, model.as_ref()).await;
        let started = Instant::now();
        let styled = model.complete(&request(case, true, profile.clone())).await;
        latencies.push(started.elapsed());
        let plain = model.complete(&request(case, false, None)).await;
        let (styled, plain) = match (styled, plain) {
            (Ok(s), Ok(p)) => {
                let ctx = |match_style| ComposeContext {
                    window_text: case.base.window.clone(),
                    user_name: case.base.user.clone(),
                    match_style,
                    ..ComposeContext::default()
                };
                (
                    compose::finish(&s.text, &ctx(true)),
                    compose::finish(&p.text, &ctx(false)),
                )
            }
            (Err(e), _) | (_, Err(e)) => {
                errors += 1;
                println!("✗ {:<28} error: {e}", case.base.id);
                continue;
            }
        };
        let faults = style_faults(case, &styled);
        let ok = faults.is_empty();
        passed += usize::from(ok);
        plain_passed += usize::from(style_faults(case, &plain).is_empty());
        let mut verdict = String::new();
        if styled.trim() == plain.trim() {
            verdict = "same".into();
        } else if !case.voice.is_empty() || !case.edits.is_empty() {
            // The user's own writing: on screen, or what they sent.
            let samples: Vec<String> = case
                .voice
                .iter()
                .cloned()
                .chain(case.edits.iter().map(|[_, sent]| sent.clone()))
                .collect();
            let first = judged_first(judge, &samples, &styled, &plain).await;
            let second = judged_first(judge, &samples, &plain, &styled).await;
            let won = usize::from(first == Some(true)) + usize::from(second == Some(false));
            points += won;
            possible += 2;
            verdict = format!("style {won}/2");
        }
        println!(
            "{} {:<28} {:>5.2}s  {:<9} {}",
            if ok { "✓" } else { "✗" },
            case.base.id,
            started.elapsed().as_secs_f32(),
            verdict,
            faults.join("; ")
        );
        if verbose && let Some(profile) = &profile {
            for line in profile.lines() {
                println!("      ✎ {line}");
            }
        }
        if verbose || !ok {
            for line in styled.lines() {
                println!("      │ {line}");
            }
            if verbose {
                println!("      without:");
                for line in plain.lines() {
                    println!("      ┆ {line}");
                }
            }
        }
    }
    summary(&name, passed, cases.len(), errors, &mut latencies);
    println!(
        "{name}: without the voice, {plain_passed}/{} pass the rules",
        cases.len()
    );
    if possible > 0 {
        println!(
            "{name}: where the drafts differ, the judge preferred the styled one {points}/{possible} ({:.0}%)",
            100.0 * points as f64 / possible as f64
        );
    }
    Ok(())
}
