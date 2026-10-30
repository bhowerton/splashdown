//! Learning vocabulary from corrections: after an insertion, if the user
//! retypes an inserted word into something close that looks like a name or
//! term ("Pria" → "Priya"), suggest adding it to the vocabulary.

/// Suggests words from `field_now` that look like corrections of words in
/// `inserted`. `known` is the current vocabulary (never re-suggested).
pub fn suggest(inserted: &str, field_now: &str, known: &[String]) -> Vec<String> {
    let inserted_words = words(inserted);
    let field_words = words(field_now);
    let is_known = |w: &str| known.iter().any(|k| k.eq_ignore_ascii_case(w));

    let mut out: Vec<String> = Vec::new();
    for original in &inserted_words {
        // Still there as dictated: not corrected.
        if original.chars().count() < 3 || field_words.contains(original) {
            continue;
        }
        let best = field_words
            .iter()
            .filter(|w| !inserted_words.contains(w) && looks_like_term(w) && !is_known(w))
            .filter(|w| !out.contains(w))
            .map(|w| (distance(&original.to_lowercase(), &w.to_lowercase()), w))
            // Distance 0 here means only the capitalization changed, which
            // still counts ("tauri" → "Tauri"); `w` differs from `original`.
            .filter(|(d, w)| *d <= max_distance(original, w))
            .min_by_key(|(d, _)| *d);
        if let Some((_, w)) = best {
            out.push(w.clone());
        }
    }
    out
}

/// Names, product terms and jargon: capitalized, or containing a digit or
/// an inner capital. Ordinary lowercase typo fixes aren't suggested.
fn looks_like_term(w: &str) -> bool {
    w.chars().count() >= 3
        && (w.chars().next().is_some_and(char::is_uppercase)
            || w.chars().any(|c| c.is_ascii_digit())
            || w.chars().skip(1).any(char::is_uppercase))
}

fn max_distance(a: &str, b: &str) -> usize {
    (a.chars().count().max(b.chars().count()) / 3).clamp(1, 3)
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
        .map(|w| w.trim_matches(|c: char| c == '\'' || c == '-'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Levenshtein distance over characters.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_corrected_names() {
        assert_eq!(
            suggest(
                "Please send it to Pria and Marcus.",
                "Please send it to Priya and Marcus.",
                &[]
            ),
            vec!["Priya"]
        );
        assert_eq!(
            suggest("Deploy the tauri app.", "Deploy the Tauri app.", &[]),
            vec!["Tauri"]
        );
    }

    #[test]
    fn ignores_ordinary_edits_and_known_words() {
        // Lowercase typo fixes aren't terms.
        assert!(suggest("I think their right", "I think they're right", &[]).is_empty());
        // Unchanged text.
        assert!(suggest("Hello Priya", "Hello Priya, more typing", &[]).is_empty());
        // Already in the vocabulary.
        assert!(suggest("to Pria", "to Priya", &["priya".into()]).is_empty());
        // Unrelated rewrite.
        assert!(suggest("meet on Monday", "meet on Thursday", &[]).is_empty());
    }

    #[test]
    fn edit_distance() {
        assert_eq!(distance("pria", "priya"), 1);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
    }
}
