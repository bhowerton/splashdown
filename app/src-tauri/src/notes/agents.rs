//! Coding agents' access to notes (through `splashdown-mcp`): search, read
//! and append, each only as far as Settings → Privacy allows.

use super::*;

/// Asked for while Settings doesn't allow it.
pub(super) const AGENTS_OFF: &str = "Splashdown's notes are off for coding agents. To allow it, turn on “Let coding agents read notes” in Splashdown → Settings → Privacy.";

pub(super) const AGENTS_NO_WRITE: &str = "Coding agents may read Splashdown's notes but not add to them. To allow it, turn on “…and add to them” in Splashdown → Settings → Privacy.";

/// Most notes one search returns.
pub(super) const SEARCH_LIMIT: usize = 50;

/// `search_notes`: notes whose title or text contains `query` (any case),
/// titles first, then newest, as JSON with a short excerpt each.
pub fn agent_search(
    settings: &NotesSettings,
    query: &str,
    limit: Option<usize>,
) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    let (store, _) = store_for(settings.folder.clone())?;
    let needle = query.trim().to_lowercase();
    let limit = limit.unwrap_or(10).clamp(1, SEARCH_LIMIT);
    let mut hits: Vec<(bool, Summary)> = store
        .list()
        .into_iter()
        .filter_map(|n| {
            let in_title = n.title.to_lowercase().contains(&needle);
            (needle.is_empty() || in_title || n.text.to_lowercase().contains(&needle))
                .then_some((in_title, n))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.modified.cmp(&a.1.modified)));
    let results: Vec<serde_json::Value> = hits
        .into_iter()
        .take(limit)
        .map(|(_, n)| {
            serde_json::json!({
                "path": n.path,
                "title": n.title,
                "modified": chrono::DateTime::from_timestamp_millis(n.modified as i64)
                    .map(|t| t.with_timezone(&chrono::Local).to_rfc3339()),
                "excerpt": excerpt(&n.text, &needle),
            })
        })
        .collect();
    serde_json::to_string(&results).map_err(|e| e.to_string())
}

/// `read_note`: a note's text as JSON.
pub fn agent_read(settings: &NotesSettings, path: &str) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    let (store, _) = store_for(settings.folder.clone())?;
    let note = store
        .read(path)
        .map_err(|e| format!("Couldn't read {path}: {e}"))?;
    // The reply is escaped twice on its way to the agent and must fit the
    // control channel's line limit, so a long note (an hour's transcript)
    // is cut short, and says so.
    let text: String = note.body.chars().take(AGENT_READ_CHARS).collect();
    let truncated = text.len() < note.body.len();
    serde_json::to_string(&serde_json::json!({
        "path": note.path,
        "title": markdown::title_of(&note.body),
        "text": text,
        "truncated": truncated,
    }))
    .map_err(|e| e.to_string())
}

/// The most of a note `read_note` returns, in characters.
pub(super) const AGENT_READ_CHARS: usize = 50_000;

/// `append_note`: adds `text` to the capture note, or the note at `to`.
pub fn agent_append(
    settings: &NotesSettings,
    text: &str,
    to: Option<&str>,
) -> Result<String, String> {
    if !settings.agents_read {
        return Err(AGENTS_OFF.into());
    }
    if !settings.agents_write {
        return Err(AGENTS_NO_WRITE.into());
    }
    if text.trim().is_empty() {
        return Err("There's no text to add.".into());
    }
    let captured = capture(settings, to, text, None)?;
    Ok(format!(
        "Added to {} ({})",
        captured.label, captured.appended.path
    ))
}
