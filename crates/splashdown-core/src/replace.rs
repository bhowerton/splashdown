//! Find and replace, done exactly: "change every Tuesday to Thursday" is
//! worked out by the AI as what to find and what goes in its place
//! (`crate::command::Command::FindReplace`), and applied here, so nothing
//! else in the text can change and a long text can't be cut short.
//!
//! Matching ignores case and, where `find` begins or ends with a letter or
//! digit, wants whole words, as the find-and-replace rules in Settings do.
//! Writing without spaces between words (Chinese, Japanese, Thai…) has no
//! word edges to find, so there a match may sit inside a run of letters.

use serde::{Deserialize, Serialize};

/// One replacement: every `find` becomes `replace` ("" removes it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Edit {
    pub find: String,
    pub replace: String,
}

impl Edit {
    pub fn new(find: impl Into<String>, replace: impl Into<String>) -> Self {
        Self {
            find: find.into(),
            replace: replace.into(),
        }
    }
}

/// What `apply` changed: each edit, and how many times it was made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replaced {
    pub edits: Vec<Edit>,
    /// One count per edit, in the same order.
    pub counts: Vec<usize>,
}

impl Replaced {
    /// How many changes there were in all.
    pub fn total(&self) -> usize {
        self.counts.iter().sum()
    }

    /// What happened, for a result line: "Changed 4 · Tuesday → Thursday",
    /// "Removed 3 · very", or with several, "Changed 5 · Tuesday →
    /// Thursday, removed very". Edits that found nothing aren't listed.
    pub fn summary(&self) -> String {
        let made: Vec<&Edit> = self
            .edits
            .iter()
            .zip(&self.counts)
            .filter(|(_, n)| **n > 0)
            .map(|(e, _)| e)
            .collect();
        let total = self.total();
        if made.is_empty() {
            return "Nothing changed".to_string();
        }
        if made.iter().all(|e| e.replace.is_empty()) {
            let words: Vec<&str> = made.iter().map(|e| e.find.as_str()).collect();
            return format!("Removed {total} · {}", words.join(", "));
        }
        let parts: Vec<String> = made
            .iter()
            .map(|e| {
                if e.replace.is_empty() {
                    format!("removed {}", e.find)
                } else {
                    format!("{} → {}", e.find, e.replace)
                }
            })
            .collect();
        format!("Changed {total} · {}", parts.join(", "))
    }
}

/// `edits` made usable: spaces trimmed and runs of them made one, quotes
/// around a word taken off ("'very'"), and edits with nothing to find, or
/// that change nothing, or that repeat an earlier `find`, dropped.
pub fn clean(edits: Vec<Edit>) -> Vec<Edit> {
    let mut out: Vec<Edit> = Vec::new();
    for edit in edits {
        let find = unquote(&collapse(&edit.find)).to_string();
        let replace = unquote(&collapse(&edit.replace)).to_string();
        if find.is_empty() || find == replace {
            continue;
        }
        let lower = find.to_lowercase();
        if out.iter().any(|e| e.find.to_lowercase() == lower) {
            continue;
        }
        out.push(Edit { find, replace });
    }
    out
}

