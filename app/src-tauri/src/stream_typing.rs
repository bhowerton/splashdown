//! Typing the AI's text while it's still being written, so it starts to
//! appear at the first words rather than once the whole reply is in.
//!
//! Only whole words are typed (the last one may still grow), shaped the way
//! the final text will be (file tags, replacements). When the reply is
//! complete, the final text (after the guard, which may reject what was
//! streamed) is compared with what was typed: the rest is typed, after
//! deleting any end that differs.

use std::sync::Arc;

use splashdown_platform::{Focus, InsertMethod, KeyAction};
use unicode_segmentation::UnicodeSegmentation;

use crate::project::Project;
use crate::replacements::Replacements;

pub struct StreamTyper {
    files: Option<Arc<Project>>,
    replacements: Replacements,
    /// What's been typed into the field so far, with a leading space if one
    /// was needed.
    typed: String,
    /// Decided when the first words are typed.
    space: Option<bool>,
    /// Typing failed or the focus moved: nothing more is typed.
    stopped: bool,
}

impl StreamTyper {
    /// For dictation into `focus`, unless it's a field not to stream into.
    pub fn new(
        focus: &Focus,
        files: Option<Arc<Project>>,
        replacements: Replacements,
    ) -> Option<Self> {
        // A terminal runs each line it's given, a password field doesn't
        // want a draft, and in a spreadsheet a typed line break moves to
        // another cell.
        if splashdown_platform::is_terminal(&focus.app.id)
            || focus.is_secure_field()
            || is_spreadsheet(focus)
        {
            return None;
        }
        Some(Self {
            files,
            replacements,
            typed: String::new(),
            space: None,
            stopped: false,
        })
    }

    /// Whether anything was typed (then `finish` must complete it).
    pub fn started(&self) -> bool {
        !self.typed.is_empty()
    }

    /// Typing stopped part way (focus moved, or typing failed): what's in
    /// the field isn't known, so it can't be finished from.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Whether the text began with a space, to keep the final text the same.
    pub fn space(&self) -> bool {
        self.space == Some(true)
    }

    /// The text the way it will be inserted: file names tagged, replacement
    /// rules applied.
    pub fn shape(&self, text: &str) -> String {
        let text = match &self.files {
            Some(project) => splashdown_core::files::tag_files(text, &project.files),
            None => text.to_string(),
        };
        self.replacements.apply(&text)
    }

    /// Takes the reply so far and types the words that are new into
    /// `focus`, the field dictated into.
    pub fn offer(&mut self, focus: &Focus, so_far: &str) {
        if self.stopped {
            return;
        }
        let Some(stable) = whole_words(so_far) else {
            return;
        };
        let shaped = self.shape(stable);
        let space = *self.space.get_or_insert_with(|| {
            focus.text_and_cursor().is_some_and(|(field, cursor)| {
                splashdown_core::spoken::needs_space(
                    field[..cursor].chars().next_back(),
                    shaped.chars().next(),
                )
            })
        });
        let wanted = if space { format!(" {shaped}") } else { shaped };
        // An earlier part changed (a replacement rule now matches across
        // the new words): leave it for `finish`.
        let Some(rest) = wanted.strip_prefix(self.typed.as_str()) else {
            return;
        };
        if rest.is_empty() {
            return;
        }
        if !focus.is_current() {
            tracing::info!("focus moved while typing the streamed text; stopped");
            self.stopped = true;
            return;
        }
        match splashdown_platform::insert_text(rest, self.method()) {
            Ok(()) => self.typed = wanted,
            Err(e) => {
                tracing::warn!("typing the streamed text failed: {e}");
                self.stopped = true;
            }
        }
    }

