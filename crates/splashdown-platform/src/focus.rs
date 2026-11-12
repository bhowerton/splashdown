//! The focused app and field (`Focus`, captured when dictation starts):
//! its text and selection, the cursor, the text around it in the window,
//! and the text helpers that work in UTF-16 offsets as the OSes do.

use super::*;

/// The app that has keyboard focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    /// Bundle identifier (macOS) or executable name (Windows).
    pub id: String,
    pub name: String,
    pub pid: i32,
    pub window_title: Option<String>,
    /// The page address when the focused field is in a web page.
    pub url: Option<String>,
    /// The file or folder the focused window shows, when the app says: a
    /// terminal's working folder, an editor's open file.
    pub document: Option<std::path::PathBuf>,
}

/// The byte offset of the `units`-th UTF-16 unit in `text`.
pub(super) fn utf16_to_byte(text: &str, units: usize) -> Option<usize> {
    let mut seen = 0;
    for (byte, c) in text.char_indices() {
        if seen >= units {
            return Some(byte);
        }
        seen += c.len_utf16();
    }
    (seen >= units).then_some(text.len())
}

/// `file:///Users/me/My%20Project/` → `/Users/me/My Project/`.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn file_url_path(url: &str) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok().map(std::path::PathBuf::from)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Text(String),
    /// The field reports that nothing is selected.
    Empty,
    /// The app doesn't expose its selection; try `copy_selection`.
    Unknown,
}

/// Where keyboard focus was when dictation started.
pub struct Focus {
    pub app: AppInfo,
    pub(super) inner: imp::FocusElement,
}

impl Focus {
    /// Captures the current focus, or `None` if no app has focus.
    pub fn capture() -> Option<Self> {
        let (app, inner) = imp::capture_focus()?;
        Some(Self { app, inner })
    }

    /// Whether the same app and field still have focus.
    pub fn is_current(&self) -> bool {
        imp::is_current(&self.app, &self.inner)
    }

    /// Selected text in the focused field, via the accessibility API.
    pub fn selection(&self) -> Selection {
        imp::selection(&self.inner)
    }

    /// Whether the focused field is a password field.
    pub fn is_secure_field(&self) -> bool {
        imp::is_secure_field(&self.inner)
    }

    /// The field's current text, if the app exposes it (skipped for very
    /// long documents).
    pub fn field_text(&self) -> Option<String> {
        imp::field_text(&self.inner)
    }

    /// Whether the focused element takes typed text. `Some(false)` only when
    /// it clearly doesn't (a button, a list, a web page outside any field,
    /// Finder's desktop), `None` when that can't be told, as in apps that
    /// expose nothing.
    pub fn takes_text(&self) -> Option<bool> {
        let known = imp::takes_text(&self.inner);
        // An element of no telling kind that has nothing a text view has
        // isn't one. Terminals are left out: some draw their own text and
        // say nothing about it.
        #[cfg(target_os = "macos")]
        if known.is_none()
            && !crate::is_terminal(&self.app.id)
            && imp::nothing_like_text(&self.inner)
        {
            return Some(false);
        }
        known
    }

    /// Whether pasting over the selection would change it: `Some(false)`
    /// when the system says its text can't be changed (a message received,
    /// a page or document opened to read, whose text view looks like an
    /// editable one), `None` when it doesn't say. Not the same question as
    /// `takes_text`, which plain dictation goes by: a terminal takes typed
    /// text without letting its text be set.
    pub fn can_replace_selection(&self) -> Option<bool> {
        imp::can_replace_selection(&self.inner)
    }

    /// Whether plain dictation into this field, which says it takes text,
    /// should be checked for going in: a read-only text view (a message
    /// received, a document opened to read) has the role of an editable
    /// one, and typing there goes nowhere. Never for a terminal, which
    /// says its text can't be set and still takes typing. On a Mac, unless
    /// the field says its selection can be set: a messaging app's typing
    /// box and its received messages both leave that unsaid, and the check
    /// passes at once in the typing box. On Windows only where UI
    /// Automation says the text is read-only, since it says nothing for
    /// most fields that do take typing.
    pub fn typing_may_not_land(&self) -> bool {
        if crate::is_terminal(&self.app.id) {
            return false;
        }
        let can_replace = imp::can_replace_selection(&self.inner);
        if cfg!(target_os = "macos") {
            can_replace != Some(true)
        } else {
            can_replace == Some(false)
        }
    }