/// `text` with every edit made, in one pass over the original (one edit's
/// result is never matched by another), and what was changed. Where two
/// edits could match at the same place, the longer `find` wins. Edits
/// should be `clean`ed first.
pub fn apply(text: &str, edits: &[Edit]) -> (String, Replaced) {
    let mut found: Vec<(usize, usize, usize)> = Vec::new();
    for (i, edit) in edits.iter().enumerate() {
        let find: Vec<char> = edit.find.chars().collect();
        if find.is_empty() {
            continue;
        }
        let mut at = 0;
        while at < text.len() {
            if let Some(end) = match_at(text, at, &find).filter(|&end| whole(text, at, end, &find))
            {
                found.push((at, end, i));
                at = end;
            } else {
                at += text[at..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    // Earliest first; at the same place, the longest.
    found.sort_by_key(|&(start, end, i)| (start, std::cmp::Reverse(end), i));

    let mut counts = vec![0; edits.len()];
    let mut out = String::with_capacity(text.len());
    // Where something was taken out, in `out`, to tidy the spaces there.
    let mut cuts = Vec::new();
    let mut done = 0;
    for (start, end, i) in found {
        if start < done {
            continue;
        }
        out.push_str(&text[done..start]);
        let edit = &edits[i];
        if edit.replace.is_empty() {
            cuts.push(out.len());
        } else {
            out.push_str(&in_case_of(&text[start..end], edit));
        }
        counts[i] += 1;
        done = end;
    }
    out.push_str(&text[done..]);
    for at in cuts.into_iter().rev() {
        tidy_spaces(&mut out, at);
    }
    (
        out,
        Replaced {
            edits: edits.to_vec(),
            counts,
        },
    )
}

/// Where a match of `find` starting at byte `at` of `text` ends, if there
/// is one. Case is ignored, curly and straight apostrophes are the same,
/// and a space in `find` matches any run of spaces or line breaks.
fn match_at(text: &str, at: usize, find: &[char]) -> Option<usize> {
    let mut chars = text[at..].char_indices().peekable();
    let mut end = at;
    for &f in find {
        if f.is_whitespace() {
            let mut any = false;
            while let Some(&(i, c)) = chars.peek() {
                if !c.is_whitespace() {
                    break;
                }
                any = true;
                end = at + i + c.len_utf8();
                chars.next();
            }
            if !any {
                return None;
            }
            continue;
        }
        let (i, c) = chars.next()?;
        if !same(c, f) {
            return None;
        }
        end = at + i + c.len_utf8();
    }
    Some(end)
}

fn same(a: char, b: char) -> bool {
    let a = if a == '’' { '\'' } else { a };
    let b = if b == '’' { '\'' } else { b };
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// The match `text[start..end]` doesn't run on into a longer word.
fn whole(text: &str, start: usize, end: usize, find: &[char]) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    edge_ok(find.first().copied(), before) && edge_ok(find.last().copied(), after)
}

/// An edge of `find` needs a word boundary only when it's a letter or
/// digit, and only between letters of writing that puts spaces between
/// words.
fn edge_ok(edge: Option<char>, outside: Option<char>) -> bool {
    let (Some(edge), Some(outside)) = (edge, outside) else {
        return true;
    };
    !edge.is_alphanumeric() || !outside.is_alphanumeric() || unspaced(edge) || unspaced(outside)
}

/// Writing without spaces between words, so with no word edges to find:
/// Chinese, Japanese, Thai, Lao, Khmer, Myanmar and Tibetan. Korean too:
/// it has spaces, but particles join on to the word ("화요일에", "on
/// Tuesday"), so a whole-word match would miss most of them.
fn unspaced(c: char) -> bool {
    matches!(c,
        '\u{0E00}'..='\u{0EFF}' // Thai, Lao
        | '\u{0F00}'..='\u{0FFF}' // Tibetan
        | '\u{1000}'..='\u{109F}' // Myanmar
        | '\u{1100}'..='\u{11FF}' // Hangul jamo
        | '\u{1780}'..='\u{17FF}' // Khmer
        | '\u{3040}'..='\u{30FF}' // Hiragana, Katakana
        | '\u{3130}'..='\u{318F}' // Hangul compatibility jamo
        | '\u{31F0}'..='\u{31FF}' // Katakana extensions
        | '\u{3400}'..='\u{4DBF}' // Han, extension A
        | '\u{4E00}'..='\u{9FFF}' // Han
        | '\u{AC00}'..='\u{D7AF}' // Hangul syllables
        | '\u{F900}'..='\u{FAFF}' // Han compatibility
        | '\u{FF66}'..='\u{FF9F}' // halfwidth Katakana
        | '\u{20000}'..='\u{3FFFF}' // Han, further extensions
    )
}

/// `edit.replace` in the case the text had: "tuesday" → "thursday",
/// "Tuesday" → "Thursday", "TUESDAY" → "THURSDAY". A replacement with
/// capitals of its own inside it ("iPhone", "New York") goes in as given
/// unless the text was in capitals, and so does one that only changes the
/// case of what's found ("tuesday" → "Tuesday").
fn in_case_of(matched: &str, edit: &Edit) -> String {
    let replace = edit.replace.as_str();
    if edit.find.to_lowercase() == replace.to_lowercase() {
        return replace.to_string();
    }
    let plain = replace
        .chars()
        .filter(|c| c.is_alphabetic())
        .skip(1)
        .all(|c| !c.is_uppercase());
    match case_of(matched) {
        Case::Upper => replace.to_uppercase(),
        Case::Lower if plain => replace.to_lowercase(),
        Case::Title if plain => {
            let mut chars = replace.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
        _ => replace.to_string(),
    }
}

enum Case {
    Lower,
    Title,
    Upper,
    Other,
}

fn case_of(s: &str) -> Case {
    let letters: Vec<char> = s
        .chars()
        .filter(|c| c.is_uppercase() || c.is_lowercase())
        .collect();
    let Some((first, rest)) = letters.split_first() else {
        return Case::Other;
    };
    if letters.iter().all(|c| c.is_lowercase()) {
        Case::Lower
    } else if !rest.is_empty() && letters.iter().all(|c| c.is_uppercase()) {
        Case::Upper
    } else if first.is_uppercase() && rest.iter().all(|c| c.is_lowercase()) {
        Case::Title
    } else {
        Case::Other
    }
}

/// After something was taken out at byte `at`: the spaces either side of it
/// become one, or none at the start or end of a line or before punctuation
/// ("a very big" → "a big", "is very." → "is.").
fn tidy_spaces(text: &mut String, at: usize) {
    let gap = |c: char| c == ' ' || c == '\t';
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| gap(c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .take_while(|&(_, c)| gap(c))
        .last()
        .map_or(at, |(i, c)| at + i + c.len_utf8());
    if start == end {
        return;
    }
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    let line_edge = |c: Option<char>| c.is_none_or(|c| c == '\n' || c == '\r');
    let closing = after.is_some_and(|c| ".,;:!?)]}…”’»。，、！？；：".contains(c));
    let opening = before.is_some_and(|c| "([{“‘«¿¡".contains(c));
    // "indeed, very." → "indeed.": the comma went with the word.
    let start = match before {
        Some(c @ (',' | ';' | ':')) if closing => start - c.len_utf8(),
        _ => start,
    };
    let keep = if line_edge(before) || line_edge(after) || closing || opening {
        ""
    } else {
        " "
    };
    text.replace_range(start..end, keep);
}

/// Runs of spaces and line breaks made one space, and trimmed.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `s` without one pair of quotes around all of it.
fn unquote(s: &str) -> &str {
    for (open, close) in [('"', '"'), ('\'', '\''), ('“', '”'), ('‘', '’'), ('«', '»')] {
        if let Some(inner) = s.strip_prefix(open).and_then(|r| r.strip_suffix(close))
            && !inner.trim().is_empty()
        {
            return inner.trim();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, edits: &[(&str, &str)]) -> (String, Vec<usize>) {
        let edits: Vec<Edit> = edits.iter().map(|(f, r)| Edit::new(*f, *r)).collect();
        let (out, replaced) = apply(text, &clean(edits));
        (out, replaced.counts)
    }

    #[test]
    fn replaces_every_whole_word_ignoring_case() {
        let (out, counts) = run(
            "Tuesday works. tuesday, TUESDAY or Tuesdays? Not Tuesday's.",
            &[("tuesday", "thursday")],
        );
        assert_eq!(
            out,
            "Thursday works. thursday, THURSDAY or Tuesdays? Not Thursday's."
        );
        assert_eq!(counts, vec![4]);
    }

    #[test]
    fn keeps_the_case_of_what_it_replaced() {
        let (out, _) = run("smith, Smith, SMITH", &[("Smith", "Smythe")]);
        assert_eq!(out, "smythe, Smythe, SMYTHE");
        // A replacement with capitals inside goes in as given.
        let (out, _) = run("my phone and my Phone", &[("phone", "iPhone")]);
        assert_eq!(out, "my iPhone and my iPhone");
        let (out, _) = run("we met in boston", &[("Boston", "New York")]);
        assert_eq!(out, "we met in New York");
        // Changing only the case is what was asked for.
        let (out, counts) = run("ask github and GitHub", &[("github", "GitHub")]);
        assert_eq!(out, "ask GitHub and GitHub");
        assert_eq!(counts, vec![2]);
        // Mixed case in the text: as given.
        let (out, _) = run("the TueSday slot", &[("tuesday", "thursday")]);
        assert_eq!(out, "the thursday slot");
    }

    #[test]
    fn edits_are_made_in_one_pass() {
        // Swapping two words: the first edit's result isn't matched again.
        let (out, counts) = run("cats chase dogs", &[("cats", "dogs"), ("dogs", "cats")]);
        assert_eq!(out, "dogs chase cats");
        assert_eq!(counts, vec![1, 1]);
        // At the same place, the longer find wins.
        let (out, counts) = run(
            "New York and York",
            &[("York", "Leeds"), ("New York", "Boston")],
        );
        assert_eq!(out, "Boston and Leeds");
        assert_eq!(counts, vec![1, 1]);
    }

    #[test]
    fn removing_a_word_leaves_no_stray_spaces() {
        let (out, counts) = run(
            "It was very good. Very good indeed, very.\nvery nice",
            &[("very", "")],
        );
        assert_eq!(out, "It was good. good indeed.\nnice");
        assert_eq!(counts, vec![4]);
        let (out, _) = run("a very very big (very) deal", &[("very", "")]);
        assert_eq!(out, "a big () deal");
        let (out, _) = run("so  very   tired", &[("very", "")]);
        assert_eq!(out, "so tired");
        let (out, _) = run("very", &[("very", "")]);
        assert_eq!(out, "");
    }

    #[test]
    fn writing_without_spaces_still_matches() {
        // Chinese: 星期二 (Tuesday) inside a sentence.
        let (out, counts) = run("我们星期二见，星期二下午。", &[("星期二", "星期四")]);
        assert_eq!(out, "我们星期四见，星期四下午。");
        assert_eq!(counts, vec![2]);
        // Japanese, with a particle joined on.
        let (out, _) = run("火曜日に会いましょう", &[("火曜日", "木曜日")]);
        assert_eq!(out, "木曜日に会いましょう");
        // Thai.
        let (out, counts) = run("พบกันวันอังคารนะ", &[("วันอังคาร", "วันพฤหัสบดี")]);
        assert_eq!(out, "พบกันวันพฤหัสบดีนะ");
        assert_eq!(counts, vec![1]);
        // Korean, with a particle joined on.
        let (out, _) = run("화요일에 만나요", &[("화요일", "목요일")]);
        assert_eq!(out, "목요일에 만나요");
        // A Latin word right against Chinese.
        let (out, _) = run("我用iPhone拍的", &[("iphone", "Pixel")]);
        assert_eq!(out, "我用Pixel拍的");
        // Removing a word leaves no gap there either.
        let (out, _) = run("非常非常好", &[("非常", "")]);
        assert_eq!(out, "好");
    }

    #[test]
    fn other_alphabets_and_accents() {
        let (out, counts) = run("Встреча во вторник. ВТОРНИК!", &[("вторник", "четверг")]);
        assert_eq!(out, "Встреча во четверг. ЧЕТВЕРГ!");
        assert_eq!(counts, vec![2]);
        let (out, _) = run("Nos vemos el Miércoles", &[("miércoles", "jueves")]);
        assert_eq!(out, "Nos vemos el Jueves");
        // "é" is a letter: "café" isn't found in "cafés".
        let (out, counts) = run("cafés", &[("café", "bar")]);
        assert_eq!(out, "cafés");
        assert_eq!(counts, vec![0]);
    }

    #[test]
    fn phrases_symbols_and_apostrophes() {
        // A space matches a line break or several spaces.
        let (out, _) = run("next\nweek and Next  week", &[("next week", "the weekend")]);
        assert_eq!(out, "the weekend and The weekend");
        // No letter at the edge: no word boundary needed.
        let (out, counts) = run("salt&pepper & oil", &[("&", "and")]);
        assert_eq!(out, "saltandpepper and oil");
        assert_eq!(counts, vec![2]);
        let (out, _) = run("I don’t know", &[("don't", "do not")]);
        assert_eq!(out, "I do not know");
        // Quotes around what to find are taken off.
        let (out, counts) = run("a really good one", &[("'really'", "")]);
        assert_eq!(out, "a good one");
        assert_eq!(counts, vec![1]);
    }

    #[test]
    fn nothing_found_changes_nothing() {
        let (out, counts) = run("Monday it is", &[("Tuesday", "Thursday")]);
        assert_eq!(out, "Monday it is");
        assert_eq!(counts, vec![0]);
        assert_eq!(run("", &[("a", "b")]).1, vec![0]);
    }

    #[test]
    fn cleaning_drops_what_can_t_be_used() {
        let edits = clean(vec![
            Edit::new("  ", "x"),
            Edit::new("same", "same"),
            Edit::new(" next   week ", " Friday "),
            Edit::new("Next week", "Monday"),
        ]);
        assert_eq!(edits, vec![Edit::new("next week", "Friday")]);
    }

    #[test]
    fn says_what_changed() {
        let r = |edits: &[(&str, &str)], counts: &[usize]| Replaced {
            edits: edits.iter().map(|(f, t)| Edit::new(*f, *t)).collect(),
            counts: counts.to_vec(),
        };
        assert_eq!(
            r(&[("Tuesday", "Thursday")], &[4]).summary(),
            "Changed 4 · Tuesday → Thursday"
        );
        assert_eq!(r(&[("very", "")], &[3]).summary(), "Removed 3 · very");
        assert_eq!(
            r(
                &[("Tuesday", "Thursday"), ("Smith", "Smythe"), ("very", "")],
                &[2, 0, 3]
            )
            .summary(),
            "Changed 5 · Tuesday → Thursday, removed very"
        );
        assert_eq!(r(&[("x", "y")], &[0]).summary(), "Nothing changed");
    }
}
