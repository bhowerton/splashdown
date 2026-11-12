//! Finding the parts of notes that match a question (for Ask), by words
//! and, when it's on, by meaning, with short excerpts around each match.

use super::*;

/// Most notes a question gets excerpts from.
pub(super) const ASK_NOTES: usize = 3;

/// Characters of each note an answer may use.
pub(super) const ASK_EXCERPT: usize = 1200;

/// Excerpts of the notes that best match `question`, for the Ask panel, if
/// Settings allows it. Matching is by the question's words, weighted by how
/// rare each is across the notes, so "the", "der" or "le" count for little
/// in any language; and by meaning too when that's turned on.
pub fn excerpts_for(
    settings: &NotesSettings,
    question: &str,
) -> Vec<splashdown_core::ask::NoteExcerpt> {
    if !settings.ask_uses_notes() {
        return Vec::new();
    }
    // No notes folder yet: no notes to find, and none is made for it.
    let Some(store) = existing_store_for(settings.folder.clone()) else {
        return Vec::new();
    };
    matching(
        &store,
        question,
        ASK_NOTES,
        ASK_EXCERPT,
        settings.ask_by_meaning,
    )
    .into_iter()
    .map(|(_, excerpt)| excerpt)
    .collect()
}

/// The `limit` notes that best match `question`: each one's path, and up
/// to `chars` of it around its best word, or the part closest in meaning.
/// With `by_meaning`, notes are ranked by both, each note by its places in
/// the two rankings (reciprocal rank fusion), so a note high in either
/// comes up and one high in both comes first.
pub(super) fn matching(
    store: &Arc<Store>,
    question: &str,
    limit: usize,
    chars: usize,
    by_meaning: bool,
) -> Vec<(String, splashdown_core::ask::NoteExcerpt)> {
    /// Damps the difference between the top places (the usual 60).
    const K: f64 = 60.0;
    let notes = store.list();
    let mut ranked: Vec<(f64, &Summary, Option<String>, Option<meaning::Piece>)> = Vec::new();
    for (place, (note, word)) in best_matches(&notes, question, limit)
        .into_iter()
        .enumerate()
    {
        ranked.push((1.0 / (K + place as f64), note, Some(word), None));
    }
    let meant = if by_meaning {
        meaning::search(store, &notes, question, limit)
    } else {
        Vec::new()
    };
    for (place, hit) in meant.into_iter().enumerate() {
        let score = 1.0 / (K + place as f64);
        match ranked.iter_mut().find(|r| r.1.path == hit.path) {
            Some(r) => {
                r.0 += score;
                r.3 = Some(hit.piece);
            }
            None => {
                if let Some(note) = notes.iter().find(|n| n.path == hit.path) {
                    ranked.push((score, note, None, Some(hit.piece)));
                }
            }
        }
    }
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.modified.cmp(&a.1.modified)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, note, word, piece)| {
            let text = match (piece, word) {
                (Some(piece), _) => around_piece(&note.text, piece, chars),
                (None, Some(word)) => around(&note.text, &word, chars),
                (None, None) => clip(&note.text, chars, false),
            };
            (
                note.path.clone(),
                splashdown_core::ask::NoteExcerpt {
                    title: note.title.clone(),
                    date: day_of(note.modified),
                    text,
                },
            )
        })
        .collect()
}

/// Up to `max` characters of `text` with `piece` in them, starting a little
/// before it.
pub(super) fn around_piece(text: &str, piece: meaning::Piece, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let before = max.saturating_sub(piece.len) / 3;
    let start = piece.start.saturating_sub(before).min(chars.len() - max);
    let mut out: String = chars[start..start + max].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if start + max < chars.len() {
        out.push('…');
    }
    out
}

/// "2026-10-30", for a time in Unix ms.
pub(super) fn day_of(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

/// The first `max` characters of `text`, or the last ones when `recent`
/// (a meeting still going: what was said lately).
pub(super) fn clip(text: &str, max: usize, recent: bool) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    if recent {
        let rest: String = text.chars().skip(count - max).collect();
        format!("…{rest}")
    } else {
        let start: String = text.chars().take(max).collect();
        format!("{start}…")
    }
}

