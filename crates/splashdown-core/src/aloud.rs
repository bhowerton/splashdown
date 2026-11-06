//! Reading text aloud ("read this to me", an Ask answer): text tidied for
//! the ear, cut into pieces a voice can start on at once, and the language
//! it's in, so a voice that speaks it can read it.

/// A piece read in one go stays under this many characters, so the first
/// words come quickly even from a long text.
pub const PIECE_CHARS: usize = 400;

/// `text` as it should be heard: Markdown marks, bullets and code blocks
/// left out, a link read as its words (or its site), and a line that ends
/// without punctuation given a full stop so the voice pauses there.
pub fn for_listening(text: &str) -> String {
    let mut lines = Vec::new();
    let mut code = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("```") {
            code = !code;
            continue;
        }
        if code {
            continue;
        }
        let line = line.trim_start_matches('#').trim_start();
        let line = ["- ", "* ", "• ", "> "]
            .iter()
            .find_map(|b| line.strip_prefix(b))
            .unwrap_or(line);
        let mut line = links(line)
            .replace("**", "")
            .replace("__", "")
            .replace('`', "");
        if line.is_empty() {
            lines.push(String::new());
            continue;
        }
        if !line.ends_with(|c: char| ".!?:;,。！？…".contains(c)) {
            line.push('.');
        }
        lines.push(line);
    }
    let joined = lines.join("\n");
    // At most one blank line in a row.
    let mut out = String::with_capacity(joined.len());
    for part in joined.split("\n\n").filter(|p| !p.trim().is_empty()) {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(part.trim());
    }
    out
}

/// `[words](url)` as its words; a bare web address as its site.
fn links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find("](") else { break };
        let Some(end) = after[close + 2..].find(')') else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&after[..close]);
        rest = &after[close + 2 + end + 1..];
    }
    out.push_str(rest);
    out.split(' ')
        .map(|word| {
            match word
                .strip_prefix("https://")
                .or(word.strip_prefix("http://"))
            {
                Some(address) => {
                    let site = address.split(['/', '?', '#']).next().unwrap_or(address);
                    site.strip_prefix("www.").unwrap_or(site).to_string()
                }
                None => word.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where the last complete sentence in `text` ends (a byte offset), if
/// any: after `.`, `!`, `?` and the like followed by a space, or at a line
/// break. For reading an answer while it's still being written.
pub fn sentence_end(text: &str) -> Option<usize> {
    let mut end = None;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let ends = match c {
            '\n' => true,
            '.' | '!' | '?' | '…' | ':' | ';' => next.is_some_and(char::is_whitespace),
            // These need no space after them.
            '。' | '！' | '？' => true,
            _ => false,
        };
        if ends {
            end = Some(i + c.len_utf8());
        }
    }
    end
}

/// `text` cut into pieces to read one after another: whole sentences, up
/// to `PIECE_CHARS` each (a longer sentence is a piece of its own).
pub fn pieces(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let cut = first_sentence_end(rest).unwrap_or(rest.len());
        let sentence = &rest[..cut];
        rest = &rest[cut..];
        if !current.is_empty() && current.chars().count() + sentence.chars().count() > PIECE_CHARS {
            out.push(std::mem::take(&mut current).trim().to_string());
        }
        current.push_str(sentence);
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out.retain(|p| !p.is_empty());
    out
}

fn first_sentence_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let ends = match c {
            '\n' => true,
            '.' | '!' | '?' | '…' => next.is_some_and(char::is_whitespace),
            '。' | '！' | '？' => true,
            _ => false,
        };
        if ends {
            return Some(i + c.len_utf8());
        }
    }
    None
}

/// The language `text` is in, as a two-letter code ("de"), when it's
/// clear enough to pick a voice by.
pub fn language(text: &str) -> Option<&'static str> {
    whatlang::detect(text)
        .filter(whatlang::Info::is_reliable)
        .and_then(|info| code(info.lang()))
}

/// The language `text` is most likely in, however short it is (for
/// checking what a model wrote).
pub fn language_guess(text: &str) -> Option<&'static str> {
    whatlang::detect(text).and_then(|info| code(info.lang()))
}

pub(crate) fn code(lang: whatlang::Lang) -> Option<&'static str> {
    use whatlang::Lang::*;
    Some(match lang {
        Eng => "en",
        Deu => "de",
        Fra => "fr",
        Spa => "es",
        Ita => "it",
        Por => "pt",
        Nld => "nl",
        Swe => "sv",
        Dan => "da",
        Nob => "nb",
        Fin => "fi",
        Pol => "pl",
        Ces => "cs",
        Slk => "sk",
        Hun => "hu",
        Ron => "ro",
        Bul => "bg",
        Hrv => "hr",
        Ell => "el",
        Tur => "tr",
        Rus => "ru",
        Ukr => "uk",
        Heb => "he",
        Ara => "ar",
        Hin => "hi",
        Tha => "th",
        Vie => "vi",
        Ind => "id",
        Cat => "ca",
        Cmn => "zh",
        Jpn => "ja",
        Kor => "ko",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_left_out() {
        let answer = "## Summary\n\n**Pricing** stays at `$10`:\n- one plan\n- [the details](https://example.com/p)\n\n```\ncode\n```\nSee https://www.example.com/a?b for more";
        assert_eq!(
            for_listening(answer),
            "Summary.\n\nPricing stays at $10:\none plan.\nthe details.\n\nSee example.com for more."
        );
    }

    #[test]
    fn finds_where_the_last_sentence_ends() {
        assert_eq!(sentence_end("Hello there"), None);
        assert_eq!(sentence_end("Hello there. How"), Some(12));
        assert_eq!(sentence_end("Version 1.5 is"), None);
        assert_eq!(sentence_end("One.\nTwo"), Some(5));
        assert_eq!(sentence_end("你好。再见"), Some("你好。".len()));
    }

    #[test]
    fn long_text_is_read_in_pieces() {
        let sentence = "This sentence has exactly fifty characters in it. ";
        let text = sentence.repeat(20);
        let pieces = pieces(&text);
        assert_eq!(pieces.len(), 3);
        assert!(pieces.iter().all(|p| p.chars().count() <= PIECE_CHARS));
        assert_eq!(pieces.join(" ") + " ", text);
        assert_eq!(super::pieces("Short."), vec!["Short."]);
        assert!(super::pieces("  ").is_empty());
    }

    #[test]
    fn knows_the_language() {
        assert_eq!(
            language("Ich habe gestern mit dem Team über die neuen Preise gesprochen."),
            Some("de")
        );
        assert_eq!(
            language("We talked about the new prices with the team yesterday afternoon."),
            Some("en")
        );
        assert_eq!(language("OK"), None);
    }
}
