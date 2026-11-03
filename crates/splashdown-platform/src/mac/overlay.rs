//! The status pill: a borderless, non-activating NSPanel that floats above
//! everything (including full-screen apps), ignores the mouse, and never
//! becomes key, so the user's focused field keeps focus.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibilityAnnouncementKey, NSAccessibilityAnnouncementRequestedNotification,
    NSAccessibilityPostNotificationWithUserInfo, NSBackingStoreType, NSBox, NSBoxType, NSColor,
    NSControlSize, NSFont, NSFontWeightMedium, NSPanel, NSProgressIndicator,
    NSProgressIndicatorStyle, NSScreen, NSSound, NSStatusWindowLevel, NSTextField, NSTitlePosition,
    NSView, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

use crate::{OverlayState, PlatformError, Result, Sound, Tone};

/// Space between the text cursor and the overlay.
const CARET_GAP: f64 = 12.0;
/// The least a caret's line is taken to be: some apps (a web page's text
/// box, Notes) report the caret as a sliver at the top of its line, and the
/// pill would cover the line being typed.
const MIN_LINE: f64 = 24.0;
const HEIGHT: f64 = 38.0;
const PAD_X: f64 = 14.0;
const DOT: f64 = 10.0;
const GAP: f64 = 10.0;
const BARS: usize = 9;
const BAR_W: f64 = 3.0;
const BAR_GAP: f64 = 2.5;
const BAR_MAX_H: f64 = 18.0;
const BAR_MIN_H: f64 = 3.0;
/// Distance from the bottom of the screen's visible area.
const BOTTOM_OFFSET: f64 = 72.0;

#[derive(Clone, Copy)]
struct Rgb(f64, f64, f64);

const RED: Rgb = Rgb(0.94, 0.27, 0.27);
const PURPLE: Rgb = Rgb(0.58, 0.45, 0.98);
const BLUE: Rgb = Rgb(0.45, 0.58, 0.98);
const GREEN: Rgb = Rgb(0.32, 0.81, 0.40);
const AMBER: Rgb = Rgb(0.98, 0.69, 0.02);
const GRAY: Rgb = Rgb(0.70, 0.70, 0.72);

fn color(c: Rgb, alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.0, c.1, c.2, alpha)
}

fn rounded_box(mtm: MainThreadMarker, radius: f64, fill: &NSColor) -> Retained<NSBox> {
    let b = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
    b.setBoxType(NSBoxType::Custom);
    b.setTitlePosition(NSTitlePosition::NoTitle);
    b.setBorderWidth(0.0);
    b.setCornerRadius(radius);
    b.setContentViewMargins(NSSize::ZERO);
    b.setFillColor(fill);
    b
}

pub(crate) struct Overlay {
    mtm: MainThreadMarker,
    panel: Retained<NSPanel>,
    background: Retained<NSBox>,
    dot: Retained<NSBox>,
    /// Shown in place of the dot while working.
    spinner: Retained<NSProgressIndicator>,
    label: Retained<NSTextField>,
    bars: Vec<Retained<NSBox>>,
    levels: RefCell<VecDeque<f32>>,
    /// Waveform color, or `None` when the current state has no waveform.
    wave: RefCell<Option<Rgb>>,
    /// The current state's own label, restored when live text is cleared.
    base_label: RefCell<String>,
    /// Live text replacing the label, if any.
    live: RefCell<Option<String>>,
    /// Auto-send is on: "↵ Send" follows the label.
    send: Cell<bool>,
    /// Shown near this text cursor rather than at the bottom of the screen.
    anchor: Cell<Option<crate::CaretRect>>,
}

/// Appended to the label while auto-send is on.
const SEND_BADGE: &str = "  ·  ↵ Send";

/// Live text longer than this shows only its end ("…on Friday").
const LIVE_TEXT_CHARS: usize = 54;
/// A question longer than this is cut short.
const QUESTION_CHARS: usize = 70;

