//! `eval instruct`: instructions run on whole texts, as a hook that asks
//! the AI does (eval/instruct.toml): the ready-made hooks' own wording on
//! made-up meetings, weeks of notes and briefs with known answers.
//!
//! A result is checked by rule, not by another model: the lines it must
//! have, each under its heading; the lines it mustn't (someone else's
//! task among the user's own); words that mustn't be there; dates the
//! text doesn't have.

use std::collections::BTreeMap;

use super::*;

#[derive(Deserialize)]
pub(super) struct InstructFile {
    /// The ready-made hooks' instructions, by a short name.
    pub(super) instructions: BTreeMap<String, String>,
    pub(super) case: Vec<InstructCase>,
}

#[derive(Deserialize)]
pub(super) struct InstructCase {
    pub(super) id: String,
    /// What the text is, as a hook's event: `meeting`, `schedule`…
    pub(super) kind: String,
    /// The text: a file beside the cases, in `instruct/`, or itself.
    pub(super) file: Option<String>,
    pub(super) text: Option<String>,
    /// Which of the file's `instructions` to run, or one in own words.
    pub(super) ask: Option<String>,
    pub(super) instruction: Option<String>,
    /// For a schedule: the days its notes are from.
    pub(super) days: Option<String>,
    #[serde(default)]
    pub(super) headings: Vec<String>,
    #[serde(default)]
    pub(super) want: Vec<Wanted>,
    #[serde(default)]
    pub(super) not: Vec<Wanted>,
    #[serde(default)]
    pub(super) never: Vec<String>,
    #[serde(default)]
    pub(super) no_dates: bool,
    #[serde(default)]
    pub(super) nothing: bool,
    pub(super) max_lines: Option<usize>,
}

/// A line of the result: one that has all these words, in any capitals,
/// and is under this heading when one is given.
#[derive(Deserialize)]
pub(super) struct Wanted {
    pub(super) has: Vec<String>,
    pub(super) under: Option<String>,
}

/// How a result did: how many of the wanted lines it has, and what's wrong
/// with it.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Checked {
    pub(super) found: usize,
    pub(super) faults: Vec<String>,
}

/// A heading's words, if `line` is one: a Markdown heading, a line that's
/// all bold, or one of `known` on a line of its own (with a colon or not).
fn heading(line: &str, known: &[String]) -> Option<String> {
    let line = line.trim();
    let marked = line.starts_with('#');
    let text = line.trim_start_matches('#').trim();
    let bold = text.len() > 4 && text.starts_with("**") && {
        let end = text.trim_end_matches(':').trim_end();
        end.ends_with("**") && !end[2..end.len() - 2].contains("**")
    };
    let text = text
        .trim_matches(|c: char| c == '*' || c == ':' || c.is_whitespace())
        .to_lowercase();
    let named = known.iter().any(|k| k.to_lowercase() == text);
    (!text.is_empty() && (marked || bold || named)).then_some(text)
}

/// The result's lines that say something, in small letters, each with the
/// heading it's under, and the headings themselves.
fn lines(result: &str, known: &[String]) -> (Vec<(Option<String>, String)>, Vec<String>) {
    let mut under = None;
    let mut out = Vec::new();
    let mut headings = Vec::new();
    for line in result.lines().filter(|l| !l.trim().is_empty()) {
        match heading(line, known) {
            Some(text) => {
                headings.push(text.clone());
                under = Some(text);
            }
            None => out.push((under.clone(), line.to_lowercase())),
        }
    }
    (out, headings)
}

