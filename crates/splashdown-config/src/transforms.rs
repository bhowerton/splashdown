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

/// A transform's answer, tidied where models are known to slip. For the
/// meeting summary: headings with nothing (or only "None") under them are
/// dropped, and action items are always checkboxes.
pub fn tidy_answer(transform_id: &str, answer: &str) -> String {
    if transform_id != "meeting-summary" {
        return answer.to_string();
    }
    let mut out: Vec<String> = Vec::new();
    let mut section: Vec<String> = Vec::new();
    let flush = |section: &mut Vec<String>, out: &mut Vec<String>| {
        let has_content = section.iter().skip(1).any(|line| !is_empty_item(line));
        if has_content || !section.first().is_some_and(|h| h.starts_with('#')) {
            out.append(section);
        }
        section.clear();
    };
    for line in answer.lines() {
        if line.trim_start().starts_with('#') {
            flush(&mut section, &mut out);
        }
        let in_actions = section
            .first()
            .is_some_and(|h| h.to_lowercase().contains("action item"));
        let trimmed = line.trim_start();
        let line = match trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            Some(rest) if in_actions && !rest.starts_with('[') => format!("- [ ] {rest}"),
            _ => line.to_string(),
        };
        section.push(line);
    }
    flush(&mut section, &mut out);
    let text = out.join("\n");
    // No blank runs left where a section was.
    let mut tidy = String::with_capacity(text.len());
    for line in text.lines() {
        if line.trim().is_empty() && tidy.ends_with("\n\n") {
            continue;
        }
        tidy.push_str(line.trim_end());
        tidy.push('\n');
    }
    tidy.trim().to_string()
}

/// A line that says nothing: blank, a bare bullet or checkbox, or "None".
fn is_empty_item(line: &str) -> bool {
    let text = line
        .trim()
        .trim_start_matches(['-', '*', '•'])
        .trim()
        .trim_start_matches("[ ]")
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        .to_lowercase();
    matches!(
        text.as_str(),
        "" | "none"
            | "n/a"
            | "na"
            | "nothing"
            | "none specified"
            | "none noted"
            | "none yet"
            | "none mentioned"
            | "none raised"
            | "none discussed"
    )
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
            "meeting-summary",
            "Meeting summary",
            Place::Below,
            "Summarize this meeting transcript for someone who was in the meeting. \"You\" and \"Them\" are the two sides of the call. Use these headings, and leave out any heading with nothing under it: \"### Overview\" (two or three sentences), \"### Decisions\" (what was agreed, as bullet points), \"### Action items\" (one per line as a Markdown checkbox, \"- [ ] \" then the task, with who will do it and by when only when the transcript says so), and \"### Open questions\" (a question goes here only if nobody answered it before the meeting ended: if the transcript has the answer, leave it out). Use only what was said: never invent a decision, a task, an owner, a date or a number, don't change who said or did what, and keep names, dates and amounts exactly as they are.",
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
    fn meeting_summaries_lose_empty_headings() {
        let answer = "### Overview\nA catch-up about the conference.\n\n### Decisions\n- None\n\n### Action items\n* Send the deck (Priya)\n- [ ] Book the room\n\n### Open questions\n*\n";
        assert_eq!(
            tidy_answer("meeting-summary", answer),
            "### Overview\nA catch-up about the conference.\n\n### Action items\n- [ ] Send the deck (Priya)\n- [ ] Book the room"
        );
        assert_eq!(
            tidy_answer(
                "meeting-summary",
                "### Overview\nShort.\n\n### Action items\n- [ ] None\n"
            ),
            "### Overview\nShort."
        );
        // Other transforms are left as they are.
        assert_eq!(
            tidy_answer("summary", "### Decisions\nNone"),
            "### Decisions\nNone"
        );
    }

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