impl Overlay {
    pub(crate) fn new() -> Result<Self> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            PlatformError::Failed("overlay must be created on the main thread".into())
        })?;

        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(200.0, HEIGHT)),
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setLevel(NSStatusWindowLevel);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setIgnoresMouseEvents(true);
        panel.setFloatingPanel(true);
        panel.setHidesOnDeactivate(false);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        // SAFETY: on the main thread (`mtm`). Turning this off is what makes
        // owning the panel through `Retained` sound: closing it won't also
        // release it behind our back.
        unsafe { panel.setReleasedWhenClosed(false) };

        let content = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        let background = rounded_box(
            mtm,
            HEIGHT / 2.0,
            &NSColor::colorWithSRGBRed_green_blue_alpha(0.09, 0.09, 0.11, 0.92),
        );
        content.addSubview(&background);

        let dot = rounded_box(mtm, DOT / 2.0, &color(RED, 1.0));
        content.addSubview(&dot);

        let spinner =
            NSProgressIndicator::initWithFrame(NSProgressIndicator::alloc(mtm), NSRect::ZERO);
        spinner.setStyle(NSProgressIndicatorStyle::Spinning);
        spinner.setControlSize(NSControlSize::Small);
        spinner.setDisplayedWhenStopped(false);
        content.addSubview(&spinner);

        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        // SAFETY: reads an immutable constant exported by AppKit.
        let weight = unsafe { NSFontWeightMedium };
        label.setFont(Some(&NSFont::systemFontOfSize_weight(13.5, weight)));
        label.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.96, 0.96, 0.96, 1.0,
        )));
        content.addSubview(&label);

        let bars = (0..BARS)
            .map(|_| {
                let b = rounded_box(mtm, BAR_W / 2.0, &color(RED, 1.0));
                content.addSubview(&b);
                b
            })
            .collect();

        panel.setContentView(Some(&content));

        Ok(Self {
            mtm,
            panel,
            background,
            dot,
            spinner,
            label,
            bars,
            levels: RefCell::new(VecDeque::from(vec![0.0; BARS])),
            wave: RefCell::new(None),
            base_label: RefCell::new(String::new()),
            live: RefCell::new(None),
            send: Cell::new(false),
            anchor: Cell::new(None),
        })
    }

    pub(crate) fn show(&self, state: &OverlayState) {
        let (dot, text, wave) = match state {
            OverlayState::Listening { preset } => {
                (RED, with_preset("Listening", preset), Some(RED))
            }
            OverlayState::Instruction => (PURPLE, "Instruction".to_string(), Some(PURPLE)),
            OverlayState::Transform { words } => (
                BLUE,
                format!(
                    "Transform · {words} word{}",
                    if *words == 1 { "" } else { "s" }
                ),
                Some(BLUE),
            ),
            OverlayState::HandsFree { preset } => (
                RED,
                with_preset("Hands-free · tap to finish", preset),
                Some(RED),
            ),
            OverlayState::Question { text } => (
                RED,
                format!("{} · tap to answer", head_chars(text, QUESTION_CHARS)),
                Some(RED),
            ),
            OverlayState::Working { label } => (BLUE, label.clone(), None),
            OverlayState::Reading { label } => (GREEN, label.clone(), None),
            OverlayState::Message { tone, text } => {
                let c = match tone {
                    Tone::Info => GRAY,
                    Tone::Success => GREEN,
                    Tone::Warning => AMBER,
                };
                (c, text.clone(), None)
            }
        };

        let working = matches!(state, OverlayState::Working { .. });
        self.dot.setHidden(working);
        // SAFETY: `self` holds a MainThreadMarker, so we're on the main
        // thread; the sender argument is unused and may be nil.
        unsafe {
            if working {
                self.spinner.startAnimation(None);
            } else {
                self.spinner.stopAnimation(None);
            }
        }
        self.dot.setFillColor(&color(dot, 1.0));
        // A new recording or a final message ends any auto-send badge.
        if matches!(
            state,
            OverlayState::Listening { .. } | OverlayState::Message { .. }
        ) {
            self.send.set(false);
        }
        *self.base_label.borrow_mut() = text;
        *self.live.borrow_mut() = None;
        self.refresh_label();
        *self.wave.borrow_mut() = wave;
        if let Some(c) = wave {
            let fill = color(c, 0.95);
            for b in &self.bars {
                b.setFillColor(&fill);
            }
        }
        self.layout();
        self.panel.orderFrontRegardless();
    }

    pub(crate) fn set_level(&self, level: f32) {
        {
            let mut levels = self.levels.borrow_mut();
            levels.pop_front();
            levels.push_back(level);
        }
        if self.wave.borrow().is_some() {
            self.layout_bars(self.bars_x());
        }
    }

    pub(crate) fn set_live_text(&self, text: Option<&str>) {
        if self.wave.borrow().is_none() {
            return;
        }
        *self.live.borrow_mut() = text
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| tail_chars(t, LIVE_TEXT_CHARS));
        self.refresh_label();
        self.layout();
    }

    pub(crate) fn set_send(&self, on: bool) {
        self.send.set(on);
        self.refresh_label();
        self.layout();
    }

    /// Live text or the state's label, plus the auto-send badge.
    fn refresh_label(&self) {
        let mut text = match &*self.live.borrow() {
            Some(live) => live.clone(),
            None => self.base_label.borrow().clone(),
        };
        if self.send.get() {
            text.push_str(SEND_BADGE);
        }
        self.label.setStringValue(&NSString::from_str(&text));
    }

    pub(crate) fn set_alpha(&self, alpha: f64) {
        self.panel.setAlphaValue(alpha.clamp(0.0, 1.0));
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.panel.isVisible()
    }

    pub(crate) fn announce(&self, text: &str) {
        let message = NSString::from_str(text);
        // SAFETY: reads an immutable constant exported by AppKit.
        let key = unsafe { NSAccessibilityAnnouncementKey };
        let info: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(&[key], &[message.as_ref()]);
        // SAFETY: on the main thread (`self.mtm`); the element is our live
        // panel, the notification name an AppKit constant, and the user info
        // has the shape this notification expects (announcement key mapped
        // to an NSString).
        unsafe {
            NSAccessibilityPostNotificationWithUserInfo(
                self.panel.as_ref(),
                NSAccessibilityAnnouncementRequestedNotification,
                Some(&info),
            );
        }
    }

    pub(crate) fn hide(&self) {
        self.send.set(false);
        // SAFETY: on the main thread (`self.mtm`); the sender may be nil.
        unsafe { self.spinner.stopAnimation(None) };
        self.panel.orderOut(None);
        self.levels.borrow_mut().iter_mut().for_each(|l| *l = 0.0);
    }

    fn label_width(&self) -> f64 {
        self.label.fittingSize().width.ceil()
    }

    fn bars_x(&self) -> f64 {
        PAD_X + DOT + GAP + self.label_width() + GAP + 2.0
    }

    fn layout(&self) {
        let has_wave = self.wave.borrow().is_some();
        let label_w = self.label_width();
        let wave_w = if has_wave {
            GAP + 2.0 + BARS as f64 * (BAR_W + BAR_GAP) - BAR_GAP
        } else {
            0.0
        };
        let width = PAD_X + DOT + GAP + label_w + wave_w + PAD_X;

        // Near the text cursor when known, else bottom center of the screen
        // with the focused window.
        let origin = self
            .anchor
            .get()
            .and_then(|caret| self.origin_near(caret, width));
        let origin = origin.or_else(|| {
            NSScreen::mainScreen(self.mtm).map(|screen| {
                let vis = screen.visibleFrame();
                NSPoint::new(
                    (vis.origin.x + (vis.size.width - width) / 2.0).round(),
                    vis.origin.y + BOTTOM_OFFSET,
                )
            })
        });
        if let Some(origin) = origin {
            self.panel
                .setFrame_display(NSRect::new(origin, NSSize::new(width, HEIGHT)), true);
        }

        self.background
            .setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, HEIGHT)));
        self.dot.setFrame(NSRect::new(
            NSPoint::new(PAD_X, (HEIGHT - DOT) / 2.0),
            NSSize::new(DOT, DOT),
        ));
        const SPIN: f64 = 14.0;
        self.spinner.setFrame(NSRect::new(
            NSPoint::new(PAD_X - (SPIN - DOT) / 2.0, (HEIGHT - SPIN) / 2.0),
            NSSize::new(SPIN, SPIN),
        ));
        let label_h = self.label.fittingSize().height.ceil();
        self.label.setFrame(NSRect::new(
            NSPoint::new(PAD_X + DOT + GAP, ((HEIGHT - label_h) / 2.0).round()),
            NSSize::new(label_w, label_h),
        ));

        for b in &self.bars {
            b.setHidden(!has_wave);
        }
        if has_wave {
            self.layout_bars(self.bars_x());
        }
    }

    /// Where the panel goes for a caret: just below it, or above it when
    /// there's no room below, kept on the caret's screen. `None` if the
    /// caret isn't on any screen.
    fn origin_near(&self, caret: crate::CaretRect, width: f64) -> Option<NSPoint> {
        let screens = NSScreen::screens(self.mtm);
        // Accessibility uses a top-left origin on the primary display;
        // AppKit a bottom-left one.
        let primary_height = screens.firstObject()?.frame().size.height;
        let caret_top = primary_height - caret.y;
        let caret_bottom = caret_top - caret.height.max(MIN_LINE);
        let point = NSPoint::new(caret.x, caret_bottom);
        let screen = screens.iter().find(|s| {
            let f = s.frame();
            point.x >= f.origin.x
                && point.x <= f.origin.x + f.size.width
                && point.y >= f.origin.y
                && point.y <= f.origin.y + f.size.height
        })?;
        let vis = screen.visibleFrame();
        let below = caret_bottom - CARET_GAP - HEIGHT;
        let y = if below >= vis.origin.y {
            below
        } else {
            caret_top + CARET_GAP
        };
        let x = (caret.x - PAD_X)
            .min(vis.origin.x + vis.size.width - width)
            .max(vis.origin.x);
        Some(NSPoint::new(x.round(), y.round()))
    }

    pub(crate) fn set_anchor(&self, caret: Option<crate::CaretRect>) {
        self.anchor.set(caret);
    }

    fn layout_bars(&self, x0: f64) {
        let levels = self.levels.borrow();
        for (i, (bar, level)) in self.bars.iter().zip(levels.iter()).enumerate() {
            // Speech RMS is roughly 0.01–0.2; a square-root curve makes quiet
            // speech visible without loud speech pinning the bars.
            let norm = (f64::from(*level) * 25.0).sqrt().min(1.0);
            let h = (BAR_MIN_H + norm * (BAR_MAX_H - BAR_MIN_H)).round();
            let x = x0 + i as f64 * (BAR_W + BAR_GAP);
            bar.setFrame(NSRect::new(
                NSPoint::new(x, ((HEIGHT - h) / 2.0).round()),
                NSSize::new(BAR_W, h),
            ));
        }
    }
}