/// The dates written in `text`, each in one form however it was written:
/// `2026-11-12`, and a day with its month ("12 November", "Nov 12th") as
/// `nov 12`. A weekday or "the 20th" is a day as people say it, not a
/// date.
fn dates(text: &str) -> Vec<String> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '/'))
        .filter(|w| !w.is_empty())
        .collect();
    let month = |word: &str| {
        MONTHS
            .iter()
            .find(|m| **m == word || (word.len() >= 3 && word != "may" && m.starts_with(word)))
            .map(|m| &m[..3])
    };
    let day = |word: &str| {
        let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let ending = &word[digits.len()..];
        let number: u32 = digits.parse().ok()?;
        (matches!(ending, "" | "st" | "nd" | "rd" | "th") && (1..=31).contains(&number))
            .then_some(number)
    };
    let mut out = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let parts: Vec<&str> = word.split('-').collect();
        let iso = parts.len() == 3
            && parts[0].len() == 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        let slashed = word.split('/').count() >= 2
            && word
                .split('/')
                .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()));
        if iso || slashed {
            out.push(word.to_string());
        } else if let Some(month) = month(word) {
            let before = i.checked_sub(1).and_then(|j| day(words[j]));
            let after = words.get(i + 1).and_then(|w| day(w));
            if let Some(day) = before.or(after) {
                out.push(format!("{month} {day}"));
            }
        }
    }
    out
}

/// Checks `result` for `case`, written from `source`.
pub(super) fn check(case: &InstructCase, source: &str, result: &str) -> Checked {
    let mut checked = Checked::default();
    if case.nothing {
        if !result.trim().is_empty() {
            checked
                .faults
                .push("wrote something where there was nothing to do".into());
        }
        return checked;
    }
    if result.trim().is_empty() {
        checked.faults.push("wrote nothing".into());
        return checked;
    }
    let (lines, headings) = lines(result, &case.headings);
    for heading in &case.headings {
        if !headings.iter().any(|h| h.contains(&heading.to_lowercase())) {
            checked.faults.push(format!("no heading {heading:?}"));
        }
    }
    let has = |wanted: &Wanted| {
        lines.iter().any(|(under, line)| {
            wanted
                .has
                .iter()
                .all(|word| line.contains(&word.to_lowercase()))
                && wanted.under.as_ref().is_none_or(|u| {
                    under
                        .as_ref()
                        .is_some_and(|h| h.contains(&u.to_lowercase()))
                })
        })
    };
    let said = |wanted: &Wanted| match &wanted.under {
        Some(under) => format!("{:?} under {under:?}", wanted.has),
        None => format!("{:?}", wanted.has),
    };
    for wanted in &case.want {
        if has(wanted) {
            checked.found += 1;
        } else {
            checked.faults.push(format!("missing {}", said(wanted)));
        }
    }
    for unwanted in &case.not {
        if has(unwanted) {
            checked.faults.push(format!("has {}", said(unwanted)));
        }
    }
    let lower = result.to_lowercase();
    for word in &case.never {
        if lower.contains(&word.to_lowercase()) {
            checked.faults.push(format!("says {word:?}"));
        }
    }
    if case.no_dates {
        let given = dates(source);
        let mut made_up: Vec<String> = dates(result)
            .into_iter()
            .filter(|d| !given.contains(d))
            .collect();
        made_up.dedup();
        if !made_up.is_empty() {
            checked.faults.push(format!(
                "dates the text doesn't have: {}",
                made_up.join(", ")
            ));
        }
    }
    if let Some(most) = case.max_lines
        && lines.len() > most
    {
        checked
            .faults
            .push(format!("{} lines, at most {most}", lines.len()));
    }
    checked
}

/// The instruction a case runs: one of the file's, or its own.
pub(super) fn instruction<'a>(file: &'a InstructFile, case: &'a InstructCase) -> Result<&'a str> {
    match (&case.ask, &case.instruction) {
        (Some(ask), None) => file
            .instructions
            .get(ask)
            .map(String::as_str)
            .with_context(|| format!("{}: no instruction called {ask:?}", case.id)),
        (None, Some(own)) => Ok(own),
        _ => bail!("{}: give `ask` or `instruction`, one of them", case.id),
    }
}

/// The text a case runs on: its own, or a file in `dir`.
fn text(case: &InstructCase, dir: &Path) -> Result<String> {
    match (&case.text, &case.file) {
        (Some(text), None) => Ok(text.clone()),
        (None, Some(file)) => {
            let path = dir.join(file);
            std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
        }
        _ => bail!("{}: give `text` or `file`, one of them", case.id),
    }
}