/// Words too common to tell notes apart, in the languages most used: with
/// only a few notes, how often a word appears across them can't show it.
pub(super) const COMMON_WORDS: &[&str] = &[
    // English
    "the", "and", "for", "are", "was", "were", "what", "when", "where", "which", "who", "whom",
    "why", "how", "that", "this", "these", "those", "with", "from", "about", "did", "does", "has",
    "have", "had", "can", "could", "would", "should", "will", "our", "your", "you", "they",
    "their", "them", "there", "then", "than", "but", "not", "any", "all", "its", "into", "out",
    "who's", "what's", "whats", "tell", "said", "say", // German
    "der", "die", "das", "und", "ist", "was", "wie", "wer", "wann", "wo", "warum", "welche",
    "haben", "hat", "wir", "ihr", "sie", "ein", "eine", "einen", "mit", "von", "für", "über",
    // French
    "les", "des", "est", "que", "qui", "quoi", "quand", "comment", "pour", "avec", "nous", "vous",
    "une", "dans", "sur", "pas", "quel", "quelle", // Spanish and Portuguese
    "los", "las", "que", "qué", "quién", "cuándo", "dónde", "cómo", "por", "para", "con", "una",
    "del", "nos", "fue", "son", "está", "uma", "não", "com", "quem", "quando", "onde", "como",
    // Italian and Dutch
    "che", "chi", "cosa", "quando", "dove", "come", "per", "con", "una", "het", "een", "wat", "wie",
    "wanneer", "waar", "hoe", "voor", "met", "van", "zijn",
];

/// Scripts written without spaces between words.
pub(super) fn unspaced(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF // Hiragana, Katakana
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF // CJK ideographs
    )
}

/// The words in `text` worth matching, lowercased: three letters or more
/// and not among the most common words; in Chinese and Japanese, which
/// have no spaces, each pair of characters.
pub(super) fn words_of(text: &str) -> std::collections::HashSet<String> {
    let mut words = std::collections::HashSet::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        let chars: Vec<char> = token.chars().collect();
        if chars.iter().any(|&c| unspaced(c)) {
            words.extend(chars.windows(2).map(|w| w.iter().collect::<String>()));
            continue;
        }
        let word = token.to_lowercase();
        if word.chars().count() >= 3 && !COMMON_WORDS.contains(&word.as_str()) {
            words.insert(word);
        }
    }
    words
}

/// The `limit` notes that match `question` best, each with its rarest
/// matching word (to excerpt around). A note matches on words found in its
/// title or text; a word in most of the notes counts for nothing.
pub(super) fn best_matches<'a>(
    notes: &'a [Summary],
    question: &str,
    limit: usize,
) -> Vec<(&'a Summary, String)> {
    let mut words: Vec<String> = words_of(question).into_iter().collect();
    words.sort();
    let texts: Vec<std::collections::HashSet<String>> = notes
        .iter()
        .map(|n| words_of(&format!("{}\n{}", n.title, n.text)))
        .collect();
    let total = notes.len() as f64;
    // How telling each word is: rare across the notes is telling, and a
    // word in most of a handful of notes or more is just a common word.
    let weights: Vec<f64> = words
        .iter()
        .map(|w| {
            let found = texts.iter().filter(|t| t.contains(w)).count() as f64;
            if found == 0.0 || (total >= 4.0 && found > total / 2.0) {
                0.0
            } else {
                (total / found).ln() + 1.0
            }
        })
        .collect();
    let mut scored: Vec<(f64, usize, Option<usize>)> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let mut score = 0.0;
            let mut rarest: Option<usize> = None;
            for (w, word) in words.iter().enumerate() {
                if weights[w] > 0.0 && text.contains(word) {
                    score += weights[w];
                    if rarest.is_none_or(|r| weights[w] > weights[r]) {
                        rarest = Some(w);
                    }
                }
            }
            (score, i, rarest)
        })
        .filter(|(score, ..)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(notes[b.1].modified.cmp(&notes[a.1].modified))
    });
    scored
        .into_iter()
        .take(limit)
        .filter_map(|(_, i, rarest)| Some((&notes[i], words[rarest?].clone())))
        .collect()
}

/// Up to `max` characters of `text` around the first `word` (any case).
pub(super) fn around(text: &str, word: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    // The word itself, not inside a longer one ("art", not "start").
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let lower = if lower.len() == chars.len() {
        lower
    } else {
        chars.clone()
    };
    let word: Vec<char> = word.chars().collect();
    let edge = |i: usize| {
        lower
            .get(i)
            .is_none_or(|c| !c.is_alphanumeric() || unspaced(*c))
    };
    let at = (0..lower.len().saturating_sub(word.len() - 1))
        .find(|&i| {
            lower[i..i + word.len()] == word[..]
                && (unspaced(word[0]) || (i == 0 || edge(i - 1)) && edge(i + word.len()))
        })
        .unwrap_or(0);
    let start = at.saturating_sub(max / 3).min(chars.len() - max);
    let mut out: String = chars[start..start + max].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if start + max < chars.len() {
        out.push('…');
    }
    out
}

/// About 240 characters of `text` around the first match.
pub(super) fn excerpt(text: &str, needle: &str) -> String {
    let lower = text.to_lowercase();
    let at = if needle.is_empty() {
        None
    } else {
        lower.find(needle)
    };
    let chars: Vec<char> = text.chars().collect();
    let start = at
        .map(|byte| lower[..byte].chars().count().saturating_sub(80))
        .unwrap_or(0)
        .min(chars.len());
    let end = (start + 240).min(chars.len());
    let mut out: String = chars[start..end].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if end < chars.len() {
        out.push('…');
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