    /// Makes what was typed into `focus` into `text`, the final text (with
    /// its leading space, if any): deletes from where they differ, then
    /// types the rest. Stops if the focus moves while deleting.
    pub fn finish(&mut self, focus: &Focus, text: &str) -> splashdown_platform::Result<()> {
        let same = common_start(&self.typed, text);
        // One Backspace deletes a whole character as shown: an emoji with
        // its skin tone, a flag, an accented letter.
        let extra = self.typed[same..].graphemes(true).count();
        if extra > 0 {
            tracing::info!(
                "the final text differs from what was streamed: retyping the last {extra} characters"
            );
        }
        for i in 0..extra {
            if i % 10 == 0 && !focus.is_current() {
                return Err(splashdown_platform::PlatformError::Failed(
                    "the focus moved".into(),
                ));
            }
            splashdown_platform::press(KeyAction::Backspace)?;
        }
        let rest = &text[same..];
        if !rest.is_empty() {
            splashdown_platform::insert_text(rest, self.method())?;
        }
        self.typed = text.to_string();
        Ok(())
    }

    /// Line breaks as Shift+Return, which chat apps take as a new line
    /// rather than "send" (terminals aren't streamed into).
    fn method(&self) -> InsertMethod {
        InsertMethod::Type { soft_breaks: true }
    }
}

/// Spreadsheets, by app id or web address.
fn is_spreadsheet(focus: &Focus) -> bool {
    let id = focus.app.id.to_lowercase();
    [
        "com.microsoft.excel",
        "com.apple.iwork.numbers",
        "excel.exe",
    ]
    .contains(&id.as_str())
        || focus
            .app
            .url
            .as_deref()
            .is_some_and(|u| u.contains("docs.google.com/spreadsheets"))
}

/// The reply up to its last whole word: the last word may still grow.
/// `None` until there's a whole word.
fn whole_words(so_far: &str) -> Option<&str> {
    let text = so_far.trim_start();
    let end = text.rfind(char::is_whitespace)?;
    Some(text[..end].trim_end()).filter(|s| !s.is_empty())
}

/// The length in bytes of the longest common start of `a` and `b`, on a
/// boundary between characters as shown in `a` (so deleting back to it
/// never leaves half an emoji or an accent without its letter).
fn common_start(a: &str, b: &str) -> usize {
    let same = common_prefix(a, b);
    a.grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(a.len()))
        .take_while(|&i| i <= same)
        .last()
        .unwrap_or(0)
}

/// The length in bytes of the longest common start of `a` and `b`, on a
/// character boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or_else(|| a.len().min(b.len()), |((i, _), _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whole_words_are_typed() {
        assert_eq!(whole_words("Hel"), None);
        assert_eq!(whole_words("  Hello"), None);
        assert_eq!(whole_words("Hello th"), Some("Hello"));
        assert_eq!(whole_words("Hello there. "), Some("Hello there."));
        assert_eq!(whole_words("First line.\n\nSec"), Some("First line."));
        assert_eq!(whole_words("Hello\n"), Some("Hello"));
    }

    #[test]
    fn common_prefix_is_on_a_character_boundary() {
        assert_eq!(common_prefix("Hello there", "Hello world"), 6);
        assert_eq!(common_prefix("Hello", "Hello there"), 5);
        assert_eq!(common_prefix("Hello there", "Hello"), 5);
        assert_eq!(common_prefix("café au", "café noir"), 6);
        assert_eq!(common_prefix("é", "è"), 0);
        assert_eq!(common_prefix("", "x"), 0);
    }

    #[test]
    fn deleting_back_keeps_whole_characters() {
        // A thumbs-up with a skin tone differs from one without in its
        // second code point: the whole emoji is retyped.
        assert_eq!(common_start("Great 👍🏽", "Great 👍"), 6);
        // An accent typed as a separate mark goes with its letter.
        assert_eq!(common_start("cafe\u{301} au", "cafe noir"), 3);
        assert_eq!(common_start("Hello there", "Hello world"), 6);
        assert_eq!(common_start("Hello", "Hello there"), 5);
        assert_eq!("Great 👍🏽"[6..].graphemes(true).count(), 1);
    }
}
