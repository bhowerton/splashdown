//! Snippets (the user's own trigger phrases, in whatever language they're
//! written in) and finding sentences in a field's text. Nothing here
//! matches built-in words: spoken formatting ("new paragraph") is left to
//! the AI pass, and commands are interpreted by the AI (`crate::command`),
//! so both work in any language.

use serde::{Deserialize, Serialize};

/// Saved text inserted when its trigger phrase is spoken ("my address").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub trigger: String,
    pub text: String,
}

/// Placeholder that stands in for snippet `i` during the AI pass, so the
/// model can't reword the saved text.
pub fn placeholder(i: usize) -> String {
    format!("[[s{}]]", i + 1)
}

/// Replaces trigger phrases with placeholders. Returns the marked text and
/// the indices of the snippets used, in order of first use.
pub fn mark_snippets(text: &str, snippets: &[Snippet]) -> (String, Vec<usize>) {
    let triggers: Vec<Vec<String>> = snippets
        .iter()
        .map(|s| {
            s.trigger
                .split_whitespace()
                .map(|w| bare(w).to_lowercase())
                .filter(|w| !w.is_empty())
                .collect()
        })
        .collect();
    // A trigger in a script written without spaces (Chinese, Japanese,
    // Thai) sits inside a sentence with nothing around it to split on:
    // those are found wherever they are, longest first.
    let mut text = text.to_string();
    let mut used = Vec::new();
    let mut spaceless: Vec<(usize, &str)> = snippets
        .iter()
        .enumerate()
        .map(|(i, s)| (i, s.trigger.trim()))
        .filter(|(_, t)| t.chars().any(unspaced))
        .collect();
    spaceless.sort_by_key(|(_, t)| std::cmp::Reverse(t.chars().count()));
    for (i, trigger) in spaceless {
        if text.contains(trigger) {
            text = text.replace(trigger, &placeholder(i));
            used.push(i);
        }
    }

    let tokens: Vec<&str> = text.split_whitespace().collect();
    if triggers.iter().all(Vec::is_empty) || tokens.is_empty() {
        return (text.clone(), used);
    }

    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        // Longest trigger first, so "my work email" beats "my work".
        let hit = triggers
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.is_empty() && t.len() <= tokens.len() - i)
            .filter(|(_, t)| {
                t.iter()
                    .zip(&tokens[i..])
                    // Any alphabet's capitals: a trigger said at the
                    // start of a sentence comes capitalized.
                    .all(|(w, tok)| bare(tok).to_lowercase() == *w)
            })
            .max_by_key(|(_, t)| t.len());
        match hit {
            Some((idx, t)) => {
                let last = tokens[i + t.len() - 1];
                let trailing: String = last
                    .chars()
                    .rev()
                    .take_while(|c| !c.is_alphanumeric())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                out.push(format!("{}{trailing}", placeholder(idx)));
                if !used.contains(&idx) {
                    used.push(idx);
                }
                i += t.len();
            }
            None => {
                out.push(tokens[i].to_string());
                i += 1;
            }
        }
    }
    (out.join(" "), used)
}

/// Puts the saved text back in place of placeholders. Returns `None` if a
/// placeholder for a used snippet is missing (the model dropped it).
pub fn fill_snippets(text: &str, used: &[usize], snippets: &[Snippet]) -> Option<String> {
    let mut out = text.to_string();
    for &i in used {
        let ph = placeholder(i);
        if !out.contains(&ph) {
            return None;
        }
        out = out.replace(&ph, &snippets[i].text);
    }
    Some(out)
}

/// Where the last sentence before `end` starts (a byte offset into
/// `text`): after the previous sentence's end mark or line break.
pub fn last_sentence_start(text: &str, end: usize) -> usize {
    let before = text[..end].trim_end();
    // Skip the sentence's own closing mark.
    let body = before.trim_end_matches(['.', '!', '?', '…', '。', '！', '？']);
    let boundary = body
        .char_indices()
        .rev()
        .find(|&(i, c)| {
            c == '\n'
                // Full-width marks (Chinese, Japanese) end a sentence on
                // their own; others need a space after them.
                || matches!(c, '。' | '！' | '？')
                || (matches!(c, '.' | '!' | '?' | '…')
                    && body[i + c.len_utf8()..].starts_with(char::is_whitespace))
        })
        .map_or(0, |(i, c)| i + c.len_utf8());
    // Start at the sentence's first non-space character.
    boundary + text[boundary..end].len() - text[boundary..end].trim_start().len()
}

/// Whether dictated text starting with `first` needs a space before it
/// when it goes right after `before` (the character before the cursor).
/// Decided by characters only, so it works in any language: no space after
/// whitespace or an opening bracket, before closing punctuation, or in
/// scripts written without spaces (Chinese, Japanese, Thai…).
pub fn needs_space(before: Option<char>, first: Option<char>) -> bool {
    let (Some(before), Some(first)) = (before, first) else {
        return false;
    };
    const OPENING: &[char] = &[
        '(', '[', '{', '“', '‘', '«', '¿', '¡', '/', '@', '#', '\u{2014}',
    ];
    const CLOSING: &[char] = &[
        '.', ',', ';', ':', '!', '?', ')', ']', '}', '”', '’', '»', '%', '…', '\u{2014}',
    ];
    !before.is_whitespace()
        && !first.is_whitespace()
        && !OPENING.contains(&before)
        && !CLOSING.contains(&first)
        && !unspaced(before)
        && !unspaced(first)
}

