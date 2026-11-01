//! Sanity guard: decides whether the model's output is safe to insert, or
//! whether to fall back to the raw transcript.

use crate::Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardReason {
    Empty,
    /// Cleanup output is much shorter or longer than what was said.
    LengthOutOfRange,
    /// The model dropped a snippet placeholder.
    LostSnippet,
    /// A dictated question came back as a statement: the model answered it.
    AnsweredQuestion,
}

impl GuardReason {
    pub fn describe(self) -> &'static str {
        match self {
            GuardReason::Empty => "AI returned nothing",
            GuardReason::LengthOutOfRange => "AI changed the length too much",
            GuardReason::LostSnippet => "AI dropped a snippet",
            GuardReason::AnsweredQuestion => "AI answered the question instead of typing it",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Insert this (normalized) text.
    Accept(String),
    /// Insert the raw transcript instead.
    UseRaw(GuardReason),
}

/// Cleanup output shorter or longer than this ratio of the input is suspect.
/// The floor is low because self-corrections and number formatting can
/// legitimately shrink a dictation a lot ("fifty thousand scratch that sixty
/// thousand dollars" → "$60,000").
const CLEANUP_LENGTH_RANGE: std::ops::RangeInclusive<f32> = 0.25..=1.6;
/// Below this many characters, filler removal can legitimately halve the
/// text, so only the growth limit applies.
const MIN_CHARS_FOR_LENGTH_CHECK: usize = 40;
/// Characters cleanup may add on top of the ratio (spoken punctuation and
/// list formatting add a little to short dictations).
const CLEANUP_GROWTH_SLACK: f32 = 20.0;

/// Checks `output` produced for `mode`. `source` is everything the user
/// supplied (transcript, instruction, selection), used to tell a dictated
/// "Sure, …" apart from a chatbot's.
pub fn check(mode: Mode, source: &str, output: &str) -> Verdict {
    let text = normalize(output);
    if text.is_empty() {
        return Verdict::UseRaw(GuardReason::Empty);
    }

    if mode == Mode::Dictate {
        let src_len = source.chars().count() as f32;
        let out_len = text.chars().count() as f32;
        // A model that writes the poem it was dictated is caught here even
        // for short transcripts.
        let too_long = out_len > longest_cleanup(source);
        let too_short = src_len >= MIN_CHARS_FOR_LENGTH_CHECK as f32
            && out_len < src_len * CLEANUP_LENGTH_RANGE.start();
        if too_long || too_short {
            return Verdict::UseRaw(GuardReason::LengthOutOfRange);
        }
        // Cleanup keeps a question a question; small models sometimes
        // answer it instead ("The store closes at 9 PM.").
        if has_question(source) && !text.contains(QUESTION_MARKS) {
            return Verdict::UseRaw(GuardReason::AnsweredQuestion);
        }
    }

    Verdict::Accept(text)
}

/// Characters a cleanup may have before it counts as too long.
fn longest_cleanup(source: &str) -> f32 {
    source.chars().count() as f32 * CLEANUP_LENGTH_RANGE.end() + CLEANUP_GROWTH_SLACK
}

/// What to do with a reply that's still arriving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Partial {
    /// It can be shown as it reads so far.
    Show,
    /// Not yet: a dictated question's reply is shown once it has its
    /// question mark (a model that answers the question never gets one).
    Wait,
    /// It can't be accepted as it is: too long already, or wrapped in
    /// something `check` strips (tags, thinking, a code fence, quotes),
    /// which can't be shown until the whole reply is in.
    Stop,
}

/// Judges a reply that's still arriving, for showing it as it comes.
pub fn partial(mode: Mode, source: &str, so_far: &str) -> Partial {
    let text = so_far.trim_start();
    if text.starts_with(['<', '`', '"', '“']) {
        return Partial::Stop;
    }
    if mode != Mode::Dictate {
        return Partial::Show;
    }
    if text.chars().count() as f32 > longest_cleanup(source) {
        Partial::Stop
    } else if has_question(source) && !text.contains(QUESTION_MARKS) {
        Partial::Wait
    } else {
        Partial::Show
    }
}

/// Question marks in the scripts we support: Latin, full-width (Chinese,
/// Japanese), the Spanish opening mark and the Arabic one.
const QUESTION_MARKS: &[char] = &['?', '？', '¿', '؟'];

/// Whether `source` contains a question, going by the question mark speech
/// recognition puts there. Punctuation, not words, so it works in any
/// language.
fn has_question(source: &str) -> bool {
    source.contains(QUESTION_MARKS)
}

