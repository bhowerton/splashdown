//! Hotkey gesture recognition (spec §2.1–§2.2), independent of any OS.
//!
//! The platform layer reports raw key presses; the recognizer turns them
//! into session actions and decides which keys to swallow. Time is passed
//! in so every rule is unit-testable.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Physical keys the app cares about. Everything else is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhysicalKey {
    LeftShift,
    RightShift,
    LeftControl,
    RightControl,
    /// Option on macOS, Alt on Windows.
    #[serde(alias = "left-option")]
    LeftAlt,
    #[serde(alias = "right-option")]
    RightAlt,
    /// Command on macOS, Windows key on Windows.
    #[serde(alias = "left-command", alias = "left-win")]
    LeftMeta,
    #[serde(alias = "right-command", alias = "right-win")]
    RightMeta,
    /// The Fn / 🌐 key (macOS).
    Fn,
    Escape,
    /// Return or keypad Enter.
    Return,
    /// A number key on the main row, 0–9.
    Digit(u8),
    /// A letter key, a–z (by position on macOS's US layout; by the
    /// layout's virtual key on Windows).
    Letter(char),
    /// The mouse's "back" side button (button 4).
    #[serde(rename = "mouse-4")]
    MouseBack,
    /// The mouse's "forward" side button (button 5).
    #[serde(rename = "mouse-5")]
    MouseForward,
    Other,
}

impl PhysicalKey {
    /// Default dictation key: Right Option on macOS, Right Ctrl on Windows
    /// (Right Alt is AltGr on many Windows layouts).
    pub fn default_dictation() -> Self {
        if cfg!(target_os = "macos") {
            PhysicalKey::RightAlt
        } else {
            PhysicalKey::RightControl
        }
    }

    pub fn default_instruction() -> Self {
        PhysicalKey::RightShift
    }

    /// Display name for UI, using the platform's terms.
    pub fn label(self) -> &'static str {
        let mac = cfg!(target_os = "macos");
        match self {
            PhysicalKey::LeftShift => "Left Shift",
            PhysicalKey::RightShift => "Right Shift",
            PhysicalKey::LeftControl => "Left Ctrl",
            PhysicalKey::RightControl => "Right Ctrl",
            PhysicalKey::LeftAlt if mac => "Left Option",
            PhysicalKey::LeftAlt => "Left Alt",
            PhysicalKey::RightAlt if mac => "Right Option",
            PhysicalKey::RightAlt => "Right Alt",
            PhysicalKey::LeftMeta if mac => "Left Command",
            PhysicalKey::LeftMeta => "Left Windows",
            PhysicalKey::RightMeta if mac => "Right Command",
            PhysicalKey::RightMeta => "Right Windows",
            PhysicalKey::Fn => "Fn",
            PhysicalKey::Escape => "Esc",
            PhysicalKey::Return if mac => "Return",
            PhysicalKey::Return => "Enter",
            PhysicalKey::Digit(_) => "number key",
            PhysicalKey::Letter(_) => "letter key",
            PhysicalKey::MouseBack => "Mouse button 4 (back)",
            PhysicalKey::MouseForward => "Mouse button 5 (forward)",
            PhysicalKey::Other => "key",
        }
    }

    /// A mouse button rather than a key.
    pub fn is_mouse(self) -> bool {
        matches!(self, PhysicalKey::MouseBack | PhysicalKey::MouseForward)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: PhysicalKey,
    pub down: bool,
}

impl KeyEvent {
    pub fn down(key: PhysicalKey) -> Self {
        Self { key, down: true }
    }
    pub fn up(key: PhysicalKey) -> Self {
        Self { key, down: false }
    }
}

