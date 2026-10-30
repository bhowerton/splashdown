//! File mentions for coding agents (Claude Code, Cursor, …): a file name in
//! the text ("chunks.rs") becomes an `@path` mention, relative to the
//! project, when exactly one file in the project has that name. Done in
//! code after the AI pass, so it also works with the AI off. Speech models
//! write file names because the project's recent file names are among their
//! spelling hints; no spoken words ("dot") are matched here, so it works in
//! any language.

use std::collections::HashMap;

/// Rewrites file names in `text` as `@path` mentions. `files` are paths
/// relative to the project root, with `/` separators.
pub fn tag_files(text: &str, files: &[String]) -> String {
    if files.is_empty() {
        return text.to_string();
    }
    let index = Index::new(files);
    let mut out = String::with_capacity(text.len() + 16);
    for (i, token) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&index.tag(token));
    }
    out
}

struct Index<'a> {
    /// Lowercase file name → paths with that name.
    by_name: HashMap<String, Vec<&'a str>>,
    files: &'a [String],
}

impl<'a> Index<'a> {
    fn new(files: &'a [String]) -> Self {
        let mut by_name: HashMap<String, Vec<&str>> = HashMap::new();
        for path in files {
            let name = path.rsplit('/').next().unwrap_or(path);
            by_name.entry(name.to_lowercase()).or_default().push(path);
        }
        Self { by_name, files }
    }

    /// One whitespace-separated token: the file mention in it tagged, its
    /// surrounding punctuation kept.
    fn tag(&self, token: &str) -> String {
        const EDGE: &[char] = &[
            ',', '.', ';', ':', '!', '?', '(', ')', '"', '\'', '`', '“', '”', '‘', '’',
        ];
        let core = token.trim_matches(EDGE);
        if core.is_empty() || core.starts_with('@') || !core.contains('.') || core.contains("://") {
            return token.to_string();
        }
        let Some(path) = self.find(core) else {
            return token.to_string();
        };
        let start = token.find(core).unwrap_or(0);
        let (mut before, mut after) = (&token[..start], &token[start + core.len()..]);
        // A mention in backticks wouldn't work as one.
        if before.ends_with('`') && after.starts_with('`') {
            before = &before[..before.len() - 1];
            after = &after[1..];
        }
        format!("{before}@{path}{after}")
    }

    /// The one project file `name` refers to: by file name, or by the end
    /// of its path ("src/main.rs").
    fn find(&self, name: &str) -> Option<&'a str> {
        let lower = name.to_lowercase();
        if lower.contains('/') {
            let suffix = format!("/{}", lower.trim_start_matches('/'));
            let mut hits = self
                .files
                .iter()
                .filter(|p| p.to_lowercase() == lower || p.to_lowercase().ends_with(&suffix));
            let first = hits.next()?;
            return hits.next().is_none().then_some(first.as_str());
        }
        match self.by_name.get(&lower)?.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Vec<String> {
        [
            "app/src-tauri/src/chunks.rs",
            "app/src-tauri/src/session.rs",
            "app/src-tauri/tauri.conf.json",
            "crates/splashdown-cli/src/main.rs",
            "app/src-tauri/src/main.rs",
            "README.md",
        ]
        .map(String::from)
        .to_vec()
    }

    #[test]
    fn tags_unique_file_names() {
        let files = project();
        assert_eq!(
            tag_files("Look at chunks.rs and session.rs.", &files),
            "Look at @app/src-tauri/src/chunks.rs and @app/src-tauri/src/session.rs."
        );
        assert_eq!(
            tag_files("update the readme.md please", &files),
            "update the @README.md please"
        );
        // Backticks around a mention are dropped.
        assert_eq!(
            tag_files("Open `tauri.conf.json`", &files),
            "Open @app/src-tauri/tauri.conf.json"
        );
    }

    #[test]
    fn ambiguous_or_unknown_names_are_left_alone() {
        let files = project();
        assert_eq!(tag_files("fix main.rs", &files), "fix main.rs");
        // A path ending picks one.
        assert_eq!(
            tag_files("fix splashdown-cli/src/main.rs", &files),
            "fix @crates/splashdown-cli/src/main.rs"
        );
        assert_eq!(tag_files("see notes.txt", &files), "see notes.txt");
        assert_eq!(tag_files("see example.com", &files), "see example.com");
        assert_eq!(tag_files("@README.md", &files), "@README.md");
        assert_eq!(tag_files("anything", &[]), "anything");
    }
}
