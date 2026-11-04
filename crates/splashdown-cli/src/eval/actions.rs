//! `eval actions`: voice actions planned from requests, with their
//! parameters checked (eval/actions.toml).

use super::*;

#[derive(Deserialize)]
pub(super) struct ActionFile {
    pub(super) case: Vec<ActionCase>,
}

#[derive(Deserialize)]
pub(super) struct ActionCase {
    pub(super) id: String,
    pub(super) said: String,
    pub(super) selection: Option<String>,
    pub(super) action: OneOrMore,
    /// Parameter → what it must contain; times exactly; "-" for absent.
    #[serde(default)]
    pub(super) params: std::collections::BTreeMap<String, String>,
}

pub(super) const TIME_PARAMS: &[&str] = &["start", "end", "due"];

pub(super) fn param_failures(case: &ActionCase, action: &Action) -> Vec<String> {
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

pub(super) async fn actions(
    cases: &[&ActionCase],
    verbose: bool,
    providers: &ProviderOpts,
) -> Result<()> {
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
        ..PlanContext::at(now)
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
                        // A case may accept another answer than an action
                        // ("compose" for an email to write).
                        if !nothing_ok && !expect.contains(&command_id(&other)) {
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