fn with_preset(label: &str, preset: &Option<String>) -> String {
    match preset {
        Some(p) => format!("{label} · {p}"),
        None => label.to_string(),
    }
}

/// Quiet system sounds: "Tink" when recording starts, "Pop" when it stops.
pub(crate) fn play_sound(sound: Sound, volume: f32) {
    let name = match sound {
        Sound::Start => "Tink",
        Sound::Stop => "Pop",
    };
    if let Some(s) = NSSound::soundNamed(&NSString::from_str(name)) {
        s.setVolume(volume.clamp(0.0, 1.0));
        s.play();
    }
}

pub(crate) fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// The first `max` characters of `text` on one line, with an ellipsis
/// when anything was cut.
fn head_chars(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", text[..i].trim_end()),
        None => text,
    }
}

/// The last `max` characters of `text`, starting at a word boundary, with a
/// leading ellipsis when anything was cut.
fn tail_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let start = text.char_indices().nth(count - max).map_or(0, |(i, _)| i);
    let tail = &text[start..];
    let tail = tail.split_once(' ').map_or(tail, |(_, rest)| rest);
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::{head_chars, tail_chars};

    #[test]
    fn head_is_one_short_line() {
        assert_eq!(head_chars("Tabs\nor spaces?", 70), "Tabs or spaces?");
        assert_eq!(head_chars("abcdef ghij", 7), "abcdef…");
    }

    #[test]
    fn tail_keeps_whole_words() {
        assert_eq!(tail_chars("short", 10), "short");
        assert_eq!(
            tail_chars("so I think we should ship it on Friday", 16),
            "…it on Friday"
        );
    }
}