/// What the session should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Dictation key went down: start recording now (so the first word
    /// isn't clipped), even though it may turn out to be an accidental tap.
    Start,
    /// Instruction key tapped: what follows is the instruction.
    MarkInstruction,
    /// Double-tap: recording continues hands-free until the next tap.
    HandsFree,
    /// Stop recording and process it.
    Finish,
    /// Stop recording and throw it away. `silent` cancels (accidental tap,
    /// keyboard shortcut) show nothing; Esc shows "Canceled".
    Cancel { silent: bool },
    /// Esc on a long recording: ask before discarding `seconds` of speech.
    /// A second Esc soon after cancels; otherwise `KeepRecording` follows.
    ConfirmCancel { seconds: u64 },
    /// The cancel confirmation expired; recording carries on.
    KeepRecording,
    /// A number key while holding the dictation key: switch to preset
    /// `n` (1-based, in the order shown in Settings).
    SelectPreset(u8),
    /// Return while recording, or while the result is still on its way:
    /// press Return after inserting (`true`), or not after all (`false`).
    /// Each press flips it.
    ToggleSend(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Shorter presses are taps, not holds.
    pub hold: Duration,
    /// Window for the second tap of a double-tap.
    pub double_tap: Duration,
    /// Esc on a recording at least this long asks for confirmation.
    pub confirm_cancel_after: Duration,
    /// How long the "press Esc again" confirmation waits.
    pub confirm_window: Duration,
    /// After a recording ends, Return toggles auto-send until the session
    /// calls `take_send`, or at most this long.
    pub processing_limit: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            hold: Duration::from_millis(250),
            double_tap: Duration::from_millis(300),
            confirm_cancel_after: Duration::from_secs(30),
            confirm_window: Duration::from_millis(2500),
            processing_limit: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// Dictation key is down; recording.
    Pressed {
        since: Instant,
        marked: bool,
    },
    /// A short tap ended; recording continues while we wait for a second tap.
    TapReleased {
        at: Instant,
    },
    /// Latched recording. `ignore_up` swallows the second tap's key-up.
    HandsFree {
        marked: bool,
        ignore_up: bool,
    },
    /// The session ended while the dictation key is still physically down;
    /// ignore keys until it comes up.
    AwaitRelease,
}

pub struct Recognizer {
    dictation: PhysicalKey,
    instruction: PhysicalKey,
    /// Pressed while holding the dictation key: carry on hands-free.
    hands_free: PhysicalKey,
    /// Whether a double-tap also starts hands-free (always for mouse
    /// buttons, which have no other key to press).
    double_tap: bool,
    /// We swallowed the hands-free key's key-down, so swallow its repeats
    /// and key-up too.
    swallow_hands_free: bool,
    timing: Timing,
    state: State,
    /// We swallowed an Esc key-down, so swallow its key-up too.
    swallow_escape_up: bool,
    /// When the current recording started.
    recording_since: Option<Instant>,
    /// A "press Esc again to discard" confirmation is open until then.
    cancel_armed_until: Option<Instant>,
    /// Press Return once the text is inserted.
    send: bool,
    /// The finished recording is being processed (until then, at most).
    processing_until: Option<Instant>,
    /// Return is physically down: its auto-repeats don't toggle again.
    return_held: bool,
    /// We swallowed a Return key-down, so swallow its key-up too.
    swallow_return_up: bool,
}

impl Recognizer {
    pub fn new(dictation: PhysicalKey, instruction: PhysicalKey, timing: Timing) -> Self {
        Self {
            dictation,
            instruction,
            hands_free: PhysicalKey::Letter('h'),
            double_tap: true,
            swallow_hands_free: false,
            timing,
            state: State::Idle,
            swallow_escape_up: false,
            recording_since: None,
            cancel_armed_until: None,
            send: false,
            processing_until: None,
            return_held: false,
            swallow_return_up: false,
        }
    }

    /// Sets the key that, pressed while holding the dictation key, carries
    /// on hands-free, and whether a double-tap does too.
    pub fn with_hands_free(mut self, key: PhysicalKey, double_tap: bool) -> Self {
        self.hands_free = key;
        self.double_tap = double_tap;
        self
    }

    /// Turns auto-send on or off for the recording in progress (a preset
    /// that always sends). Return still toggles it afterward.
    pub fn set_send(&mut self, on: bool) {
        self.send = on;
    }

    /// Ends the processing window and returns whether to press Return after
    /// inserting. Call once the result is ready (or the dictation failed);
    /// Return reaches apps normally again from then on.
    pub fn take_send(&mut self) -> bool {
        self.processing_until = None;
        std::mem::take(&mut self.send)
    }

    fn processing(&self, now: Instant) -> bool {
        self.processing_until.is_some_and(|t| now < t)
    }

    /// Whether a recording session is in progress.
    pub fn is_recording(&self) -> bool {
        matches!(
            self.state,
            State::Pressed { .. } | State::TapReleased { .. } | State::HandsFree { .. }
        )
    }

    pub fn is_hands_free(&self) -> bool {
        matches!(self.state, State::HandsFree { .. })
    }