    /// The text around the field in its window, besides the field's own:
    /// usually the conversation being answered (an email thread, a chat).
    /// Read from the nearest container holding at least `min_chars`, and
    /// at most the last `max_chars` of it. `None` for password fields, or
    /// where the app exposes no text.
    pub fn surrounding_text(&self, min_chars: usize, max_chars: usize) -> Option<String> {
        imp::surrounding_text(&self.inner, min_chars, max_chars)
    }

    /// The field's whole text and where the insertion point is in it (a
    /// byte offset into the text), if the app reports both.
    pub fn text_and_cursor(&self) -> Option<(String, usize)> {
        let text = imp::field_text(&self.inner)?;
        let cursor = imp::cursor_offset(&self.inner)?;
        Some((text.clone(), utf16_to_byte(&text, cursor)?))
    }

    /// Selects the text between two byte offsets of `text_and_cursor`'s
    /// text. Returns whether the app did it.
    pub fn select(&self, text: &str, start: usize, end: usize) -> bool {
        let utf16 = |b: usize| text.get(..b).map(|s| s.encode_utf16().count());
        match (utf16(start), utf16(end)) {
            (Some(from), Some(to)) if to >= from => imp::select_range(&self.inner, from, to - from),
            _ => false,
        }
    }

    /// Where the insertion point is on screen, if the app reports it.
    pub fn caret_rect(&self) -> Option<CaretRect> {
        imp::caret_rect(&self.inner)
    }

    /// Up to `before` characters before the insertion point and `after`
    /// characters after it, with "…" where the text was cut. `None` for
    /// password fields, empty fields, or apps that don't expose their text.
    /// Without a reported cursor, the end of the text is used.
    pub fn text_around_cursor(&self, before: usize, after: usize) -> Option<String> {
        let text = self.field_text()?;
        let cursor = imp::cursor_offset(&self.inner);
        window_around(&text, cursor, before, after)
    }
}

/// What the system's answers to "can this be set?" for the selected text
/// and for the whole value (`None` where it didn't answer) say about
/// pasting over the selection. Only the selection's own answer is taken as
/// a yes: some apps say a read-only text view's value can be set, and say
/// the same of their real typing box, so a settable value proves nothing.
/// No only when both were asked and both said no.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn replaceable(selected: Option<bool>, value: Option<bool>) -> Option<bool> {
    match (selected, value) {
        (Some(true), _) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

/// The text within `before`/`after` characters of `cursor` (a UTF-16 offset,
/// as accessibility APIs report it), trimmed, with "…" marking cuts.
pub(super) fn window_around(
    text: &str,
    cursor_utf16: Option<usize>,
    before: usize,
    after: usize,
) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    // UTF-16 offset → char index.
    let cursor = match cursor_utf16 {
        Some(target) => {
            let mut units = 0;
            chars
                .iter()
                .position(|c| {
                    let reached = units >= target;
                    units += c.len_utf16();
                    reached
                })
                .unwrap_or(chars.len())
        }
        None => chars.len(),
    };
    let start = cursor.saturating_sub(before);
    let end = (cursor + after).min(chars.len());
    let window: String = chars[start..end].iter().collect();
    let window = window.trim();
    if window.is_empty() {
        return None;
    }
    let before_mark = if start > 0 { "…" } else { "" };
    let after_mark = if end < chars.len() { "…" } else { "" };
    Some(format!("{before_mark}{window}{after_mark}"))
}

/// A rectangle on screen in points, with the origin at the top left of the
/// primary display (accessibility coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The last `chars` characters of `text`, starting at a line or word
/// where there is one close by.
pub(crate) fn keep_end(text: &str, chars: usize) -> String {
    let count = text.chars().count();
    if count <= chars {
        return text.to_string();
    }
    let start = text.char_indices().nth(count - chars).map_or(0, |(i, _)| i);
    let tail = &text[start..];
    // A clean start: the next line, or else the next word, if it's near.
    let cut = tail
        .find('\n')
        .filter(|&i| i < 200)
        .or_else(|| tail.find(' ').filter(|&i| i < 40))
        .map_or(0, |i| i + 1);
    tail[cut..].to_string()
}