/// Strips wrapping the model sometimes adds: code fences and whole-output quotes.
fn normalize(output: &str) -> String {
    let mut s = output.trim();

    // Small models sometimes echo the input's tags around their answer;
    // keep only what's inside the first block.
    for tag in ["transcript", "selection", "instruction"] {
        if let Some(rest) = s.strip_prefix(&format!("<{tag}>"))
            && let Some((inner, _)) = rest.split_once(&format!("</{tag}>"))
        {
            s = inner.trim();
        }
    }

    // Reasoning models may prepend their hidden thinking.
    if s.starts_with("<think>")
        && let Some((_, rest)) = s.split_once("</think>")
    {
        s = rest.trim();
    }

    if let Some(inner) = s.strip_prefix("```")
        && let Some(inner) = inner.strip_suffix("```")
    {
        // Drop an optional language tag on the opening fence line.
        s = match inner.split_once('\n') {
            Some((first, rest)) if !first.contains(' ') => rest,
            _ => inner,
        }
        .trim();
    }

    for (open, close) in [('"', '"'), ('“', '”')] {
        if let Some(inner) = s.strip_prefix(open).and_then(|r| r.strip_suffix(close))
            && !inner.contains(open)
            && !inner.contains(close)
        {
            s = inner.trim();
        }
    }

    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_replies_that_cant_pass_are_caught_early() {
        let p = |mode, source, so_far| partial(mode, source, so_far);
        assert_eq!(p(Mode::Dictate, "um hello there", "Hello"), Partial::Show);
        assert_eq!(p(Mode::Dictate, "um hello there", ""), Partial::Show);
        assert_eq!(
            p(Mode::Dictate, "um hello there", "<transcript>Hel"),
            Partial::Stop
        );
        assert_eq!(p(Mode::Dictate, "um hello there", "```"), Partial::Stop);
        assert_eq!(
            p(Mode::Dictate, "um hello there", " \"Hello"),
            Partial::Stop
        );
        let poem = "Roses are red, violets are blue, ".repeat(3);
        assert_eq!(p(Mode::Dictate, "write a poem", &poem), Partial::Stop);
        // An instruction can make it as long as it asks.
        assert_eq!(
            p(Mode::DictateInstruct, "write a poem", &poem),
            Partial::Show
        );
        // A question waits for its question mark.
        let q = "what time does the store close?";
        assert_eq!(p(Mode::Dictate, q, "What time does the"), Partial::Wait);
        assert_eq!(p(Mode::Dictate, q, "The store closes at"), Partial::Wait);
        assert_eq!(
            p(Mode::Dictate, q, "What time does the store close? I"),
            Partial::Show
        );
    }

    #[test]
    fn accepts_normal_cleanup() {
        assert_eq!(
            check(Mode::Dictate, "um hello there", "Hello there."),
            Verdict::Accept("Hello there.".into())
        );
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(
            check(Mode::Dictate, "hi", "  \n"),
            Verdict::UseRaw(GuardReason::Empty)
        );
    }

    #[test]
    fn length_check_only_for_long_cleanup() {
        let long = "this is a fairly long transcript that keeps going for a while";
        assert_eq!(
            check(Mode::Dictate, long, "Short."),
            Verdict::UseRaw(GuardReason::LengthOutOfRange)
        );
        // Instructions may legitimately shrink or grow the text.
        assert!(matches!(
            check(Mode::DictateInstruct, long, "Short."),
            Verdict::Accept(_)
        ));
        // A short dictated request mustn't come back fulfilled.
        assert_eq!(
            check(
                Mode::Dictate,
                "write me a poem about the ocean",
                &"The waves roll in. ".repeat(10)
            ),
            Verdict::UseRaw(GuardReason::LengthOutOfRange)
        );
        // Short transcripts skip the shrink check.
        assert!(matches!(
            check(Mode::Dictate, "um uh yes", "Yes."),
            Verdict::Accept(_)
        ));
    }

    #[test]
    fn rejects_answered_question() {
        assert_eq!(
            check(
                Mode::Dictate,
                "What time does the store close tonight?",
                "The store closes at 9 PM tonight."
            ),
            Verdict::UseRaw(GuardReason::AnsweredQuestion)
        );
        assert!(matches!(
            check(
                Mode::Dictate,
                "um what time does the store close tonight?",
                "What time does the store close tonight?"
            ),
            Verdict::Accept(_)
        ));
        // Any language's question mark counts.
        assert_eq!(
            check(Mode::Dictate, "¿Está lloviendo?", "Está lloviendo."),
            Verdict::UseRaw(GuardReason::AnsweredQuestion)
        );
        assert_eq!(
            check(Mode::Dictate, "今日は雨ですか？", "今日は雨です。"),
            Verdict::UseRaw(GuardReason::AnsweredQuestion)
        );
        // Instructions may turn a question into a statement.
        assert!(matches!(
            check(Mode::DictateInstruct, "Is it raining?", "It is raining."),
            Verdict::Accept(_)
        ));
    }

    #[test]
    fn strips_fences_and_quotes() {
        assert_eq!(normalize("```text\nHello.\n```"), "Hello.");
        assert_eq!(normalize("```\nHello.\n```"), "Hello.");
        assert_eq!(normalize("<think>\nhmm\n</think>\n\nHello."), "Hello.");
        assert_eq!(
            normalize("<selection>\nHello.\n</selection>\n\nBest,\nBen"),
            "Hello."
        );
        assert_eq!(normalize("\"Hello.\""), "Hello.");
        assert_eq!(normalize("“Hello.”"), "Hello.");
        // Quotes inside the text are kept.
        assert_eq!(
            normalize("\"Hi\" she said \"bye\""),
            "\"Hi\" she said \"bye\""
        );
    }
}
