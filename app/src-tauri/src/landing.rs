//! Whether text put into another app lands there: where a new version of
//! selected text goes (`Replace`), what Insert does in a panel (`Insert`),
//! and the check that a field's text changed after a paste. The one place
//! these rules live, for the session and the panels alike.

use std::time::{Duration, Instant};

use splashdown_platform::Focus;

/// Where a new version of selected text goes. Every rewrite of a
/// selection (a spoken instruction, a transform's shortcut, translating)
/// goes by this one rule (`replace_rule`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Replace {
    /// Pasted over it: the field takes text and says it can be changed.
    Paste,
    /// Pasted over it, then the field is checked for the change: it takes
    /// text but doesn't say whether this text can be changed.
    PasteAndCheck,
    /// Shown in the Ask panel: pasted, it would go nowhere (a message
    /// received, a web page, a document opened to read).
    Show,
}

/// The rule for `Replace`, from what the focused element says: whether it
/// takes typed text (`Focus::takes_text`) and whether its selection can be
/// changed (`Focus::can_replace_selection`). Only a field known to take
/// text gets a paste, and only one known to be changeable gets it without
/// a check. A checked paste that isn't seen to change the field is shown
/// in the Ask panel after all (`went`).
pub(crate) fn replace_rule(takes_text: Option<bool>, can_replace: Option<bool>) -> Replace {
    match (takes_text, can_replace) {
        (Some(true), Some(true)) => Replace::Paste,
        (Some(true), None) => Replace::PasteAndCheck,
        _ => Replace::Show,
    }
}

/// `replace_rule` for the selection in `focus`.
pub(crate) fn replace_target(focus: &Focus) -> Replace {
    let takes_text = focus.takes_text();
    // Not asked where it wouldn't change the answer.
    let can_replace = (takes_text == Some(true))
        .then(|| focus.can_replace_selection())
        .flatten();
    replace_rule(takes_text, can_replace)
}

/// What a panel's Insert does with what has focus in the app it pastes
/// into (an answer, a draft, a note sent to an app).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Insert {
    /// Nothing there can take it (a list, a message received): not
    /// pasted, and the user is asked to click where it should go.
    Refuse,
    /// Pasted: the field says it takes it, or it's a terminal, which pastes
    /// at its prompt whatever its text view says.
    Paste,
    /// Pasted, then the field is checked for the change.
    PasteAndCheck,
}

/// The rule for `Insert`: `takes_text` and `can_replace` as
/// `replace_rule` takes them, and whether the app is a terminal. A
/// terminal's text view says its text can't be set, yet it takes a paste
/// at the prompt, so only "takes no text" refuses there.
pub(crate) fn insert_rule(
    takes_text: Option<bool>,
    can_replace: Option<bool>,
    terminal: bool,
) -> Insert {
    if takes_text == Some(false) || (can_replace == Some(false) && !terminal) {
        Insert::Refuse
    } else if terminal || (takes_text == Some(true) && can_replace == Some(true)) {
        Insert::Paste
    } else {
        Insert::PasteAndCheck
    }
}

/// Why a panel's Insert didn't put the text in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unpasted {
    /// What has focus there can't take text.
    Nowhere,
    /// Pasted, but the field wasn't seen to change.
    NotSeen,
    /// The app didn't come to the front, or the paste failed.
    Failed(String),
}

impl Unpasted {
    /// What the panel says, for an app called `name`. The text is on the
    /// clipboard by then.
    pub(crate) fn message(&self, name: &str) -> String {
        match self {
            Unpasted::Nowhere => format!(
                "{name} can't take text there. Click where it should go and press Insert again. It's on the clipboard."
            ),
            Unpasted::NotSeen => format!(
                "It didn't show up in {name}. Click where it should go and press Insert again. It's on the clipboard."
            ),
            Unpasted::Failed(_) => format!("Couldn't paste into {name}. It's on the clipboard."),
        }
    }
}

/// Pastes `text` into what has focus now (the app the panel inserts into,
/// already in front), by `insert_rule`.
pub(crate) fn paste_here(text: &str) -> Result<(), Unpasted> {
    let focus = Focus::capture();
    let how = match &focus {
        Some(focus) => {
            let takes_text = focus.takes_text();
            let can_replace = (takes_text != Some(false))
                .then(|| focus.can_replace_selection())
                .flatten();
            insert_rule(
                takes_text,
                can_replace,
                splashdown_platform::is_terminal(&focus.app.id),
            )
        }
        // Nothing to ask: as before, pasted.
        None => Insert::Paste,
    };
    if how == Insert::Refuse {
        return Err(Unpasted::Nowhere);
    }
    let before = focus
        .as_ref()
        .filter(|_| how == Insert::PasteAndCheck)
        .and_then(|f| f.field_text().map(|t| (f, t)));
    splashdown_platform::insert_text(text, splashdown_platform::InsertMethod::Paste)
        .map_err(|e| Unpasted::Failed(e.to_string()))?;
    match before {
        Some((focus, before)) if !field_changed(focus, &before) => Err(Unpasted::NotSeen),
        // Seen to change, or there's nothing to compare with.
        _ => Ok(()),
    }
}

/// How long to wait for a paste to show in a field's text.
pub(crate) const PASTE_CHECK: Duration = Duration::from_millis(800);

/// Whether the field's text moved on from `before` within `PASTE_CHECK`.
pub(crate) fn field_changed(focus: &Focus, before: &str) -> bool {
    let until = Instant::now() + PASTE_CHECK;
    loop {
        if focus.field_text().is_some_and(|now| now != before) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_is_only_pasted_over_where_it_can_change() {
        // A field that says it can be changed: pasted, no waiting.
        assert_eq!(replace_rule(Some(true), Some(true)), Replace::Paste);
        // A field that doesn't say: pasted, and checked for.
        assert_eq!(replace_rule(Some(true), None), Replace::PasteAndCheck);
        // A text view that says none of its text can be set (a terminal).
        assert_eq!(replace_rule(Some(true), Some(false)), Replace::Show);
        // Not known to take text at all (a web page, an app that says
        // nothing): a paste would go nowhere to be seen.
        assert_eq!(replace_rule(None, None), Replace::Show);
        assert_eq!(replace_rule(None, Some(true)), Replace::Show);
        assert_eq!(replace_rule(Some(false), None), Replace::Show);
    }

    #[test]
    fn insert_refuses_only_where_nothing_can_take_it() {
        // A list, a web page outside any field.
        assert_eq!(insert_rule(Some(false), None, false), Insert::Refuse);
        // Text that says it can't be changed: a message received.
        assert_eq!(insert_rule(Some(true), Some(false), false), Insert::Refuse);
        // A terminal says the same of its text, and still pastes at the
        // prompt.
        assert_eq!(insert_rule(Some(true), Some(false), true), Insert::Paste);
        assert_eq!(insert_rule(Some(false), None, true), Insert::Refuse);
        // A field known to take it: no check.
        assert_eq!(insert_rule(Some(true), Some(true), false), Insert::Paste);
        // Anything not known: pasted and checked where it can be read.
        assert_eq!(insert_rule(Some(true), None, false), Insert::PasteAndCheck);
        assert_eq!(insert_rule(None, None, false), Insert::PasteAndCheck);
    }
}