pub(super) async fn instruct(
    file: &InstructFile,
    cases: &[&InstructCase],
    dir: &Path,
    verbose: bool,
    providers: &ProviderOpts,
) -> Result<()> {
    use splashdown_core::instruct::{self, About};
    let (name, pipeline) = text_pipeline(providers)?;
    let model = pipeline
        .answer_model()
        .context("this eval needs an AI provider (--ai …)")?;
    let about = |case: &InstructCase, text: &str| About {
        what: instruct::what(&case.kind, case.days.as_deref()),
        // As the app gives it: a meeting's or transcript's own title.
        title: matches!(case.kind.as_str(), "meeting" | "transcript")
            .then(|| text.lines().find_map(|l| l.strip_prefix("# ")))
            .flatten()
            .map(|t| t.trim().to_string()),
        user_name: None,
        today: "Friday, 13 November 2026".into(),
    };
    // Untimed warm-up so a local model's load time doesn't skew latency.
    let _ = instruct::run(
        model.as_ref(),
        "Say OK.",
        "Hello.",
        &About {
            what: instruct::what("dictation", None),
            title: None,
            user_name: None,
            today: "Friday, 13 November 2026".into(),
        },
    )
    .await;
    eprintln!("\nInstruct eval · {name}\n");
    let mut latencies = Vec::new();
    let (mut passed, mut errors) = (0, 0);
    let (mut found, mut wanted) = (0, 0);
    let mut cost = splashdown_core::Usage::default();
    for case in cases {
        let source = text(case, dir)?;
        let instruction = instruction(file, case)?;
        let started = Instant::now();
        let result =
            instruct::run(model.as_ref(), instruction, &source, &about(case, &source)).await;
        latencies.push(started.elapsed());
        wanted += case.want.len();
        let (written, parts, checked) = match result {
            Ok(done) => {
                cost += done.usage;
                let checked = check(case, &source, &done.text);
                (done.text, done.parts, checked)
            }
            Err(failed) => {
                errors += 1;
                cost += failed.usage;
                let faults = vec![format!("error: {}", failed.error)];
                (String::new(), 0, Checked { found: 0, faults })
            }
        };
        found += checked.found;
        let ok = checked.faults.is_empty();
        passed += usize::from(ok);
        let lines = if case.want.is_empty() {
            String::new()
        } else {
            format!("{}/{} lines", checked.found, case.want.len())
        };
        println!(
            "{} {:<26} {:>6.2}s  {:<11} {}{}",
            if ok { "✓" } else { "✗" },
            case.id,
            started.elapsed().as_secs_f32(),
            lines,
            if parts > 1 {
                format!("({parts} parts) ")
            } else {
                String::new()
            },
            checked.faults.join("; ")
        );
        if (verbose || !ok) && !written.is_empty() {
            for line in written.lines() {
                println!("      │ {line}");
            }
        }
    }
    summary(&name, passed, cases.len(), errors, &mut latencies);
    println!(
        "{name}: {found} of {wanted} wanted lines found ({:.0}%) · cost ${:.4}",
        found as f64 * 100.0 / wanted.max(1) as f64,
        cost.cost_usd
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(toml: &str) -> InstructCase {
        toml::from_str(&format!(
            "id = \"t\"\nkind = \"schedule\"\ntext = \"x\"\nask = \"a\"\n{toml}"
        ))
        .unwrap()
    }

    const REVIEW: &str = "**Decided**\n- The pricing page goes live on Friday (Pricing review)\n\n## For me to do\n- [ ] Send the numbers to Dana by Wednesday (Pricing review)\n- [ ] Book flights for the offsite\n\nWaiting on others:\n- Sam books the interviews for next week (Hiring sync)\n";

    #[test]
    fn wanted_lines_are_found_under_their_headings() {
        let weekly = case(
            r#"headings = ["Decided", "For me to do", "Waiting on others"]
want = [
  { has = ["numbers", "dana"], under = "For me to do" },
  { has = ["pricing page", "Friday"], under = "Decided" },
  { has = ["Sam", "interviews"], under = "Waiting on others" },
  { has = ["dentist"], under = "For me to do" },
]
not = [{ has = ["interviews"], under = "For me to do" }, { has = ["export fix"] }]
never = ["the user"]
no_dates = true
max_lines = 4"#,
        );
        let checked = check(&weekly, "by Wednesday", REVIEW);
        assert_eq!(checked.found, 3);
        assert_eq!(
            checked.faults,
            [r#"missing ["dentist"] under "For me to do""#]
        );

        // Someone else's task among the user's, the wrong heading, a word
        // that mustn't be there, a made-up date, and too much.
        let wrong = "## For me to do\n- [ ] Sam books the interviews\n- [ ] The user sends the numbers to Dana by 11 November\n- [ ] One more\n- [ ] And another\n- [ ] And a fifth\n";
        let checked = check(&weekly, "by Wednesday", wrong);
        assert_eq!(checked.found, 1);
        let faults = checked.faults.join("\n");
        for fault in [
            "no heading \"Decided\"",
            "no heading \"Waiting on others\"",
            "missing [\"Sam\", \"interviews\"] under \"Waiting on others\"",
            "has [\"interviews\"] under \"For me to do\"",
            "says \"the user\"",
            "dates the text doesn't have: nov 11",
            "5 lines, at most 4",
        ] {
            assert!(faults.contains(fault), "{fault}\n{faults}");
        }
    }

    #[test]
    fn nothing_to_do_wants_nothing_written() {
        let nothing = case("nothing = true");
        assert_eq!(check(&nothing, "x", "  \n"), Checked::default());
        assert_eq!(
            check(&nothing, "x", "- [ ] Enjoy the weather").faults.len(),
            1
        );
        // And something to do wants something.
        assert_eq!(check(&case(""), "x", "").faults, ["wrote nothing"]);
    }

    #[test]
    fn headings_and_dates_are_read_however_they_re_written() {
        let known = vec!["For me to do".to_string()];
        assert_eq!(heading("## Decided", &known).as_deref(), Some("decided"));
        assert_eq!(heading("**Decided**:", &known).as_deref(), Some("decided"));
        assert_eq!(heading("**Decided:**", &known).as_deref(), Some("decided"));
        assert_eq!(
            heading("For me to do:", &known).as_deref(),
            Some("for me to do")
        );
        assert_eq!(heading("Decided", &known), None);
        assert_eq!(heading("- **Dana** owes the **copy**", &known), None);
        assert_eq!(heading("**Dana** owes the **copy**", &known), None);

        assert_eq!(
            dates("By 2026-11-12, or 12 November, or Nov 13th; 11/14 too."),
            ["2026-11-12", "nov 12", "nov 13", "11/14"]
        );
        // Days as people say them, and words that only look like months.
        assert!(dates("by Wednesday, on the 20th, next Tuesday").is_empty());
        assert!(dates("It may be 5 people, and we march on. Mar 5 is a date.").len() == 1);
        // The same date written another way isn't made up.
        let dated = case("no_dates = true");
        assert!(
            check(&dated, "on 12 November", "- by Nov 12")
                .faults
                .is_empty()
        );
        assert_eq!(check(&dated, "by Wednesday", "- by Nov 12").faults.len(), 1);
    }

    /// The ready-made hooks' wording here is the wording in Settings: a
    /// change to one that isn't made to the other would score words nobody
    /// gets.
    #[test]
    fn the_instructions_are_the_ready_made_hooks_own() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let file: InstructFile = read_cases(&root.join("eval/instruct.toml")).unwrap();
        let settings = std::fs::read_to_string(root.join("app/ui/settings/actions.js")).unwrap();
        for (name, instruction) in &file.instructions {
            // As a string is written in the script.
            let written = serde_json::to_string(instruction).unwrap();
            assert!(
                settings.contains(&written),
                "{name} isn't in app/ui/settings/actions.js as eval/instruct.toml has it"
            );
        }
        // Every case can be run: its instruction and its text are there.
        let dir = root.join("eval/instruct");
        for case in &file.case {
            assert!(!instruction(&file, case).unwrap().is_empty(), "{}", case.id);
            assert!(!text(case, &dir).unwrap().trim().is_empty(), "{}", case.id);
            for wanted in case.want.iter().chain(&case.not) {
                assert!(!wanted.has.is_empty(), "{}", case.id);
            }
        }
    }
}