    /// When `on_timeout` must next be called, if ever.
    pub fn deadline(&self) -> Option<Instant> {
        let tap = match self.state {
            State::TapReleased { at } => Some(at + self.timing.double_tap),
            _ => None,
        };
        match (tap, self.cancel_armed_until) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Handles a deadline passing. Call when `now >= deadline()`.
    pub fn on_timeout(&mut self, now: Instant) -> Option<Action> {
        if self.cancel_armed_until.is_some_and(|t| now >= t) {
            self.cancel_armed_until = None;
            if self.is_recording() {
                return Some(Action::KeepRecording);
            }
        }
        match self.state {
            State::TapReleased { at } if now >= at + self.timing.double_tap => {
                self.end_recording();
                Some(Action::Cancel { silent: true })
            }
            _ => None,
        }
    }

    /// Back to idle (or waiting for the key to come up), clearing the
    /// per-recording bookkeeping.
    fn end_recording(&mut self) {
        self.state = match self.state {
            State::Pressed { .. } | State::AwaitRelease => State::AwaitRelease,
            _ => State::Idle,
        };
        self.recording_since = None;
        self.cancel_armed_until = None;
    }

    /// Ends the session from outside (e.g. hands-free auto-stop).
    pub fn finish(&mut self) -> Option<Action> {
        if !self.is_recording() {
            return None;
        }
        self.end_recording();
        self.processing_until = Some(Instant::now() + self.timing.processing_limit);
        Some(Action::Finish)
    }

    /// Starts a hands-free recording from outside (`splashdown-cli start`, a
    /// `splashdown://` link, an agent's question): the next tap of the
    /// dictation key finishes it, Esc cancels it. `None` if a recording is
    /// already in progress.
    pub fn start_hands_free(&mut self, now: Instant) -> Option<Action> {
        if self.is_recording() {
            return None;
        }
        // If the key is still down (`AwaitRelease`), its key-up is ignored
        // like a double-tap's.
        self.state = State::HandsFree {
            marked: false,
            ignore_up: false,
        };
        self.recording_since = Some(now);
        self.cancel_armed_until = None;
        self.send = false;
        self.processing_until = None;
        Some(Action::Start)
    }

    /// Discards the recording from outside. `None` if there's none.
    pub fn cancel(&mut self) -> Option<Action> {
        if !self.is_recording() {
            return None;
        }
        self.end_recording();
        self.send = false;
        Some(Action::Cancel { silent: false })
    }

    /// Handles a key event. Returns whether to swallow it (so the focused
    /// app never sees it) and the resulting action.
    pub fn on_event(&mut self, ev: KeyEvent, now: Instant) -> (bool, Option<Action>) {
        // Mouse side buttons count only as the dictation key, and then the
        // app never sees them (no going back a page); otherwise they're
        // left alone, and never cancel a recording as a chord.
        if ev.key.is_mouse() && ev.key != self.dictation {
            return (false, None);
        }
        if ev.key == PhysicalKey::Escape {
            return self.on_escape(ev.down, now);
        }
        if ev.key == PhysicalKey::Return
            && let Some(result) = self.on_return(ev.down, now)
        {
            return result;
        }

        let is_dictation = ev.key == self.dictation;
        let is_instruction = ev.key == self.instruction;

        // The hands-free key while holding the dictation key: let go and
        // carry on. Swallowed, with its repeats and key-up, so nothing is
        // typed.
        if ev.key == self.hands_free && ev.key != PhysicalKey::Other {
            if self.swallow_hands_free {
                self.swallow_hands_free = ev.down;
                return (true, None);
            }
            if let (State::Pressed { marked, .. }, true) = (self.state, ev.down) {
                self.state = State::HandsFree {
                    marked,
                    ignore_up: false,
                };
                self.swallow_hands_free = true;
                return (true, Some(Action::HandsFree));
            }
        }

        // Number keys while holding the dictation key pick a preset; they're
        // swallowed (key-down and key-up) so nothing gets typed.
        if let (State::Pressed { .. }, PhysicalKey::Digit(n)) = (self.state, ev.key) {
            let action = (ev.down && (1..=9).contains(&n)).then_some(Action::SelectPreset(n));
            return (true, action);
        }

        let action = match (self.state, ev.down) {
            (State::Idle, true) if is_dictation => {
                self.state = State::Pressed {
                    since: now,
                    marked: false,
                };
                Some(Action::Start)
            }
            (State::Idle, _) => None,

            (State::Pressed { since, marked }, false) if is_dictation => {
                if marked || now.duration_since(since) >= self.timing.hold {
                    self.state = State::Idle;
                    Some(Action::Finish)
                } else if self.double_tap || self.dictation.is_mouse() {
                    self.state = State::TapReleased { at: now };
                    None
                } else {
                    // A tap is too short to be dictation, and there's no
                    // double-tap to wait for.
                    self.state = State::Idle;
                    Some(Action::Cancel { silent: true })
                }
            }
            (
                State::Pressed {
                    since,
                    marked: false,
                },
                true,
            ) if is_instruction => {
                self.state = State::Pressed {
                    since,
                    marked: true,
                };
                Some(Action::MarkInstruction)
            }
            // Any other key while the dictation key is held means the user
            // is typing a shortcut (⌥E for an accent, ⌥⌘…), not dictating.
            (State::Pressed { .. }, true) if !is_dictation && !is_instruction => {
                self.state = State::AwaitRelease;
                Some(Action::Cancel { silent: true })
            }
            (State::Pressed { .. }, _) => None,

            (State::TapReleased { at }, true) if is_dictation => {
                if now.duration_since(at) <= self.timing.double_tap {
                    self.state = State::HandsFree {
                        marked: false,
                        ignore_up: true,
                    };
                    Some(Action::HandsFree)
                } else {
                    // Too slow for a double-tap: the old tap was accidental
                    // and this is a fresh press. The session restarts.
                    self.state = State::Pressed {
                        since: now,
                        marked: false,
                    };
                    Some(Action::Start)
                }
            }
            // Typing right after an accidental tap.
            (State::TapReleased { .. }, true) => {
                self.state = State::Idle;
                Some(Action::Cancel { silent: true })
            }
            (State::TapReleased { .. }, false) => None,

            (State::HandsFree { marked, .. }, false) if is_dictation => {
                self.state = State::HandsFree {
                    marked,
                    ignore_up: false,
                };
                None
            }
            (State::HandsFree { .. }, true) if is_dictation => {
                self.state = State::AwaitRelease;
                Some(Action::Finish)
            }
            (
                State::HandsFree {
                    marked: false,
                    ignore_up,
                },
                true,
            ) if is_instruction => {
                self.state = State::HandsFree {
                    marked: true,
                    ignore_up,
                };
                Some(Action::MarkInstruction)
            }
            // Other keys don't affect hands-free recording.
            (State::HandsFree { .. }, _) => None,

            (State::AwaitRelease, false) if is_dictation => {
                self.state = State::Idle;
                None
            }
            (State::AwaitRelease, _) => None,
        };
        match action {
            Some(Action::Start) => {
                self.recording_since = Some(now);
                self.cancel_armed_until = None;
                self.send = false;
                self.processing_until = None;
            }
            Some(Action::Finish) => {
                self.recording_since = None;
                self.cancel_armed_until = None;
                self.processing_until = Some(now + self.timing.processing_limit);
            }
            Some(Action::Cancel { .. }) => {
                self.recording_since = None;
                self.cancel_armed_until = None;
                self.send = false;
            }
            _ if !self.is_recording() => {
                self.recording_since = None;
                self.cancel_armed_until = None;
            }
            _ => {}
        }
        // Only the dictation key gets here as a mouse button.
        (ev.key.is_mouse(), action)
    }

    /// Return while recording or processing toggles auto-send and is
    /// swallowed. `None`: not ours, handle it like any other key.
    fn on_return(&mut self, down: bool, now: Instant) -> Option<(bool, Option<Action>)> {
        if !down {
            self.return_held = false;
            return std::mem::take(&mut self.swallow_return_up).then_some((true, None));
        }
        let ours = matches!(self.state, State::Pressed { .. } | State::HandsFree { .. })
            || (!matches!(self.state, State::TapReleased { .. }) && self.processing(now));
        if !ours {
            return None;
        }
        self.swallow_return_up = true;
        if std::mem::replace(&mut self.return_held, true) {
            return Some((true, None)); // auto-repeat
        }
        self.send = !self.send;
        Some((true, Some(Action::ToggleSend(self.send))))
    }

    fn on_escape(&mut self, down: bool, now: Instant) -> (bool, Option<Action>) {
        if !down {
            let swallow = std::mem::take(&mut self.swallow_escape_up);
            return (swallow, None);
        }
        if !self.is_recording() {
            // Esc while the result is on its way calls off a pending send.
            if self.send && self.processing(now) {
                self.send = false;
                self.swallow_escape_up = true;
                return (true, Some(Action::ToggleSend(false)));
            }
            return (false, None);
        }
        self.swallow_escape_up = true;

        let recorded = self
            .recording_since
            .map_or(Duration::ZERO, |t| now.duration_since(t));
        let armed = self.cancel_armed_until.is_some_and(|t| now < t);
        if recorded >= self.timing.confirm_cancel_after && !armed {
            self.cancel_armed_until = Some(now + self.timing.confirm_window);
            return (
                true,
                Some(Action::ConfirmCancel {
                    seconds: recorded.as_secs(),
                }),
            );
        }
        self.end_recording();
        self.send = false;
        (true, Some(Action::Cancel { silent: false }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PhysicalKey::*;

    const DICT: PhysicalKey = RightAlt;

    struct Clock {
        r: Recognizer,
        t0: Instant,
        now: Instant,
    }

    impl Clock {
        fn new() -> Self {
            let t0 = Instant::now();
            Self {
                r: Recognizer::new(DICT, RightShift, Timing::default()),
                t0,
                now: t0,
            }
        }
        fn at(&mut self, ms: u64) -> &mut Self {
            self.now = self.t0 + Duration::from_millis(ms);
            self
        }
        fn down(&mut self, k: PhysicalKey) -> Option<Action> {
            self.r.on_event(KeyEvent::down(k), self.now).1
        }
        fn up(&mut self, k: PhysicalKey) -> Option<Action> {
            self.r.on_event(KeyEvent::up(k), self.now).1
        }
        fn tick(&mut self) -> Option<Action> {
            self.r.on_timeout(self.now)
        }
    }

    #[test]
    fn hold_and_release_finishes() {
        let mut c = Clock::new();
        assert_eq!(c.at(0).down(DICT), Some(Action::Start));
        assert!(c.r.is_recording());
        assert_eq!(c.at(1200).up(DICT), Some(Action::Finish));
        assert!(!c.r.is_recording());
    }

    #[test]
    fn short_tap_is_discarded_after_double_tap_window() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        assert_eq!(c.at(100).up(DICT), None);
        assert!(
            c.r.is_recording(),
            "keeps recording while waiting for a 2nd tap"
        );
        assert_eq!(c.r.deadline(), Some(c.t0 + Duration::from_millis(400)));
        assert_eq!(c.at(399).tick(), None);
        assert_eq!(c.at(400).tick(), Some(Action::Cancel { silent: true }));
        assert!(!c.r.is_recording());
    }

    #[test]
    fn hands_free_key_while_holding_latches() {
        let mut c = Clock::new();
        c.r = Recognizer::new(DICT, RightShift, Timing::default())
            .with_hands_free(Letter('h'), false);
        assert_eq!(c.at(0).down(DICT), Some(Action::Start));
        // H while holding: hands-free, and swallowed (down, repeat, up).
        assert_eq!(
            c.r.on_event(KeyEvent::down(Letter('h')), c.now),
            (true, Some(Action::HandsFree))
        );
        assert_eq!(
            c.r.on_event(KeyEvent::down(Letter('h')), c.now),
            (true, None)
        );
        assert_eq!(c.r.on_event(KeyEvent::up(Letter('h')), c.now), (true, None));
        // Letting go keeps recording; a tap finishes.
        assert_eq!(c.at(2000).up(DICT), None);
        assert!(c.r.is_recording());
        assert_eq!(c.at(5000).down(DICT), Some(Action::Finish));
        // Typing H later is untouched.
        c.at(6000).up(DICT);
        assert_eq!(
            c.r.on_event(KeyEvent::down(Letter('h')), c.now),
            (false, None)
        );
    }

    #[test]
    fn double_tap_can_be_turned_off() {
        let mut c = Clock::new();
        c.r = Recognizer::new(DICT, RightShift, Timing::default())
            .with_hands_free(Letter('h'), false);
        assert_eq!(c.at(0).down(DICT), Some(Action::Start));
        // A short tap is dropped at once instead of waiting for a second.
        assert_eq!(c.at(100).up(DICT), Some(Action::Cancel { silent: true }));
        assert!(!c.r.is_recording());
        // Mouse buttons still double-click into hands-free.
        let mut r = Recognizer::new(MouseBack, RightShift, Timing::default())
            .with_hands_free(Letter('h'), false);
        let t = Instant::now();
        r.on_event(KeyEvent::down(MouseBack), t);
        r.on_event(KeyEvent::up(MouseBack), t + Duration::from_millis(80));
        assert_eq!(
            r.on_event(KeyEvent::down(MouseBack), t + Duration::from_millis(200))
                .1,
            Some(Action::HandsFree)
        );
    }

    #[test]
    fn double_tap_latches_hands_free_and_tap_finishes() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        c.at(80).up(DICT);
        assert_eq!(c.at(200).down(DICT), Some(Action::HandsFree));
        assert_eq!(c.at(260).up(DICT), None, "second tap's key-up is ignored");
        assert!(c.r.is_hands_free());
        assert_eq!(c.at(5_000).down(LeftShift), None, "typing doesn't stop it");
        assert_eq!(c.at(9_000).down(DICT), Some(Action::Finish));
        assert_eq!(c.at(9_080).up(DICT), None);
        assert_eq!(c.at(9_500).down(DICT), Some(Action::Start), "back to idle");
    }

    #[test]
    fn instruction_tap_marks_once() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        assert_eq!(c.at(900).down(RightShift), Some(Action::MarkInstruction));
        assert_eq!(c.at(950).up(RightShift), None);
        assert_eq!(
            c.at(1_100).down(RightShift),
            None,
            "only the first tap counts"
        );
        assert_eq!(c.at(2_000).up(DICT), Some(Action::Finish));
    }

    #[test]
    fn marked_short_press_still_finishes() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        c.at(50).down(RightShift);
        assert_eq!(c.at(150).up(DICT), Some(Action::Finish));
    }

