//! The language of the conversation on screen, for "Reply in their
//! language": from the window's text as read around the field, with the
//! app's own labels, names and times mixed in. It answers only when it's
//! clear, since a wrong guess puts the wrong language in a message.

/// Characters of message text needed before anything is said.
const LEAST_CHARS: usize = 25;
/// Characters for scripts written without spaces (a few words' worth).
const LEAST_CJK_CHARS: usize = 8;
/// The share of the message text one language must have.
const SHARE: f64 = 0.6;

/// The language to reply in (a two-letter code), when the conversation in
/// `window` is clearly in one the user doesn't dictate in (`spoken`,
/// language tags); `None` when it's in one of them, or unclear.
pub fn reply_language<S: AsRef<str>>(window: &str, spoken: &[S]) -> Option<&'static str> {
    let lang = language(window)?;
    let speaks = spoken
        .iter()
        .any(|s| crate::language::base(s.as_ref()) == lang);
    (!speaks).then_some(lang)
}

/// The conversation's language, when it's clear: most of the messages'
/// text, the latest message included, and reliably detected.
pub fn language(window: &str) -> Option<&'static str> {
    let messages = messages(window);
    let detected: Vec<(&'static str, usize)> = messages
        .iter()
        .filter_map(|m| Some((guess(m)?, m.chars().count())))
        .collect();
    let total: usize = detected.iter().map(|(_, n)| n).sum();
    let mut shares: Vec<(&'static str, usize)> = Vec::new();
    for (lang, n) in &detected {
        match shares.iter_mut().find(|(l, _)| l == lang) {
            Some((_, sum)) => *sum += n,
            None => shares.push((lang, *n)),
        }
    }
    let (lang, chars) = shares.into_iter().max_by_key(|(_, n)| *n)?;
    let enough = if is_cjk(lang) {
        LEAST_CJK_CHARS
    } else {
        LEAST_CHARS
    };
    if chars < enough || (chars as f64) < total as f64 * SHARE {
        return None;
    }
    // The latest message is in it too (newest last in a chat; for mail,
    // the quoted thread below the newest is already cut off). Not a short
    // line, which may be the app's ("Type a new message").
    let latest = detected
        .iter()
        .rev()
        .find(|(l, n)| {
            *n >= if is_cjk(l) {
                LEAST_CJK_CHARS
            } else {
                LEAST_CHARS
            }
        })
        .map(|(l, _)| *l);
    if latest != Some(lang) {
        return None;
    }
    let text: Vec<&str> = messages
        .iter()
        .filter(|m| guess(m) == Some(lang))
        .map(String::as_str)
        .collect();
    let joined = text.join("\n");
    detector()
        .detect(&joined)
        .filter(|info| info.is_reliable() || info.confidence() >= SURE)
        .and_then(|info| crate::aloud::code(info.lang()))
        .filter(|&l| l == lang)
}

/// Confidence that's enough when whatlang doesn't call it reliable (short
/// text, among the languages it's choosing from).
pub(crate) const SURE: f64 = 0.5;

/// whatlang choosing only among the languages Splashdown names: left to
/// itself, it takes Dutch for Afrikaans and short Spanish for Latvian.
pub(crate) fn detector() -> &'static whatlang::Detector {
    static DETECTOR: std::sync::OnceLock<whatlang::Detector> = std::sync::OnceLock::new();
    DETECTOR.get_or_init(|| {
        let langs = whatlang::Lang::all()
            .iter()
            .copied()
            .filter(|&l| crate::aloud::code(l).is_some())
            .collect();
        whatlang::Detector::with_allowlist(langs)
    })
}

/// The lines that look like what people wrote: not the app's labels, not
/// names with times, not mail headers, and nothing after a quoted reply or
/// a signature.
fn messages(window: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in window.lines().map(str::trim) {
        if is_quote_start(line) || line == "--" || line == "-- " {
            break;
        }
        if line.starts_with('>') || is_header(line) {
            continue;
        }
        let words = line.split_whitespace().count();
        let cjk = line.chars().filter(|&c| is_cjk_char(c)).count();
        if words >= 4 || cjk >= LEAST_CJK_CHARS {
            out.push(line.to_string());
        }
    }
    out
}

