//! Vocabulary, corrections and snippets from other dictation apps, so
//! nothing built up there is lost when switching. The kind of file is told
//! from its structure:
//!
//! - **A dictionary export:** a JSON object with `vocabulary` (`{ "term" }`)
//!   and `replacements` (`{ "sources": [...], "replacement" }`).
//! - **A settings file with `custom_words`** (at the top, or under
//!   `settings`): those words are vocabulary.
//! - **Snippets as JSON:** an array of `{ "name", "text" }`.
//! - **Word lists:** a JSON array of strings, or text/CSV with a word per
//!   line; a second column makes a line a correction (the misspelling,
//!   then the right spelling).

use std::path::Path;

use serde::Deserialize;
use splashdown_core::spoken::Snippet;

use crate::{Replacement, Settings};

/// What a file had in it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Imported {
    /// What kind of file it was, for messages: "dictionary export".
    pub kind: &'static str,
    pub vocabulary: Vec<String>,
    pub replacements: Vec<Replacement>,
    pub snippets: Vec<Snippet>,
    /// Lines or entries that couldn't be used.
    pub skipped: usize,
}

/// What merging added.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Merged {
    pub vocabulary: usize,
    pub replacements: usize,
    pub snippets: usize,
    /// Already there (any case).
    pub duplicates: usize,
    /// A correction or snippet for the same words that says something
    /// different: the one already there is kept.
    pub conflicts: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("couldn't read it: {0}")]
    Read(#[from] std::io::Error),
    #[error("it has nothing Splashdown can import")]
    Empty,
}

/// Snippet triggers and texts longer than this are skipped.
const MAX_TRIGGER: usize = 60;
const MAX_SNIPPET: usize = 4000;

pub fn read(path: &Path) -> Result<Imported, ImportError> {
    let bytes = std::fs::read(path)?;
    // Spreadsheets often save CSV as UTF-8 with a byte-order mark.
    let text = String::from_utf8_lossy(&bytes);
    parse(text.trim_start_matches('\u{feff}'))
}

pub fn parse(text: &str) -> Result<Imported, ImportError> {
    let imported = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(json) => from_json(json),
        Err(_) => from_lines(text),
    };
    if imported.vocabulary.is_empty()
        && imported.replacements.is_empty()
        && imported.snippets.is_empty()
    {
        return Err(ImportError::Empty);
    }
    Ok(imported)
}

fn from_json(json: serde_json::Value) -> Imported {
    #[derive(Deserialize)]
    struct Dictionary {
        #[serde(default)]
        vocabulary: Vec<Term>,
        #[serde(default)]
        replacements: Vec<DictionaryReplacement>,
    }
    /// `{ "term": "Tauri" }`, or just `"Tauri"`.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Term {
        Object { term: String },
        Plain(String),
    }
    #[derive(Deserialize)]
    struct DictionaryReplacement {
        sources: Vec<String>,
        replacement: String,
    }

    let mut out = Imported::default();
    match &json {
        serde_json::Value::Object(map)
            if map.get("vocabulary").is_some_and(|v| v.is_array())
                || map.get("replacements").is_some_and(|r| r.is_array()) =>
        {
            out.kind = "dictionary export";
            if let Ok(file) = serde_json::from_value::<Dictionary>(json) {
                out.vocabulary = file
                    .vocabulary
                    .into_iter()
                    .map(|t| match t {
                        Term::Object { term } | Term::Plain(term) => term,
                    })
                    .collect();
                for r in file.replacements {
                    for find in r.sources {
                        out.replacements.push(Replacement {
                            find,
                            replace: r.replacement.clone(),
                            regex: false,
                        });
                    }
                }
            }
        }
        serde_json::Value::Object(map) => {
            // Some apps keep their settings under "settings".
            let words = map
                .get("settings")
                .and_then(|s| s.get("custom_words"))
                .or_else(|| map.get("custom_words"))
                .and_then(|w| w.as_array());
            if let Some(words) = words {
                out.kind = "custom words";
                out.vocabulary = strings(words, &mut out.skipped);
            }
        }
        serde_json::Value::Array(items) if items.iter().all(|i| i.is_string()) => {
            out.kind = "word list";
            out.vocabulary = strings(items, &mut out.skipped);
        }
        serde_json::Value::Array(items) => {
            out.kind = "snippets";
            for item in items {
                let field = |name| item.get(name).and_then(|v| v.as_str()).map(str::trim);
                match (field("name"), field("text")) {
                    (Some(name), Some(text))
                        if !name.is_empty()
                            && !text.is_empty()
                            && name.chars().count() <= MAX_TRIGGER
                            && text.chars().count() <= MAX_SNIPPET =>
                    {
                        out.snippets.push(Snippet {
                            trigger: name.to_string(),
                            text: text.to_string(),
                        });
                    }
                    _ => out.skipped += 1,
                }
            }
        }
        _ => {}
    }
    out.vocabulary.retain(|w| !w.trim().is_empty());
    out
}

