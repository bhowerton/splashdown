//! The language of a meeting (or any recording transcribed turn by turn)
//! when the user's spoken language is left on Automatic. A multilingual
//! speech model then guesses each turn's language on its own, and on a
//! short or noisy turn it can guess wrong ("क्यों बेबी" in an English
//! call). This tells which language the recording is in, and which turns
//! came out in another one, so they can be transcribed again in it.
//!
//! Only short turns are ever redone: a real conversation in two languages
//! has long turns in each, and those are kept as heard.

/// Text needed before the recording's language is taken from what was
/// said rather than the computer's language; tried again after as much
/// more each time it isn't clear yet.
const SETTLE_CHARS: usize = 200;
/// Past this much text with no clear language, it stops trying (and keeps
/// what it expected): a long meeting mustn't be detected over and over.
const MOST_CHARS: usize = 2_000;
/// A turn longer than this is never redone, in words.
const MOST_WORDS: usize = 15;
/// …or, for scripts written without spaces, characters.
const MOST_CJK_CHARS: usize = 30;

/// A recording's language, learned turn by turn (`heard`), and which
/// turns stray from it (`is_stray`).
pub struct TurnLanguage {
    /// What to expect until the recording settles: the computer's
    /// language (two-letter code).
    expected: Option<String>,
    said: String,
    /// Characters in `said` when it was last tried.
    tried_at: usize,
    settled: Option<&'static str>,
}

impl TurnLanguage {
    /// Starts out expecting `expected` (a language tag such as "en-US",
    /// usually the computer's language).
    pub fn new(expected: Option<String>) -> Self {
        Self {
            expected: expected.map(|e| crate::language::base(&e)),
            said: String::new(),
            tried_at: 0,
            settled: None,
        }
    }

    /// The recording's language: settled from what was said, or expected.
    pub fn language(&self) -> Option<&str> {
        self.settled.or(self.expected.as_deref())
    }

    /// Notes a turn as transcribed (after any redo).
    pub fn heard(&mut self, text: &str) {
        if self.settled.is_some() || self.tried_at >= MOST_CHARS || text.trim().is_empty() {
            return;
        }
        self.said.push_str(text);
        self.said.push('\n');
        let chars = self.said.chars().count();
        if chars >= self.tried_at + SETTLE_CHARS {
            self.tried_at = chars;
            self.settled = clearly(&self.said);
            if self.settled.is_some() || chars >= MOST_CHARS {
                self.said = String::new();
            }
        }
    }

    /// Whether `text`, a short turn, came out in another language than the
    /// recording's: letters of another script, or clearly another
    /// language. Such a turn is worth transcribing again in the
    /// recording's language.
    pub fn is_stray(&self, text: &str) -> bool {
        let Some(lang) = self.language() else {
            return false;
        };
        let text = text.trim();
        let cjk = text
            .chars()
            .filter(|&c| script(c) == Some(Script::Cjk))
            .count();
        if text.is_empty() || text.split_whitespace().count() > MOST_WORDS || cjk > MOST_CJK_CHARS {
            return false;
        }
        let ours = scripts_of(lang);
        let foreign_script = text.chars().filter_map(script).any(|s| !ours.contains(&s));
        if foreign_script {
            return true;
        }
        // Same script: only when it's plainly another language (a few
        // words aren't enough to tell).
        text.split_whitespace().count() >= 4 && clearly(text).is_some_and(|l| l != lang)
    }
}

/// The language `text` is clearly in, if it is.
fn clearly(text: &str) -> Option<&'static str> {
    crate::conversation::detector()
        .detect(text)
        .filter(|info| info.is_reliable() || info.confidence() >= crate::conversation::SURE)
        .and_then(|info| crate::aloud::code(info.lang()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    Devanagari,
    Bengali,
    Thai,
    Hangul,
    /// Chinese characters, and Japanese kana with them.
    Cjk,
}

/// The script of a letter (`None` for digits, punctuation, emoji…).
fn script(c: char) -> Option<Script> {
    if !c.is_alphabetic() {
        return None;
    }
    Some(match c as u32 {
        0x0041..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F => Script::Cyrillic,
        0x0590..=0x05FF => Script::Hebrew,
        0x0600..=0x06FF | 0x0750..=0x077F => Script::Arabic,
        0x0900..=0x097F => Script::Devanagari,
        0x0980..=0x09FF => Script::Bengali,
        0x0E00..=0x0E7F => Script::Thai,
        0x1100..=0x11FF | 0xAC00..=0xD7AF => Script::Hangul,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF => Script::Cjk,
        _ => return None,
    })
}

/// The scripts `lang` is written in. Latin is always allowed: names,
/// brands and code turn up in every language.
fn scripts_of(lang: &str) -> Vec<Script> {
    let own = match lang {
        "ru" | "uk" | "bg" => Some(Script::Cyrillic),
        "el" => Some(Script::Greek),
        "ar" => Some(Script::Arabic),
        "he" => Some(Script::Hebrew),
        "hi" => Some(Script::Devanagari),
        "bn" => Some(Script::Bengali),
        "th" => Some(Script::Thai),
        "ko" => Some(Script::Hangul),
        "zh" | "ja" => Some(Script::Cjk),
        _ => None,
    };
    std::iter::once(Script::Latin).chain(own).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_turn_in_another_script_is_stray() {
        let lang = TurnLanguage::new(Some("en-US".into()));
        assert_eq!(lang.language(), Some("en"));
        assert!(lang.is_stray("Going above, but check out this baby. Hold on. क्यों बेबी"));
        assert!(lang.is_stray("クリック"));
        assert!(!lang.is_stray("Screens right here."));
        assert!(!lang.is_stray("No it's up."));
        // Names and code in Latin letters are fine in any language.
        let hindi = TurnLanguage::new(Some("hi".into()));
        assert!(!hindi.is_stray("हाँ, Priya ने GitHub पर भेजा"));
    }

    #[test]
    fn a_long_turn_is_kept_as_heard() {
        let lang = TurnLanguage::new(Some("en".into()));
        let spanish = "Bueno, entonces revisamos el presupuesto mañana por la mañana y \
                       después hablamos con el equipo de diseño sobre las fechas de entrega";
        assert!(!lang.is_stray(spanish));
        // A short one plainly in another language is redone.
        assert!(lang.is_stray("Bueno, entonces revisamos el presupuesto mañana"));
    }

    #[test]
    fn the_language_spoken_wins_over_the_computers() {
        let mut lang = TurnLanguage::new(Some("en".into()));
        for _ in 0..4 {
            lang.heard(
                "Bueno, entonces revisamos el presupuesto mañana por la mañana y después hablamos con el equipo.",
            );
        }
        assert_eq!(lang.language(), Some("es"));
        // Now English is the stray one, when it's clear enough to tell; a
        // few words aren't, and are kept as heard.
        assert!(
            lang.is_stray("Okay so we can review the whole budget with the team tomorrow morning")
        );
        assert!(!lang.is_stray("Okay, sure."));
    }

    #[test]
    fn nothing_to_go_on_redoes_nothing() {
        let lang = TurnLanguage::new(None);
        assert!(!lang.is_stray("क्यों बेबी"));
    }

    #[test]
    fn unclear_text_is_given_up_on() {
        let mut lang = TurnLanguage::new(Some("en".into()));
        for _ in 0..500 {
            lang.heard("ok ok 123 hm");
        }
        assert!(lang.said.is_empty(), "stopped keeping what was said");
        assert_eq!(lang.language(), Some("en"));
    }
}
