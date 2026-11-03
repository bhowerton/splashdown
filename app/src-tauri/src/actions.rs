//! Actions shared by the menu bar and global shortcuts.

use std::sync::Arc;

use splashdown_platform::InsertMethod;

use crate::history::History;

/// Types the last raw transcript (exactly what was said, before the AI
/// pass) into the focused field: the one-step fix for a bad AI edit.
pub fn paste_last_raw(history: &Arc<History>) {
    let history = history.clone();
    // Pasting waits briefly for the clipboard; keep it off the UI thread.
    std::thread::spawn(move || {
        let Some(last) = history.last() else {
            return;
        };
        let raw = if last.raw.is_empty() {
            last.text
        } else {
            last.raw
        };
        if let Err(e) = splashdown_platform::insert_text(&raw, InsertMethod::Paste) {
            tracing::warn!("paste last raw failed: {e}");
        }
    });
}

/// Puts the last result on the clipboard.
pub fn copy_last(history: &Arc<History>) {
    if let Some(last) = history.last()
        && let Err(e) = splashdown_platform::copy_to_clipboard(&last.text)
    {
        tracing::warn!("copy last failed: {e}");
    }
}

/// Reads the last result aloud, to check it by ear.
pub fn read_last(history: &Arc<History>) {
    let Some(last) = history.last() else {
        return;
    };
    if let Err(e) = crate::reading::read(&last.text, "Reading your last dictation") {
        tracing::info!("read last: {e}");
    }
}