fn strings(items: &[serde_json::Value], skipped: &mut usize) -> Vec<String> {
    items
        .iter()
        .filter_map(|v| {
            let word = v.as_str().map(str::trim).filter(|w| !w.is_empty());
            if word.is_none() {
                *skipped += 1;
            }
            word.map(str::to_string)
        })
        .collect()
}

/// A word per line, or `misspelling,correction`. A header row ("word",
/// "from,to") is skipped.
fn from_lines(text: &str) -> Imported {
    let mut out = Imported {
        kind: "word list",
        ..Imported::default()
    };
    for (i, line) in text.lines().enumerate() {
        let Some(fields) = csv_fields(line) else {
            out.skipped += 1;
            continue;
        };
        let fields: Vec<&str> = fields.iter().map(|f| f.trim()).collect();
        if i == 0 && is_header(&fields) {
            continue;
        }
        match fields.as_slice() {
            [] | [""] => {}
            [word] | [word, ""] => out.vocabulary.push(word.to_string()),
            [find, replace] if !find.is_empty() => out.replacements.push(Replacement {
                find: find.to_string(),
                replace: replace.to_string(),
                regex: false,
            }),
            _ => out.skipped += 1,
        }
    }
    out
}

fn is_header(fields: &[&str]) -> bool {
    const NAMES: [&str; 12] = [
        "word",
        "words",
        "term",
        "terms",
        "vocabulary",
        "phrase",
        "from",
        "to",
        "misspelling",
        "correction",
        "find",
        "replace",
    ];
    !fields.is_empty()
        && fields
            .iter()
            .all(|f| NAMES.contains(&f.to_lowercase().as_str()))
}

/// One CSV line's fields (RFC 4180 quoting: `"a, b"` and `""` for a
/// quote). `None` for an unterminated quote.
fn csv_fields(line: &str) -> Option<Vec<String>> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if field.trim().is_empty() => {
                field.clear();
                quoted = true;
            }
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    if quoted {
        return None;
    }
    fields.push(field);
    Some(fields)
}

