//! `eval commands`: spoken commands read by the interpreter, with voice
//! actions on and off (eval/commands.toml).

use super::*;

/// One acceptable answer, or several.
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum OneOrMore {
    One(String),
    More(Vec<String>),
}

impl OneOrMore {
    pub(super) fn list(&self) -> Vec<&str> {
        match self {
            OneOrMore::One(s) => vec![s.as_str()],
            OneOrMore::More(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct CommandFile {
    pub(super) case: Vec<CommandCase>,
}

#[derive(Deserialize)]
pub(super) struct CommandCase {
    pub(super) id: String,
    pub(super) said: String,
    pub(super) last: Option<String>,
    pub(super) app: Option<String>,
    pub(super) expect: OneOrMore,
    /// Case-insensitive substrings the command's text must contain.
    #[serde(default)]
    pub(super) text: Vec<String>,
}

pub(super) fn command_id(command: &Command) -> &'static str {
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
        Command::Compose(_) => "compose",
        Command::Ask(_) => "ask",
        Command::Read(_) => "read",
        Command::Do(_) => "do",
        Command::Unclear => "unclear",
    }
}

pub(super) fn command_text(command: &Command) -> &str {
    match command {
        Command::ReplaceLast(t)
        | Command::Write(t)
        | Command::Compose(t)
        | Command::Ask(t)
        | Command::Read(t)
        | Command::Do(t) => t,
        _ => "",
    }
}

pub(super) async fn commands(
    cases: &[&CommandCase],
    verbose: bool,
    providers: &ProviderOpts,
) -> Result<()> {
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
