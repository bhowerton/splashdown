//! Find-and-replace rules applied to dictated text.

use super::*;

/// A find-and-replace rule. Plain rules match whole words, ignoring case;
/// regex rules are used as written (`$1` refers to groups).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Replacement {
    pub find: String,
    pub replace: String,
    pub regex: bool,
}

impl Replacement {
    /// The rule as a pattern: a plain rule matches its text ignoring case,
    /// as whole words where it starts or ends with a letter or digit; a
    /// regex rule is used as written.
    pub fn compile(&self) -> Result<regex::Regex, regex::Error> {
        if self.find.is_empty() {
            return Err(regex::Error::Syntax("nothing to find".into()));
        }
        if self.regex {
            return regex::Regex::new(&self.find);
        }
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        let pattern = format!(
            "{}{}{}",
            if word(self.find.chars().next()) {
                r"\b"
            } else {
                ""
            },
            regex::escape(&self.find),
            if word(self.find.chars().last()) {
                r"\b"
            } else {
                ""
            },
        );
        regex::RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .build()
    }
}

/// Why one of these replacement rules can't be used: `None` if they're
/// all fine. Settings and the CLI refuse to save them otherwise.
pub fn replacement_problem(rules: &[Replacement]) -> Option<String> {
    rules.iter().enumerate().find_map(|(i, r)| {
        r.compile().err().map(|e| {
            format!(
                "Replacement {} (“{}”) isn't a valid pattern: {e}",
                i + 1,
                r.find
            )
        })
    })
}