/// Adds what's new to `settings`, keeping everything already there.
pub fn merge(settings: &mut Settings, imported: &Imported) -> Merged {
    let mut merged = Merged::default();
    let vocabulary = &mut settings.general.vocabulary;
    for word in &imported.vocabulary {
        if vocabulary.iter().any(|w| w.eq_ignore_ascii_case(word)) {
            merged.duplicates += 1;
        } else {
            vocabulary.push(word.clone());
            merged.vocabulary += 1;
        }
    }
    for rule in &imported.replacements {
        match settings
            .replacements
            .iter()
            .find(|r| !r.regex && r.find.eq_ignore_ascii_case(&rule.find))
        {
            Some(r) if r.replace == rule.replace => merged.duplicates += 1,
            Some(_) => merged.conflicts += 1,
            None => {
                settings.replacements.push(rule.clone());
                merged.replacements += 1;
            }
        }
    }
    for snippet in &imported.snippets {
        match settings
            .snippets
            .iter()
            .find(|s| s.trigger.eq_ignore_ascii_case(&snippet.trigger))
        {
            Some(s) if s.text == snippet.text => merged.duplicates += 1,
            Some(_) => merged.conflicts += 1,
            None => {
                settings.snippets.push(snippet.clone());
                merged.snippets += 1;
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_dictionary_export() {
        let file = r#"{
          "appVersion" : "2.20",
          "exportedAt" : "2026-10-01T09:00:00Z",
          "format" : "dictionary",
          "replacements" : [
            { "createdAt" : "2026-09-01T09:00:00Z", "replacement" : "Kubernetes", "sources" : [ "cooper netties", "kuber net ease" ] }
          ],
          "schemaVersion" : 1,
          "vocabulary" : [
            { "createdAt" : "2026-09-01T09:00:00Z", "term" : "Tauri" },
            { "term" : "OpenRouter" }
          ]
        }"#;
        let got = parse(file).unwrap();
        assert_eq!(got.kind, "dictionary export");
        assert_eq!(got.vocabulary, ["Tauri", "OpenRouter"]);
        assert_eq!(got.replacements.len(), 2);
        assert_eq!(got.replacements[1].find, "kuber net ease");
        assert_eq!(got.replacements[1].replace, "Kubernetes");
        // Plain strings work too.
        let plain = parse(r#"{"vocabulary": ["Tauri", "Anselm"]}"#).unwrap();
        assert_eq!(plain.vocabulary, ["Tauri", "Anselm"]);
    }

    #[test]
    fn reads_custom_words_from_a_settings_file() {
        let file =
            r#"{"settings": {"custom_words": ["Tauri", "Anselm", " "], "push_to_talk": true}}"#;
        let got = parse(file).unwrap();
        assert_eq!(got.kind, "custom words");
        assert_eq!(got.vocabulary, ["Tauri", "Anselm"]);
    }

    #[test]
    fn reads_snippets() {
        let file = r#"[
          { "name": "my sig", "text": "Best,\nAlex", "id": 3 },
          { "name": "", "text": "no trigger" },
          { "name": "addr", "text": "12 Main St" }
        ]"#;
        let got = parse(file).unwrap();
        assert_eq!(got.kind, "snippets");
        assert_eq!(got.snippets.len(), 2);
        assert_eq!(got.snippets[0].text, "Best,\nAlex");
        assert_eq!(got.skipped, 1);
    }

    #[test]
    fn reads_word_lists_and_corrections() {
        let file = "word\nTauri\n\"Smith, Jones & Co\"\nopen router,OpenRouter\n\"cooper netties\",\"Kubernetes\"\na,b,c\n\"unterminated\n";
        let got = parse(file).unwrap();
        assert_eq!(got.vocabulary, ["Tauri", "Smith, Jones & Co"]);
        assert_eq!(got.replacements.len(), 2);
        assert_eq!(got.replacements[0].find, "open router");
        assert_eq!(got.replacements[1].replace, "Kubernetes");
        // Three columns, and the unterminated quote.
        assert_eq!(got.skipped, 2);
        assert!(matches!(parse("\n\n"), Err(ImportError::Empty)));
    }

    #[test]
    fn merging_keeps_whats_there() {
        let mut settings = Settings::default();
        settings.general.vocabulary = vec!["tauri".into()];
        settings.replacements = vec![Replacement {
            find: "cooper netties".into(),
            replace: "Kubernetes".into(),
            regex: false,
        }];
        settings.snippets = vec![Snippet {
            trigger: "my sig".into(),
            text: "Cheers".into(),
        }];
        let imported = Imported {
            kind: "test",
            vocabulary: vec!["Tauri".into(), "OpenRouter".into()],
            replacements: vec![
                Replacement {
                    find: "Cooper Netties".into(),
                    replace: "Kubernetes".into(),
                    regex: false,
                },
                Replacement {
                    find: "jay son".into(),
                    replace: "JSON".into(),
                    regex: false,
                },
            ],
            snippets: vec![Snippet {
                trigger: "My sig".into(),
                text: "Best, Alex".into(),
            }],
            skipped: 0,
        };
        let merged = merge(&mut settings, &imported);
        assert_eq!(
            merged,
            Merged {
                vocabulary: 1,
                replacements: 1,
                snippets: 0,
                duplicates: 2,
                conflicts: 1,
            }
        );
        assert_eq!(settings.general.vocabulary, ["tauri", "OpenRouter"]);
        // The user's own snippet is kept.
        assert_eq!(settings.snippets[0].text, "Cheers");
        // Merging again adds nothing.
        assert_eq!(merge(&mut settings, &imported).vocabulary, 0);
    }
}
