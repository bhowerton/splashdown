//! What Splashdown shows on screen outside its windows: the status pill
//! (`Overlay`, `OverlayState`) and the mini bar, as the session drives
//! them; `mac/` and `win/` draw them.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Warning,
}

/// What the status pill shows.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayState {
    /// `preset` names a non-default preset in use ("Listening · Email").
    Listening {
        preset: Option<String>,
    },
    Instruction,
    Transform {
        words: usize,
    },
    HandsFree {
        preset: Option<String>,
    },
    /// Hands-free recording of the answer to a question (from a coding
    /// agent): shows the question.
    Question {
        text: String,
    },
    Working {
        label: String,
    },
    /// Text being read aloud.
    Reading {
        label: String,
    },
    Message {
        tone: Tone,
        text: String,
    },
}

/// The floating status pill. Never takes focus and ignores the mouse.
/// Create and use it on the main (UI) thread only.
pub struct Overlay(imp::Overlay);

impl Overlay {
    pub fn new() -> Result<Self> {
        imp::Overlay::new().map(Self)
    }

    pub fn show(&self, state: &OverlayState) {
        self.0.show(state);
    }

    /// Feeds the live microphone level (0.0–1.0) to the waveform.
    pub fn set_level(&self, level: f32) {
        self.0.set_level(level);
    }

    /// Replaces the label with the words heard so far (only while a
    /// recording state is shown); `None` restores the state's label.
    pub fn set_live_text(&self, text: Option<&str>) {
        self.0.set_live_text(text);
    }

    /// Shows "↵ Send" after the label while auto-send is on. Cleared by
    /// the next recording, any message, or hiding.
    pub fn set_send(&self, on: bool) {
        self.0.set_send(on);
    }

    /// Shows the overlay just below `caret` (or above it near the bottom of
    /// the screen) instead of at the bottom of the screen. `None` restores
    /// the usual place.
    pub fn set_anchor(&self, caret: Option<CaretRect>) {
        self.0.set_anchor(caret);
    }

    pub fn hide(&self) {
        self.0.hide();
    }

    /// Opacity 0.0–1.0, for fades.
    pub fn set_alpha(&self, alpha: f64) {
        self.0.set_alpha(alpha);
    }

    /// Whether the pill is on screen.
    pub fn is_visible(&self) -> bool {
        self.0.is_visible()
    }

    /// Speaks `text` with VoiceOver (if it's running).
    pub fn announce(&self, text: &str) {
        self.0.announce(text);
    }
}

/// What the mini bar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiniBarState {
    Idle,
    Recording,
    /// The recording is being transcribed and cleaned up.
    Working,
    /// Dictation isn't set up (permission missing, no provider).
    NotReady,
}

/// Where the mini bar is, in the platform's own screen coordinates (the
/// bottom-left corner in AppKit points on macOS, the top-left corner in
/// physical pixels on Windows). Only meant to be saved and handed back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiniBarPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MiniBarEvent {
    /// Pressed and released without dragging.
    Click,
    /// Dragged somewhere else; save this to put it back there next time.
    Moved(MiniBarPosition),
    /// An item from its right-click menu, by index in the list given to
    /// `MiniBar::new`.
    Menu(usize),
}

/// Called on the main thread for each event; must not block.
pub type MiniBarHandler = Box<dyn Fn(MiniBarEvent) + 'static>;

/// A small round button that stays on screen, for starting and finishing
/// dictation with the mouse. Unlike the overlay it takes clicks, but it
/// still never takes focus: clicking it leaves the user's app active and
/// its text field focused. It can be dragged anywhere. Create and use it on
/// the main (UI) thread only.
pub struct MiniBar(imp::MiniBar);

impl MiniBar {
    /// Creates the bar, hidden, at `position` (or a default place by the
    /// right edge of the screen when it's unset or no longer on a screen).
    /// `menu` lists the right-click menu's items (macOS only for now).
    pub fn new(
        position: Option<MiniBarPosition>,
        menu: &[&str],
        handler: MiniBarHandler,
    ) -> Result<Self> {
        imp::MiniBar::new(position, menu, handler).map(Self)
    }

    pub fn show(&self) {
        self.0.show();
    }

    pub fn hide(&self) {
        self.0.hide();
    }

    pub fn set_state(&self, state: MiniBarState) {
        self.0.set_state(state);
    }

    /// The hint shown when the pointer rests on the bar, also read out by
    /// screen readers.
    pub fn set_tooltip(&self, text: &str) {
        self.0.set_tooltip(text);
    }
}