    #[test]
    fn chord_cancels_silently_and_waits_for_release() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        assert_eq!(c.at(120).down(Other), Some(Action::Cancel { silent: true }));
        assert!(!c.r.is_recording());
        assert_eq!(c.at(300).up(DICT), None);
        assert_eq!(c.at(400).down(DICT), Some(Action::Start));
    }

    #[test]
    fn typing_after_accidental_tap_cancels() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        c.at(60).up(DICT);
        assert_eq!(c.at(150).down(Other), Some(Action::Cancel { silent: true }));
        assert!(!c.r.is_recording());
    }

    #[test]
    fn slow_second_press_restarts() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        c.at(60).up(DICT);
        // Deadline tick missed, but the next press still behaves.
        assert_eq!(c.at(800).down(DICT), Some(Action::Start));
        assert_eq!(c.at(2_000).up(DICT), Some(Action::Finish));
    }

    #[test]
    fn escape_cancels_and_is_swallowed_only_while_recording() {
        let mut c = Clock::new();
        assert_eq!(
            c.r.on_event(KeyEvent::down(Escape), c.now),
            (false, None),
            "idle Esc passes through"
        );
        c.at(0).down(DICT);
        assert_eq!(
            c.r.on_event(KeyEvent::down(Escape), c.now),
            (true, Some(Action::Cancel { silent: false }))
        );
        assert_eq!(c.r.on_event(KeyEvent::up(Escape), c.now), (true, None));
        assert_eq!(c.at(500).up(DICT), None, "release after Esc does nothing");
        assert_eq!(c.r.on_event(KeyEvent::up(Escape), c.now), (false, None));
    }

    #[test]
    fn long_recording_asks_before_discarding() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        c.at(50).up(DICT);
        c.at(100).down(DICT); // hands-free
        c.at(150).up(DICT);
        let (swallow, action) =
            c.r.on_event(KeyEvent::down(Escape), c.t0 + Duration::from_secs(40));
        assert!(swallow);
        assert_eq!(action, Some(Action::ConfirmCancel { seconds: 40 }));
        assert!(c.r.is_recording(), "still recording while asking");

        // Second Esc within the window discards.
        let (_, action) =
            c.r.on_event(KeyEvent::down(Escape), c.t0 + Duration::from_secs(41));
        assert_eq!(action, Some(Action::Cancel { silent: false }));
        assert!(!c.r.is_recording());
    }

    #[test]
    fn unanswered_confirmation_keeps_recording() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        let t = c.t0 + Duration::from_secs(35);
        let (_, action) = c.r.on_event(KeyEvent::down(Escape), t);
        assert_eq!(action, Some(Action::ConfirmCancel { seconds: 35 }));
        assert_eq!(c.r.deadline(), Some(t + Duration::from_millis(2500)));
        assert_eq!(
            c.r.on_timeout(t + Duration::from_secs(3)),
            Some(Action::KeepRecording)
        );
        assert!(c.r.is_recording());
        assert_eq!(c.r.deadline(), None);
        // Releasing the key still finishes normally.
        assert_eq!(c.at(36_000).up(DICT), Some(Action::Finish));
    }

    #[test]
    fn escape_cancels_hands_free() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        c.at(50).up(DICT);
        c.at(100).down(DICT);
        c.at(150).up(DICT);
        let (swallow, action) = c.r.on_event(KeyEvent::down(Escape), c.now);
        assert!(swallow);
        assert_eq!(action, Some(Action::Cancel { silent: false }));
        assert_eq!(c.at(300).down(DICT), Some(Action::Start));
    }

    #[test]
    fn external_finish() {
        let mut c = Clock::new();
        assert_eq!(c.r.finish(), None);
        c.at(0).down(DICT);
        c.at(50).up(DICT);
        c.at(100).down(DICT);
        assert_eq!(c.r.finish(), Some(Action::Finish));
        assert!(!c.r.is_recording());
    }

    #[test]
    fn digits_while_holding_select_presets() {
        let mut c = Clock::new();
        c.at(0).down(DICT);
        let (swallow, action) = c.r.on_event(KeyEvent::down(Digit(2)), c.now);
        assert!(swallow);
        assert_eq!(action, Some(Action::SelectPreset(2)));
        assert_eq!(c.r.on_event(KeyEvent::up(Digit(2)), c.now), (true, None));
        assert!(c.r.is_recording(), "a digit is not a chord-cancel");
        assert_eq!(c.at(1_000).up(DICT), Some(Action::Finish));
        // When idle, digits are just typing.
        assert_eq!(c.r.on_event(KeyEvent::down(Digit(2)), c.now), (false, None));
    }

    #[test]
    fn return_toggles_send_while_recording_and_processing() {
        let mut c = Clock::new();
        assert_eq!(
            c.r.on_event(KeyEvent::down(Return), c.now),
            (false, None),
            "idle Return passes through"
        );
        c.r.on_event(KeyEvent::up(Return), c.now);

        c.down(DICT);
        assert_eq!(
            c.r.on_event(KeyEvent::down(Return), c.now),
            (true, Some(Action::ToggleSend(true)))
        );
        // Auto-repeat while held doesn't toggle back.
        assert_eq!(c.r.on_event(KeyEvent::down(Return), c.now), (true, None));
        assert_eq!(c.r.on_event(KeyEvent::up(Return), c.now), (true, None));
        assert!(c.r.is_recording(), "Return is not a chord-cancel");
        assert_eq!(c.at(800).up(DICT), Some(Action::Finish));

        // While processing: a second press turns it off, a third on again.
        let t = c.now + Duration::from_millis(200);
        assert_eq!(
            c.r.on_event(KeyEvent::down(Return), t),
            (true, Some(Action::ToggleSend(false)))
        );
        c.r.on_event(KeyEvent::up(Return), t);
        assert_eq!(
            c.r.on_event(KeyEvent::down(Return), t),
            (true, Some(Action::ToggleSend(true)))
        );
        c.r.on_event(KeyEvent::up(Return), t);
        assert!(c.r.take_send());
        assert!(!c.r.take_send(), "taken once");

        // Once the result is in, Return is the app's again.
        assert_eq!(c.r.on_event(KeyEvent::down(Return), t), (false, None));
    }

    #[test]
    fn send_resets_and_esc_calls_it_off() {
        let mut c = Clock::new();
        c.down(DICT);
        c.r.on_event(KeyEvent::down(Return), c.now);
        c.r.on_event(KeyEvent::up(Return), c.now);
        c.at(800).up(DICT);
        assert_eq!(
            c.r.on_event(KeyEvent::down(Escape), c.now),
            (true, Some(Action::ToggleSend(false)))
        );
        assert!(!c.r.take_send());

        // A new recording starts with send off.
        c.r.on_event(KeyEvent::up(Escape), c.now);
        c.at(2000).down(DICT);
        c.r.on_event(KeyEvent::down(Return), c.now);
        c.at(2100).down(Escape);
        assert!(!c.r.take_send(), "canceled recordings never send");

        // The processing window expires on its own.
        let mut c = Clock::new();
        c.down(DICT);
        c.at(800).up(DICT);
        let late = c.now + Duration::from_secs(31);
        assert_eq!(c.r.on_event(KeyEvent::down(Return), late), (false, None));
    }

    #[test]
    fn key_names_parse() {
        #[derive(Deserialize)]
        struct K {
            k: PhysicalKey,
        }
        let parse = |s: &str| toml::from_str::<K>(&format!("k = \"{s}\"")).unwrap().k;
        assert_eq!(parse("right-option"), RightAlt);
        assert_eq!(parse("right-alt"), RightAlt);
        assert_eq!(parse("right-command"), RightMeta);
        assert_eq!(parse("fn"), Fn);
        assert_eq!(parse("mouse-4"), MouseBack);
        assert_eq!(parse("mouse-5"), MouseForward);
        assert!(toml::from_str::<K>("k = \"mouse-3\"").is_err());
        assert_eq!(
            toml::to_string(&K2 { k: MouseForward }).unwrap().trim(),
            "k = \"mouse-5\""
        );

        #[derive(Serialize)]
        struct K2 {
            k: PhysicalKey,
        }
    }

    #[test]
    fn mouse_button_as_dictation_key_is_swallowed() {
        let mut r = Recognizer::new(MouseBack, RightShift, Timing::default());
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        assert_eq!(
            r.on_event(KeyEvent::down(MouseBack), at(0)),
            (true, Some(Action::Start))
        );
        // The other side button is the app's, and doesn't chord-cancel.
        assert_eq!(
            r.on_event(KeyEvent::down(MouseForward), at(100)),
            (false, None)
        );
        assert_eq!(
            r.on_event(KeyEvent::up(MouseForward), at(150)),
            (false, None)
        );
        assert!(r.is_recording());
        assert_eq!(
            r.on_event(KeyEvent::up(MouseBack), at(900)),
            (true, Some(Action::Finish)),
            "hold-to-talk works like a key"
        );
        // Double-click latches hands-free, as with a key.
        r.on_event(KeyEvent::down(MouseBack), at(2_000));
        r.on_event(KeyEvent::up(MouseBack), at(2_080));
        assert_eq!(
            r.on_event(KeyEvent::down(MouseBack), at(2_200)),
            (true, Some(Action::HandsFree))
        );
        assert_eq!(r.on_event(KeyEvent::up(MouseBack), at(2_260)), (true, None));
    }

    #[test]
    fn mouse_buttons_pass_through_when_not_the_dictation_key() {
        let mut c = Clock::new();
        assert_eq!(
            c.r.on_event(KeyEvent::down(MouseBack), c.now),
            (false, None)
        );
        c.at(0).down(DICT);
        assert_eq!(
            c.r.on_event(KeyEvent::down(MouseBack), c.now),
            (false, None)
        );
        assert!(c.r.is_recording(), "a mouse click isn't a chord");
    }

    #[test]
    fn external_hands_free_start_cancel_and_finish() {
        let mut c = Clock::new();
        assert_eq!(c.r.cancel(), None, "nothing to cancel");
        assert_eq!(c.r.start_hands_free(c.now), Some(Action::Start));
        assert!(c.r.is_hands_free());
        assert_eq!(c.r.start_hands_free(c.now), None, "already recording");
        // A tap of the dictation key finishes it.
        assert_eq!(c.at(3_000).down(DICT), Some(Action::Finish));
        assert_eq!(c.at(3_100).up(DICT), None);

        c.r.start_hands_free(c.now);
        assert_eq!(c.r.cancel(), Some(Action::Cancel { silent: false }));
        assert!(!c.r.is_recording());
        assert_eq!(c.at(4_000).down(DICT), Some(Action::Start), "idle again");
        c.at(5_000).up(DICT);

        // Esc cancels an external recording like any other.
        c.r.start_hands_free(c.now);
        let (swallow, action) = c.r.on_event(KeyEvent::down(Escape), c.now);
        assert!(swallow);
        assert_eq!(action, Some(Action::Cancel { silent: false }));

        // Started while the key is still held after a chord-cancel: the
        // key-up is ignored and the next tap finishes.
        let mut c = Clock::new();
        c.at(0).down(DICT);
        c.at(100).down(Other);
        assert_eq!(c.r.start_hands_free(c.now), Some(Action::Start));
        assert_eq!(c.at(200).up(DICT), None);
        assert!(c.r.is_recording());
        assert_eq!(c.at(900).down(DICT), Some(Action::Finish));
    }
}
