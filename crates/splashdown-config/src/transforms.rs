//! Transforms: saved instructions run over a note or selected
//! text with no speaking needed, from Notes or from a global shortcut in
//! any app. They go through the same transform prompt as a spoken
//! instruction on a selection.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    pub id: String,
    pub name: String,
    /// What to do with the text, phrased as an instruction to the AI.
    pub instruction: String,
    /// Where the result goes in Notes. Elsewhere it always replaces the
    /// selection.
    pub place: Place,
    /// A global shortcut (e.g. "Ctrl+Alt+Digit1") that runs it on the
    /// selected text in any app.
    pub shortcut: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Place {
    /// Instead of the note (or the selection in it). The text it replaced
    /// is kept as a version.
    #[default]
    Replace,
    /// Under the note (or the selection), after a `---` rule.
    Below,
}

/// What a transform answers when there's nothing to do (To-dos on text
/// without tasks). The text is then left as it was.
pub const NOTHING_TO_DO: &str = "NONE";

/// Whether a transform's answer means there's nothing to do. Models dress
/// it up: "None.", `NONE`, "**NONE**".
pub fn is_nothing_to_do(answer: &str) -> bool {
    answer
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        .eq_ignore_ascii_case(NOTHING_TO_DO)
}

/// The transforms a new install has. Each can be edited or removed.
pub fn builtin_transforms() -> Vec<Transform> {
    let t = |id: &str, name: &str, place: Place, instruction: &str| Transform {
        id: id.into(),
        name: name.into(),
        instruction: instruction.into(),
        place,
        shortcut: None,
    };
    vec![
        t(
            "clean-up",
            "Clean up",
            Place::Replace,
            "Clean this up: remove filler words, false starts and repetition, fix grammar and punctuation, and tighten rambling sentences. Keep the meaning, every fact, name and number, and the language it's in. Don't add anything.",
        ),
        t(
            "summary",
            "Summary",
            Place::Below,
            "Summarize this in a few short bullet points: the main points, and any decisions or dates. Use only what the text says.",
        ),
        t(
            "todos",
            "To-dos",
            Place::Below,
            "List the tasks this text says someone needs to do, one per line as a Markdown checkbox (\"- [ ] \" then the task), in the order they come up, keeping names, dates and amounts exactly. A task is something the text says someone will, must or should do (\"I need to…\", \"can you…\", \"remember to…\"); what already happened, opinions and descriptions are not tasks, and never make one up. If the text has no tasks, output exactly NONE.",
        ),
        t(
            "email",
            "Email",
            Place::Replace,
            "Turn this into a clear, friendly email. Keep every fact. Don't invent details, names or a subject line.",
        ),
        t(
            "prompt",
            "Prompt for an AI",
            Place::Replace,
            "Rewrite this as a clear prompt for an AI assistant or coding agent: the goal first, then the context, requirements and constraints as short bullet points, then what a good answer looks like. Keep every detail from the text and don't add requirements it doesn't state.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_do_however_it_is_written() {
        for answer in ["NONE", " None. ", "`NONE`", "**NONE**", "none!"] {
            assert!(is_nothing_to_do(answer), "{answer}");
        }
        for answer in ["- [ ] Call none of them", "NONE of these", ""] {
            assert!(!is_nothing_to_do(answer), "{answer}");
        }
    }

    #[test]
    fn builtins_have_unique_ids_and_no_shortcuts() {
        let all = builtin_transforms();
        let mut ids: Vec<_> = all.iter().map(|t| t.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        assert!(
            all.iter()
                .all(|t| t.shortcut.is_none() && !t.instruction.is_empty())
        );
    }
}