/// "On Tue, Ben wrote:" and its translations: a mail's quoted thread
/// starts here.
fn is_quote_start(line: &str) -> bool {
    let lower = line.to_lowercase();
    let ends = [
        "wrote:",
        "escribió:",
        "a écrit :",
        "a écrit:",
        "schrieb:",
        "ha scritto:",
        "escreveu:",
        "schreef:",
        "napisał:",
        "napisała:",
    ];
    ends.iter().any(|e| lower.ends_with(e))
        || lower.starts_with("-----original message")
        || lower.starts_with("from:") && lower.contains("sent:")
}

/// A mail header ("From: …", "Von: …") or a name with a time
/// ("Priya Shah 10:02 AM").
fn is_header(line: &str) -> bool {
    let headers = [
        "from:", "to:", "cc:", "subject:", "de:", "para:", "von:", "an:", "à:", "objet:",
        "betreff:", "asunto:", "da:", "a:", "oggetto:",
    ];
    let lower = line.to_lowercase();
    if headers.iter().any(|h| lower.starts_with(h)) {
        return true;
    }
    // Ends with a time: "10:02", "10:02 AM", "2:41 PM".
    let last = line
        .split_whitespace()
        .rev()
        .find(|w| !matches!(*w, "AM" | "PM" | "am" | "pm"));
    last.is_some_and(|w| {
        let (h, m) = w.split_once(':').unwrap_or(("", ""));
        !h.is_empty()
            && h.len() <= 2
            && m.len() == 2
            && (h.to_string() + m).chars().all(|c| c.is_ascii_digit())
    }) && line.split_whitespace().count() <= 5
}

/// One message line's language, if whatlang has any idea.
fn guess(line: &str) -> Option<&'static str> {
    detector()
        .detect(line)
        .and_then(|info| crate::aloud::code(info.lang()))
}

fn is_cjk(lang: &str) -> bool {
    matches!(lang, "zh" | "ja" | "ko" | "th")
}

fn is_cjk_char(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0x0E00..=0x0E7F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_in_another_language_is_answered_in_it() {
        let slack = "#projekt\nLukas Becker 09:12\nGuten Morgen! Habt ihr die neuen Zahlen schon gesehen?\nLukas Becker 09:14\nIch glaube, wir sollten den Termin auf nächste Woche verschieben.\nPeople\nThreads\nMessage #projekt";
        assert_eq!(reply_language(slack, &["en"]), Some("de"));
        // Already the user's language.
        assert_eq!(reply_language(slack, &["de"]), None);
        assert_eq!(reply_language(slack, &["de-DE"]), None);
    }

    #[test]
    fn labels_names_times_and_quotes_dont_count() {
        let english = "Bandeja de entrada (4)\nDestacados\nBorradores\nDe: Sarah Kim\nHi Ben, can we push our review to Thursday? Something came up. Thanks, Sarah";
        assert_eq!(reply_language(english, &["en"]), None);
        let french = "Inbox\nFrom: Antoine Dubois\nBonjour Ben, merci pour votre message. Je suis disponible jeudi matin pour en discuter.\nOn Tue, Ben wrote:\n> Hi Antoine, would you have time this week to talk about the partnership? We would love to show you the product.";
        assert_eq!(reply_language(french, &["en"]), Some("fr"));
    }

    #[test]
    fn unclear_conversations_get_no_answer() {
        assert_eq!(
            reply_language("Messages\nLéa\nok 👍\n😂😂\nDelivered", &["en"]),
            None
        );
        let mixed = "#product\nPriya 10:02 AM\nThe beta build is up, please test the onboarding flow today.\nJordan 10:20 AM\nOn it, I'll file anything I find in Linear.\nDiego 10:31 AM\nPerfecto, yo reviso la parte de pagos esta tarde.";
        assert_eq!(reply_language(mixed, &["en"]), None);
    }

    #[test]
    fn every_language_answered_has_a_name() {
        for lang in whatlang::Lang::all() {
            if let Some(code) = crate::aloud::code(*lang) {
                assert!(crate::language::english_name(code).is_some(), "{code}");
            }
        }
    }
}