/// Characters of scripts written without spaces between words, and
/// full-width punctuation.
pub(crate) fn unspaced(c: char) -> bool {
    matches!(c as u32,
        0x0E00..=0x0EFF   // Thai, Lao
        | 0x1000..=0x109F // Myanmar
        | 0x1780..=0x17FF // Khmer
        | 0x3000..=0x30FF // CJK punctuation, Hiragana, Katakana
        | 0x3400..=0x4DBF // CJK Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xFF00..=0xFFEF // Full-width forms
    )
}

/// The word without surrounding punctuation.
fn bare(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snips() -> Vec<Snippet> {
        vec![
            Snippet {
                trigger: "my address".into(),
                text: "1 Infinite Loop\nCupertino".into(),
            },
            Snippet {
                trigger: "my email".into(),
                text: "ben@example.com".into(),
            },
            Snippet {
                trigger: "my work email".into(),
                text: "ben@work.example".into(),
            },
        ]
    }

    #[test]
    fn marks_and_fills_snippets() {
        let s = snips();
        let (marked, used) = mark_snippets("Send it to My Email, please", &s);
        assert_eq!(marked, "Send it to [[s2]], please");
        assert_eq!(used, vec![1]);
        assert_eq!(
            fill_snippets("Send it to [[s2]], please.", &used, &s).unwrap(),
            "Send it to ben@example.com, please."
        );
        assert_eq!(
            fill_snippets("Send it to me.", &used, &s),
            None,
            "dropped placeholder"
        );
    }

    #[test]
    fn triggers_work_in_other_alphabets_and_scripts() {
        let s = vec![
            Snippet {
                trigger: "мой адрес".into(),
                text: "Москва".into(),
            },
            Snippet {
                trigger: "Über mich".into(),
                text: "Ich bin Ben.".into(),
            },
            Snippet {
                trigger: "我的地址".into(),
                text: "北京市".into(),
            },
            Snippet {
                trigger: "我的地址簿".into(),
                text: "通讯录".into(),
            },
        ];
        // Capitalized at the start of a sentence, as speech-to-text writes it.
        assert_eq!(mark_snippets("Мой адрес.", &s), ("[[s1]].".into(), vec![0]));
        assert_eq!(
            mark_snippets("das steht in über mich", &s).0,
            "das steht in [[s2]]"
        );
        // Inside a sentence written without spaces; the longer trigger first.
        assert_eq!(
            mark_snippets("请寄到我的地址。", &s),
            ("请寄到[[s3]]。".into(), vec![2])
        );
        assert_eq!(mark_snippets("打开我的地址簿", &s).0, "打开[[s4]]");
        let (marked, used) = mark_snippets("请寄到我的地址。", &s);
        assert_eq!(fill_snippets(&marked, &used, &s).unwrap(), "请寄到北京市。");
    }

    #[test]
    fn longest_trigger_wins_and_no_partial_words() {
        let s = snips();
        assert_eq!(mark_snippets("use my work email.", &s).0, "use [[s3]].");
        assert_eq!(mark_snippets("my emailing list", &s).0, "my emailing list");
        assert_eq!(mark_snippets("nothing here", &[]).0, "nothing here");
    }

    #[test]
    fn spaces_between_dictations() {
        // After a sentence or a word: a space.
        assert!(needs_space(Some('.'), Some('D')));
        assert!(needs_space(Some('d'), Some('a')));
        // At the start, after a space or line break, or an opening bracket.
        assert!(!needs_space(None, Some('A')));
        assert!(!needs_space(Some(' '), Some('A')));
        assert!(!needs_space(Some('\n'), Some('A')));
        assert!(!needs_space(Some('('), Some('a')));
        // Before punctuation.
        assert!(!needs_space(Some('d'), Some(',')));
        assert!(!needs_space(Some('d'), Some('?')));
        // Scripts without spaces.
        assert!(!needs_space(Some('。'), Some('今')));
        assert!(!needs_space(Some('日'), Some('本')));
        assert!(!needs_space(Some('ค'), Some('น')));
        // Korean does use spaces.
        assert!(needs_space(Some('다'), Some('그')));
    }

    #[test]
    fn finds_the_last_sentence() {
        let t = "First one. Second one! Third one.";
        assert_eq!(&t[last_sentence_start(t, t.len())..], "Third one.");
        let t = "Only one sentence here";
        assert_eq!(last_sentence_start(t, t.len()), 0);
        let t = "Line one\nline two. And this";
        assert_eq!(&t[last_sentence_start(t, t.len())..], "And this");
        // Stops at the cursor, not the end of the text.
        let t = "One. Two. Three.";
        assert_eq!(&t[last_sentence_start(t, 9)..9], "Two.");
        // Version numbers aren't sentence ends.
        let t = "一句。二句。三句";
        assert_eq!(&t[last_sentence_start(t, t.len())..], "三句");
        let t = "Use 2.5 now. Then upgrade to v3.1";
        assert_eq!(
            &t[last_sentence_start(t, t.len())..],
            "Then upgrade to v3.1"
        );
    }
}
