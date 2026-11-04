//! Learning vocabulary from corrections: after an insertion, if the user
//! retypes an inserted word into something close that looks like a name or
//! term ("Pria" → "Priya"), it's learned into the vocabulary (the app
//! watches the field and saves it: `learning.rs`). Also the names and
//! terms on screen, which join the vocabulary for one dictation.

/// Whether most of `inserted` is still in `field_now`: false once the
/// field was sent, cleared or replaced, when only an earlier reading can
/// show a correction.
pub fn still_there(inserted: &str, field_now: &str) -> bool {
    let field: std::collections::HashSet<String> =
        words(field_now).iter().map(|w| w.to_lowercase()).collect();
    let long: Vec<String> = words(inserted)
        .into_iter()
        .filter(|w| w.chars().count() >= 3)
        .map(|w| w.to_lowercase())
        .collect();
    let kept = long.iter().filter(|w| field.contains(*w)).count();
    long.is_empty() || kept * 2 >= long.len()
}

/// The words in `field_now` that look like corrections of words in
/// `inserted`. Words already in `known` (the vocabulary) are left out.
pub fn corrections(inserted: &str, field_now: &str, known: &[String]) -> Vec<String> {
    let inserted_words = words(inserted);
    let field_words = words(field_now);
    let is_known = |w: &str| known.iter().any(|k| k.eq_ignore_ascii_case(w));

    let mut out: Vec<String> = Vec::new();
    for original in &inserted_words {
        // Still there as dictated: not corrected.
        if original.chars().count() < 3 || field_words.contains(original) {
            continue;
        }
        let lower = original.to_lowercase();
        let best = field_words
            .iter()
            .filter(|w| !inserted_words.contains(w) && looks_like_term(w) && !is_known(w))
            // Partway through deleting it ("Pri"), not a correction.
            .filter(|w| {
                let w = w.to_lowercase();
                !(lower.len() > w.len() && lower.starts_with(&w))
            })
            .filter(|w| !out.contains(w))
            // Spelled close, or sounding the same ("trucks" → "TRUX"): how
            // speech recognition gets a name wrong. The closer, the better.
            .filter_map(|w| {
                let spelled = distance(&lower, &w.to_lowercase());
                let sounded = distance(&sound(&lower), &sound(w));
                // Distance 0 here means only the capitalization changed,
                // which still counts ("tauri" → "Tauri"); `w` differs from
                // `original`.
                (spelled <= max_distance(original, w) || sounded == 0)
                    .then_some((spelled.min(sounded), w))
            })
            .min_by_key(|(d, _)| *d);
        if let Some((_, w)) = best {
            out.push(w.clone());
        }
    }
    out
}

/// Names, product terms and jargon: capitalized, or containing a digit or
/// an inner capital. Ordinary lowercase typo fixes aren't learned.
fn looks_like_term(w: &str) -> bool {
    w.chars().count() >= 3
        && (w.chars().next().is_some_and(char::is_uppercase)
            || w.chars().any(|c| c.is_ascii_digit())
            || w.chars().skip(1).any(char::is_uppercase))
}

