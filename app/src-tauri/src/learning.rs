//! Learning names and terms from corrections: a word the user fixes right
//! after a dictation ("Pria" → "Priya", see `splashdown_core::vocab`) goes
//! into the vocabulary at once and is marked as learned. It can be
//! forgotten from the menu bar or Settings, and a forgotten word is never
//! learned again.

use std::sync::Mutex;

use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// The words learned last, for the menu bar's Forget item.
static LAST: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Adds `words` to the vocabulary, leaving out ones already there, ones
/// forgotten before and ones that aren't names or terms. Saves, rebuilds
/// the engine, and returns what was added.
pub fn learn(app: &AppHandle, words: Vec<String>) -> Vec<String> {
    let Some(state) = app.try_state::<AppState>() else {
        return Vec::new();
    };
    let has = |list: &[String], w: &str| list.iter().any(|v| v.eq_ignore_ascii_case(w));
    let mut added: Vec<String> = Vec::new();
    let saved = state.update(app, |settings| {
        let general = &mut settings.general;
        for word in words {
            if worth_learning(&word)
                && !has(&general.vocabulary, &word)
                && !has(&general.forgotten_words, &word)
                && !has(&added, &word)
            {
                added.push(word);
            }
        }
        general.vocabulary.extend(added.iter().cloned());
        general.learned_words.extend(added.iter().cloned());
    });
    if let Err(e) = saved {
        tracing::warn!("couldn't save learned words: {e}");
        return Vec::new();
    }
    if added.is_empty() {
        return added;
    }
    crate::state::rebuild_in_background(app);
    tracing::info!("learned {} word(s) from corrections", added.len());
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = added.clone();
    crate::tray::show_forget(added.last().map(String::as_str));
    added
}

/// Removes a learned word from the vocabulary for good.
pub fn forget(app: &AppHandle, word: &str) -> Result<(), String> {
    let state = app
        .try_state::<AppState>()
        .ok_or("Splashdown isn't ready")?;
    state.update(app, |settings| {
        settings
            .general
            .vocabulary
            .retain(|v| !v.eq_ignore_ascii_case(word));
        settings.general.settle_learned();
    })?;
    crate::state::rebuild_in_background(app);
    tracing::info!("forgot a learned word");
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    last.retain(|w| !w.eq_ignore_ascii_case(word));
    crate::tray::show_forget(last.last().map(String::as_str));
    Ok(())
}

/// The menu bar's Forget item: the word learned last.
pub fn forget_last(app: &AppHandle) {
    let word = LAST
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .last()
        .cloned();
    if let Some(word) = word
        && let Err(e) = forget(app, &word)
    {
        tracing::warn!("couldn't forget a learned word: {e}");
    }
}

/// Names and terms only: not addresses, paths, emails or numbers.
fn worth_learning(word: &str) -> bool {
    let len = word.chars().count();
    (3..=40).contains(&len)
        && word.chars().filter(|c| c.is_alphabetic()).count() * 2 > len
        && !word.contains(['@', '/', '\\', ':'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_names_and_terms_only() {
        for word in ["Priya", "OpenRouter", "k8s", "O'Connell", "Jean-Luc"] {
            assert!(worth_learning(word), "{word}");
        }
        for word in [
            "ab",
            "A1234",
            "ben@example.com",
            "src/main",
            "C:\\Users",
            "2026",
        ] {
            assert!(!worth_learning(word), "{word}");
        }
    }
}
