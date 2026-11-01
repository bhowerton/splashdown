//! Smarter snippets: fill-ins like `{date}` in snippet text, and
//! find-and-replace rules applied to the final text.

use chrono::{DateTime, Local};
use regex::Regex;
use splashdown_config::Replacement;
use splashdown_core::spoken::Snippet;

/// Replacement rules, compiled once per engine.
#[derive(Default, Clone)]
pub struct Replacements(Vec<(Regex, String)>);

impl Replacements {
    /// Compiles the rules. Invalid ones are skipped (Settings refuses to
    /// save them; see `splashdown_config::replacement_problem`).
    pub fn new(rules: &[Replacement]) -> Self {
        Self(
            rules
                .iter()
                .filter_map(|r| match r.compile() {
                    Ok(re) => Some((re, r.replace.clone())),
                    Err(e) => {
                        tracing::warn!("skipping replacement {:?}: {e}", r.find);
                        None
                    }
                })
                .collect(),
        )
    }

    /// Applies every rule in order.
    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (re, replace) in &self.0 {
            out = re.replace_all(&out, replace.as_str()).into_owned();
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Snippets with their fill-ins replaced for a dictation happening `now`:
/// `{date}`, `{iso-date}`, `{day}`, `{time}` and `{clipboard}`.
pub fn fill_in(
    snippets: &[Snippet],
    now: DateTime<Local>,
    clipboard: Option<&str>,
) -> Vec<Snippet> {
    snippets
        .iter()
        .map(|s| {
            if !s.text.contains('{') {
                return s.clone();
            }
            let mut text = s
                .text
                .replace("{date}", &now.format("%B %-d, %Y").to_string())
                .replace("{iso-date}", &now.format("%Y-%m-%d").to_string())
                .replace("{day}", &now.format("%A").to_string())
                .replace("{time}", &now.format("%-I:%M %p").to_string());
            if text.contains("{clipboard}") {
                let paste = clipboard
                    .map(str::to_string)
                    .or_else(splashdown_platform::clipboard_text)
                    .unwrap_or_default();
                text = text.replace("{clipboard}", &paste);
            }
            Snippet {
                trigger: s.trigger.clone(),
                text,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn rule(find: &str, replace: &str, regex: bool) -> Replacement {
        Replacement {
            find: find.into(),
            replace: replace.into(),
            regex,
        }
    }

    #[test]
    fn plain_rules_match_whole_words_ignoring_case() {
        let r = Replacements::new(&[
            rule("teh", "the", false),
            rule("e.g.", "for example", false),
        ]);
        assert_eq!(r.apply("Teh cat ate teh fish"), "the cat ate the fish");
        assert_eq!(r.apply("tehran"), "tehran", "only whole words");
        assert_eq!(r.apply("fruit, e.g. apples"), "fruit, for example apples");
    }

    #[test]
    fn regex_rules_are_used_as_written() {
        let r = Replacements::new(&[rule(r"(\d+) percent", "$1%", true)]);
        assert_eq!(r.apply("up 15 percent"), "up 15%");
        assert!(
            splashdown_config::replacement_problem(&[rule("(", "x", true)])
                .unwrap()
                .contains("Replacement 1")
        );
        assert!(
            splashdown_config::replacement_problem(&[rule("(", "x", false)]).is_none(),
            "plain text is escaped"
        );
        // An invalid rule is skipped, not fatal.
        assert!(Replacements::new(&[rule("(", "x", true)]).is_empty());
    }

    #[test]
    fn snippets_get_their_fill_ins() {
        let now = Local.with_ymd_and_hms(2026, 10, 30, 15, 4, 0).unwrap();
        let snippets = [Snippet {
            trigger: "sign off".into(),
            text: "Thanks — {day}, {date} ({iso-date}) at {time}. {clipboard}".into(),
        }];
        let filled = fill_in(&snippets, now, Some("Link"));
        assert_eq!(
            filled[0].text,
            "Thanks — Friday, October 30, 2026 (2026-10-30) at 3:04 PM. Link"
        );
    }
}