/// Names and terms in text read from the screen (the email or chat around
/// where the user is dictating), for speech-to-text and the AI to spell
/// right: words that look like terms (see `looks_like_term`), leaving out
/// common words that are only capitalized because they start a sentence or
/// a line. Most frequent first, then nearest the end (the field); at most
/// `max`.
pub fn terms_on_screen(text: &str, max: usize) -> Vec<String> {
    let all = words(text);
    let lowercase: std::collections::HashSet<&str> = all
        .iter()
        .filter(|w| w.chars().next().is_some_and(char::is_lowercase))
        .map(String::as_str)
        .collect();
    // (term, count, last position)
    let mut found: Vec<(String, usize, usize)> = Vec::new();
    for (i, w) in all.iter().enumerate() {
        if !looks_like_term(w)
            || w.chars().all(|c| c.is_ascii_digit() || c == '-')
            || COMMON.contains(&w.to_lowercase().as_str())
            || (w.chars().next().is_some_and(char::is_uppercase)
                && lowercase.contains(w.to_lowercase().as_str()))
            || (w.chars().count() > 1 && w.chars().all(char::is_uppercase) && w.chars().count() < 3)
        {
            continue;
        }
        match found.iter_mut().find(|(t, ..)| t == w) {
            Some(entry) => {
                entry.1 += 1;
                entry.2 = i;
            }
            None => found.push((w.clone(), 1, i)),
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    found.into_iter().take(max).map(|(t, ..)| t).collect()
}

/// Words capitalized on screen only because of where they are (the start
/// of a sentence, a line, a button or a header): never terms.
const COMMON: &[&str] = &[
    "the",
    "this",
    "that",
    "these",
    "those",
    "there",
    "then",
    "than",
    "and",
    "but",
    "for",
    "not",
    "you",
    "your",
    "yours",
    "our",
    "ours",
    "their",
    "they",
    "them",
    "she",
    "her",
    "his",
    "him",
    "its",
    "can",
    "could",
    "would",
    "should",
    "will",
    "shall",
    "may",
    "might",
    "must",
    "just",
    "also",
    "please",
    "thanks",
    "thank",
    "hey",
    "hello",
    "dear",
    "best",
    "regards",
    "cheers",
    "sincerely",
    "yes",
    "yeah",
    "okay",
    "sure",
    "great",
    "good",
    "nice",
    "cool",
    "sounds",
    "let",
    "lets",
    "let's",
    "what",
    "when",
    "where",
    "which",
    "who",
    "why",
    "how",
    "here",
    "have",
    "has",
    "had",
    "are",
    "was",
    "were",
    "been",
    "being",
    "did",
    "does",
    "doing",
    "with",
    "from",
    "into",
    "onto",
    "about",
    "after",
    "before",
    "today",
    "tomorrow",
    "yesterday",
    "morning",
    "afternoon",
    "evening",
    "tonight",
    "subject",
    "reply",
    "forward",
    "sent",
    "inbox",
    "drafts",
    "draft",
    "archive",
    "trash",
    "spam",
    "starred",
    "flagged",
    "unread",
    "search",
    "compose",
    "new",
    "more",
    "less",
    "all",
    "any",
    "some",
    "one",
    "two",
    "see",
    "view",
    "show",
    "hide",
    "open",
    "close",
    "edit",
    "delete",
    "send",
    "save",
    "cancel",
    "done",
    "share",
    "copy",
    "paste",
    "undo",
    "message",
    "messages",
    "threads",
    "thread",
    "people",
    "channels",
    "channel",
    "home",
    "activity",
    "files",
    "later",
    "now",
    "wrote",
    "said",
    "via",
    "sir",
    "madam",
    "mr",
    "mrs",
    "ms",
    "i'm",
    "i've",
    "i'll",
    "i'd",
    "it's",
    "that's",
    "don't",
    "can't",
    "won't",
    "didn't",
    "we're",
    "we'll",
    "you're",
    "you'll",
];

/// A rough key for how an English word sounds, so spellings of the same
/// sound match: "trucks" and "TRUX" are both "truks", "Kaitlyn" and
/// "Caitlin" both "katlin". Letters that are often silent or doubled are
/// folded together; vowels stay, so different words stay apart.
fn sound(word: &str) -> String {
    let w: Vec<char> = word.to_lowercase().chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < w.len() {
        let next = w.get(i + 1).copied();
        let (piece, skip): (&str, usize) = match (w[i], next) {
            ('p', Some('h')) => ("f", 2),
            ('c', Some('k')) => ("k", 2),
            ('q', Some('u')) => ("kw", 2),
            ('c', Some('e' | 'i' | 'y')) => ("s", 1),
            ('c' | 'q', _) => ("k", 1),
            ('x', _) => ("ks", 1),
            ('z', _) => ("s", 1),
            ('y', _) => ("i", 1),
            // Silent or near-silent after another letter: "Sarah", "Kaitlyn".
            ('h', _) if i > 0 => ("", 1),
            _ => ("", 0),
        };
        if skip == 0 {
            out.push(w[i]);
            i += 1;
        } else {
            out.push_str(piece);
            i += skip;
        }
    }
    // Doubled letters and vowel pairs sound like one: "Ann" "An", "ai" "a".
    let mut folded = String::new();
    for c in out.chars() {
        let vowel = |c: char| "aeiou".contains(c);
        match folded.chars().last() {
            Some(last) if last == c || (vowel(last) && vowel(c)) => {}
            _ => folded.push(c),
        }
    }
    folded
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
    fn terms_on_screen_are_names_not_sentence_starts() {
        let thread = "#launch\nPriya Shah 10:02 AM\nMorning! Standup moved to 10:30 today.\n\
                      Jordan Lee 10:15 AM\nCan someone review the pricing page in the Launch doc? \
                      Thanks Priya. Ping me on OpenRouter or the k8s channel.\nPeople\nThreads";
        let terms = terms_on_screen(thread, 10);
        assert_eq!(
            terms[0], "Priya",
            "the most mentioned comes first: {terms:?}"
        );
        for want in ["Shah", "Jordan", "Lee", "OpenRouter", "k8s", "Standup"] {
            assert!(
                terms.contains(&want.to_string()),
                "{want} missing: {terms:?}"
            );
        }
        // "Launch" is also written "#launch": an ordinary word.
        for not in [
            "Morning", "Can", "Thanks", "People", "Threads", "AM", "Launch",
        ] {
            assert!(!terms.contains(&not.to_string()), "{not} kept: {terms:?}");
        }
        assert_eq!(terms_on_screen(thread, 2).len(), 2);
    }

    #[test]
    fn a_capitalized_word_also_used_lowercase_is_not_a_term() {
        assert!(
            terms_on_screen("Movers come Thursday. The movers are early.", 5)
                .iter()
                .all(|t| t != "Movers")
        );
    }

    #[test]
    fn a_sent_or_cleared_field_no_longer_has_the_dictation() {
        let inserted = "Thanks Pria, I'll look at it before lunch.";
        assert!(still_there(
            inserted,
            "Thanks Priya, I'll look at it before lunch."
        ));
        assert!(!still_there(inserted, ""));
        assert!(!still_there(inserted, "Something else entirely now"));
    }

    #[test]
    fn a_correction_that_sounds_the_same_is_learned() {
        assert_eq!(
            corrections(
                "I parked at trucks parking.",
                "I parked at TRUX parking.",
                &[]
            ),
            vec!["TRUX"]
        );
        assert_eq!(
            corrections("Ask Caitlin about it.", "Ask Kaitlyn about it.", &[]),
            vec!["Kaitlyn"]
        );
        assert_eq!(
            corrections("Send it to Jon.", "Send it to John.", &[]),
            vec!["John"]
        );
        assert_eq!(sound("trucks"), sound("TRUX"));
        // Sounding different stays different.
        assert!(corrections("meet at the docks", "meet at the Dexter", &[]).is_empty());
        assert!(corrections("call Mark later", "call Marcy later", &[]).is_empty());
    }

    #[test]
    fn a_word_partway_through_deletion_is_not_a_correction() {
        assert!(corrections("Thanks Pria, see you", "Thanks Pri", &[]).is_empty());
    }

    #[test]
    fn finds_corrected_names() {
        assert_eq!(
            corrections(
                "Please send it to Pria and Marcus.",
                "Please send it to Priya and Marcus.",
                &[]
            ),
            vec!["Priya"]
        );
        assert_eq!(
            corrections("Deploy the tauri app.", "Deploy the Tauri app.", &[]),
            vec!["Tauri"]
        );
    }

    #[test]
    fn ignores_ordinary_edits_and_known_words() {
        // Lowercase typo fixes aren't terms.
        assert!(corrections("I think their right", "I think they're right", &[]).is_empty());
        // Unchanged text.
        assert!(corrections("Hello Priya", "Hello Priya, more typing", &[]).is_empty());
        // Already in the vocabulary.
        assert!(corrections("to Pria", "to Priya", &["priya".into()]).is_empty());
        // Unrelated rewrite.
        assert!(corrections("meet on Monday", "meet on Thursday", &[]).is_empty());
    }

    #[test]
    fn edit_distance() {
        assert_eq!(distance("pria", "priya"), 1);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
    }
}
