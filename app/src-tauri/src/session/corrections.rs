//! Watching the field after an insertion for names and terms the user
//! corrects, and learning them (`crate::learning`).

use super::*;

impl Controller {
    /// Reads the watched field and learns the names and terms the user
    /// corrected ("Pria" → "Priya"), saying so. A reading is looked at
    /// once it's settled (unchanged for `CORRECTION_SETTLE`), or,
    /// when the field was sent or cleared, the reading before that.
    /// `moving_on`: a new dictation is starting, so this is the last look.
    pub(super) fn check_corrections(&mut self, moving_on: bool) {
        let Some(mut check) = self.correction_check.take() else {
            return;
        };
        let Ok(engine) = &self.engine else {
            return;
        };
        let now = Instant::now();
        let reading = check.focus.field_text();
        let gone = reading
            .as_deref()
            .is_none_or(|t| !splashdown_core::vocab::still_there(&check.inserted, t));
        if reading != check.last {
            check.changed = now;
        }
        // What to look at now, if anything: the reading before the field
        // was sent or cleared, or a settled one, or this one when it's the
        // last look.
        let settled = match (&reading, gone) {
            (_, true) => check.last.take(),
            (Some(text), false)
                if moving_on
                    || check.again
                    || now.duration_since(check.changed) >= CORRECTION_SETTLE =>
            {
                Some(text.clone())
            }
            _ => None,
        };
        let words = settled
            .map(|text| {
                splashdown_core::vocab::corrections(
                    &check.inserted,
                    &text,
                    &engine.context.vocabulary,
                )
            })
            .unwrap_or_default();
        // Keep watching while the dictation is still in the field.
        if !moving_on && !gone {
            if now < check.until {
                self.correction_check = Some(PendingCorrectionCheck {
                    due: now + CORRECTION_POLL,
                    last: reading,
                    ..check
                });
            } else if !check.again && check.focus.is_current() {
                self.correction_check = Some(PendingCorrectionCheck {
                    due: now + CORRECTION_CHECK_AGAIN,
                    until: now,
                    last: reading,
                    again: true,
                    ..check
                });
            }
        }
        if words.is_empty() {
            return;
        }
        let learned = crate::learning::learn(self.ui.app(), words);
        // Not over a recording that's starting.
        if !learned.is_empty() && self.active.is_none() {
            let quoted: Vec<String> = learned
                .iter()
                .map(|w| format!("\u{201c}{w}\u{201d}"))
                .collect();
            self.ui.show(OverlayState::Message {
                tone: Tone::Success,
                text: format!("Learned {}", quoted.join(", ")),
            });
            self.hide_at = Some(Instant::now() + LEARNED_SHOWN);
        }
    }
}

/// An insertion being watched for corrections: its field is read every
/// `CORRECTION_POLL` until `CORRECTION_WATCH` is up, so a fix made just
/// before the message is sent is still seen.
pub(super) struct PendingCorrectionCheck {
    pub(super) focus: Focus,
    pub(super) inserted: String,
    /// The next reading.
    pub(super) due: Instant,
    /// When watching ends (then one more look, `CORRECTION_CHECK_AGAIN`).
    pub(super) until: Instant,
    /// The last reading, to look at once the field is sent or cleared.
    pub(super) last: Option<String>,
    /// When the reading last changed: it's looked at once it has stayed
    /// the same for `CORRECTION_SETTLE` (not halfway through a word).
    pub(super) changed: Instant,
    /// The second look: there's no third.
    pub(super) again: bool,
}

/// How long "Learned “Priya”" shows.
pub(super) const LEARNED_SHOWN: Duration = Duration::from_secs(3);

/// How often the field is read after an insertion, and for how long; then,
/// while the field is still the one in use, one more look after
/// `CORRECTION_CHECK_AGAIN`.
pub(super) const CORRECTION_POLL: Duration = Duration::from_millis(500);

pub(super) const CORRECTION_WATCH: Duration = Duration::from_secs(30);

pub(super) const CORRECTION_CHECK_AGAIN: Duration = Duration::from_secs(120);

pub(super) const CORRECTION_SETTLE: Duration = Duration::from_secs(2);
