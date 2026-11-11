//! The mini bar: a small pill with a microphone and a short sound wave
//! that stays on screen, for starting and finishing dictation with the
//! mouse. Both turn orange under the pointer; the wave becomes the sound's
//! level while it listens, and a row of dots that sweeps while it works.
//!
//! Unlike the overlay it takes clicks, but it must never take focus, or the
//! text would have nowhere to go. Three things make sure of that:
//! - the panel is created with `NonactivatingPanel`, so clicking it never
//!   activates Splashdown (and the app is never activated in code);
//! - it's borderless, so it can't become the key window, and
//!   `becomesKeyOnlyIfNeeded` with no view that needs it adds a second
//!   guard;
//! - the view takes the first click (`acceptsFirstMouse:`) and handles
//!   the mouse itself, so the click acts at once instead of first trying
//!   to bring the window forward.

use std::cell::{Cell, OnceCell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityButtonRole, NSBackingStoreType, NSBox, NSBoxType, NSColor,
    NSEvent, NSEventModifierFlags, NSImage, NSImageScaling, NSImageView, NSMenu, NSMenuItem,
    NSPanel, NSScreen, NSStatusWindowLevel, NSTitlePosition, NSTrackingArea, NSTrackingAreaOptions,
    NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::{MiniBarEvent, MiniBarHandler, MiniBarPosition, MiniBarState, PlatformError, Result};

/// The pill.
const WIDTH: f64 = 50.0;
const HEIGHT: f64 = 16.0;
/// The microphone's frame (the symbol is drawn a little inside it), and
/// the space between it and the wave.
const MIC_W: f64 = 9.0;
const MIC_H: f64 = 12.0;
const SPACE: f64 = 3.0;
/// The wave's bars: how many, how wide, and the space between them.
const BARS: usize = 5;
const BAR_W: f64 = 2.0;
const GAP: f64 = 2.0;
/// How tall each bar is at rest: the shape of a sound wave.
const REST: [f64; BARS] = [4.0, 8.0, 5.0, 7.0, 3.0];
/// A bar as the sound's level, while it listens.
const BAR_MIN_H: f64 = 3.0;
const BAR_MAX_H: f64 = 11.0;
/// How much of the level each bar shows: tallest in the middle.
const ENVELOPE: [f64; BARS] = [0.55, 0.8, 1.0, 0.8, 0.55];
/// A bar as a dot of the sweep, while it works.
const DOT_H: f64 = 3.0;
/// Seconds between the steps of the sweep that shows it's working.
const SWEEP: f64 = 0.11;
/// The mouse must move this far before a press counts as a drag.
const DRAG_SLOP: f64 = 3.0;
/// Default place: the bottom middle of the screen, this far above the Dock.
const BOTTOM_MARGIN: f64 = 10.0;

/// What's drawn: the pill, its microphone and the bars of its wave.
struct Parts {
    background: Retained<NSBox>,
    /// `None` if the system has no such symbol: then it's the wave alone.
    mic: Option<Retained<NSImageView>>,
    bars: Vec<Retained<NSBox>>,
}

struct Ivars {
    handler: MiniBarHandler,
    /// Where the mouse and the panel were when the button went down.
    press: Cell<Option<(NSPoint, NSPoint)>>,
    dragged: Cell<bool>,
    menu: OnceCell<Retained<NSMenu>>,
    parts: OnceCell<Parts>,
    state: Cell<MiniBarState>,
    /// The pointer is over the pill.
    hover: Cell<bool>,
    /// The last levels heard (0–1), newest last: the middle bar shows the
    /// newest, the ones beside it the ones before.
    levels: RefCell<[f32; 3]>,
    /// Where the sweep is, while it works.
    step: Cell<usize>,
    /// A step of the sweep is waiting to run.
    sweeping: Cell<bool>,
}

define_class!(
    // SAFETY: NSView has no special subclassing requirements; we only
    // override event and accessibility methods, and don't implement Drop.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SplashdownMiniBarView"]
    #[ivars = Ivars]
    struct BarView;

    impl BarView {
        /// Act on the first click even though our window isn't key.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// We move the panel ourselves, so a click can't also drag it.
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        /// Every click inside lands here, not on the pill or a part of it.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            // SAFETY: calls NSView's own hitTest: with its own argument.
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            hit.map(|_| self.retain().into_super())
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if event.modifierFlags().contains(NSEventModifierFlags::Control) {
                self.show_menu(event);
                return;
            }
            let Some(window) = self.window() else {
                return;
            };
            self.ivars()
                .press
                .set(Some((NSEvent::mouseLocation(), window.frame().origin)));
            self.ivars().dragged.set(false);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, _event: &NSEvent) {
            let (Some((start, origin)), Some(window)) = (self.ivars().press.get(), self.window())
            else {
                return;
            };
            let now = NSEvent::mouseLocation();
            let (dx, dy) = (now.x - start.x, now.y - start.y);
            if !self.ivars().dragged.get() && dx.hypot(dy) < DRAG_SLOP {
                return;
            }
            self.ivars().dragged.set(true);
            window.setFrameOrigin(NSPoint::new(origin.x + dx, origin.y + dy));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            if self.ivars().press.take().is_none() {
                return;
            }
            if !self.ivars().dragged.get() {
                (self.ivars().handler)(MiniBarEvent::Click);
                return;
            }
            let Some(window) = self.window() else {
                return;
            };
            let frame = window.frame();
            let origin = keep_on_screen(self.mtm(), frame.origin);
            window.setFrameOrigin(origin);
            (self.ivars().handler)(MiniBarEvent::Moved(MiniBarPosition {
                x: origin.x,
                y: origin.y,
            }));
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.show_menu(event);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.ivars().hover.set(true);
            self.redraw();
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().hover.set(false);
            self.redraw();
        }

        /// The next step of the sweep, for as long as it's working.
        #[unsafe(method(sweep))]
        fn sweep(&self) {
            self.ivars().sweeping.set(false);
            if self.ivars().state.get() != MiniBarState::Working {
                return;
            }
            self.ivars().step.set(self.ivars().step.get() + 1);
            self.redraw();
            self.sweep_later();
        }

        #[unsafe(method(menuItemChosen:))]
        fn menu_item_chosen(&self, item: &NSMenuItem) {
            if let Ok(index) = usize::try_from(item.tag()) {
                (self.ivars().handler)(MiniBarEvent::Menu(index));
            }
        }

        /// VoiceOver's "press" works like a click.
        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> bool {
            (self.ivars().handler)(MiniBarEvent::Click);
            true
        }
    }
);

impl BarView {
    fn new(mtm: MainThreadMarker, handler: MiniBarHandler) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler,
            press: Cell::new(None),
            dragged: Cell::new(false),
            menu: OnceCell::new(),
            parts: OnceCell::new(),
            state: Cell::new(MiniBarState::Idle),
            hover: Cell::new(false),
            levels: RefCell::new([0.0; 3]),
            step: Cell::new(0),
            sweeping: Cell::new(false),
        });
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, HEIGHT));
        // SAFETY: NSView's designated initializer, on a freshly allocated
        // instance whose ivars are set.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn show_menu(&self, event: &NSEvent) {
        if let Some(menu) = self.ivars().menu.get() {
            NSMenu::popUpContextMenu_withEvent_forView(menu, event, self);
        }
    }

    /// Runs `sweep` once, a moment from now, unless one is already waiting
    /// (or motion is to be kept down: then the dots stay still).
    fn sweep_later(&self) {
        if self.ivars().sweeping.get() || crate::reduce_motion() {
            return;
        }
        self.ivars().sweeping.set(true);
        // SAFETY: `sweep` is a method of this class taking no arguments;
        // the run loop keeps `self` alive until it has run.
        let _: () = unsafe {
            msg_send![
                self,
                performSelector: sel!(sweep),
                withObject: None::<&AnyObject>,
                afterDelay: SWEEP
            ]
        };
    }

    /// Draws the pill, its microphone and its wave for the state it's in.
    fn redraw(&self) {
        let Some(parts) = self.ivars().parts.get() else {
            return;
        };
        let state = self.ivars().state.get();
        let orange = color(1.0, 0.60, 0.24, 1.0);
        let white = |alpha: f64| color(1.0, 1.0, 1.0, alpha);

        parts
            .background
            .setFillColor(&color(0.09, 0.09, 0.11, 0.92));
        // A hairline so it shows on a dark desktop; orange while it listens.
        if state == MiniBarState::Recording {
            parts.background.setBorderWidth(1.5);
            parts.background.setBorderColor(&orange);
        } else {
            parts.background.setBorderWidth(1.0);
            parts.background.setBorderColor(&white(0.18));
        }

        let hover = self.ivars().hover.get();
        if let Some(mic) = &parts.mic {
            let tint = match state {
                MiniBarState::Recording => orange.clone(),
                MiniBarState::Idle if hover => orange.clone(),
                MiniBarState::NotReady => white(0.3),
                MiniBarState::Idle | MiniBarState::Working => white(0.85),
            };
            mic.setContentTintColor(Some(&tint));
        }

        let levels = *self.ivars().levels.borrow();
        // The sweep goes to the last dot and back.
        let step = self.ivars().step.get() % (2 * BARS - 2);
        let lit = if step < BARS {
            step
        } else {
            2 * BARS - 2 - step
        };
        for (i, bar) in parts.bars.iter().enumerate() {
            let from_middle = i.abs_diff(BARS / 2);
            let (h, fill) = match state {
                MiniBarState::Recording => {
                    // Speech RMS is roughly 0.01–0.2; a square-root curve
                    // makes quiet speech visible without loud speech pinning
                    // the bars (as the overlay's waveform does).
                    let level = f64::from(levels[2 - from_middle]);
                    let norm = (level * 25.0).sqrt().min(1.0) * ENVELOPE[i];
                    let h = (BAR_MIN_H + norm * (BAR_MAX_H - BAR_MIN_H)).round();
                    (h, orange.clone())
                }
                MiniBarState::Working => {
                    let alpha = match i.abs_diff(lit) {
                        0 => 1.0,
                        1 => 0.55,
                        _ => 0.28,
                    };
                    (DOT_H, white(alpha))
                }
                MiniBarState::NotReady => (REST[i], white(0.25)),
                MiniBarState::Idle if hover => (REST[i], orange.clone()),
                MiniBarState::Idle => (REST[i], white(0.7)),
            };
            bar.setFrame(NSRect::new(
                NSPoint::new(bars_x() + i as f64 * (BAR_W + GAP), (HEIGHT - h) / 2.0),
                NSSize::new(BAR_W, h),
            ));
            bar.setCornerRadius(BAR_W / 2.0);
            bar.setFillColor(&fill);
        }
    }
}

/// Where the microphone starts: it and the wave sit in the pill's middle.
fn mic_x() -> f64 {
    let wave = BARS as f64 * BAR_W + (BARS - 1) as f64 * GAP;
    (WIDTH - (MIC_W + SPACE + wave)) / 2.0
}

/// Where the first bar of the wave starts.
fn bars_x() -> f64 {
    mic_x() + MIC_W + SPACE
}

/// The microphone: the system's own symbol, tinted as the state asks.
fn mic_view(mtm: MainThreadMarker) -> Option<Retained<NSImageView>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str("mic.fill"),
        None,
    )?;
    let frame = NSRect::new(
        NSPoint::new(mic_x(), (HEIGHT - MIC_H) / 2.0),
        NSSize::new(MIC_W, MIC_H),
    );
    let view = NSImageView::initWithFrame(NSImageView::alloc(mtm), frame);
    view.setImage(Some(&image));
    view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    Some(view)
}

/// A plain filled box, for the pill and each bar.
fn filled_box(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSBox> {
    let b = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    b.setBoxType(NSBoxType::Custom);
    b.setTitlePosition(NSTitlePosition::NoTitle);
    b.setBorderWidth(0.0);
    b.setContentViewMargins(NSSize::ZERO);
    b
}

fn color(r: f64, g: f64, b: f64, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}

pub(crate) struct MiniBar {
    panel: Retained<NSPanel>,
    view: Retained<BarView>,
}

impl MiniBar {
    pub(crate) fn new(
        position: Option<MiniBarPosition>,
        menu: &[&str],
        handler: MiniBarHandler,
    ) -> Result<Self> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            PlatformError::Failed("the mini bar must be created on the main thread".into())
        })?;

        let origin = position
            .map(|p| NSPoint::new(p.x, p.y))
            .filter(|&p| on_a_screen(mtm, p))
            .or_else(|| default_origin(mtm))
            .unwrap_or(NSPoint::ZERO);
        // `NonactivatingPanel` only works when given here, at creation.
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            NSRect::new(origin, NSSize::new(WIDTH, HEIGHT)),
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setLevel(NSStatusWindowLevel);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setFloatingPanel(true);
        panel.setHidesOnDeactivate(false);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setMovable(false);
        // Splashdown is almost never the active app while the bar is used.
        panel.setAllowsToolTipsWhenApplicationIsInactive(true);
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

        let view = BarView::new(mtm, handler);
        view.setAccessibilityElement(true);
        // SAFETY: reads an immutable constant exported by AppKit.
        view.setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
        view.setAccessibilityLabel(Some(&NSString::from_str("Splashdown dictation")));

        let background = filled_box(mtm, NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, HEIGHT)));
        background.setCornerRadius(HEIGHT / 2.0);
        view.addSubview(&background);
        let mic = mic_view(mtm);
        if let Some(mic) = &mic {
            view.addSubview(mic);
        }
        let bars: Vec<Retained<NSBox>> = (0..BARS)
            .map(|_| {
                let bar = filled_box(mtm, NSRect::ZERO);
                view.addSubview(&bar);
                bar
            })
            .collect();
        let _ = view.ivars().parts.set(Parts {
            background,
            mic,
            bars,
        });

        // The pointer coming and going, whichever app is in front.
        // SAFETY: the area is owned by the view it reports to, which
        // implements `mouseEntered:` and `mouseExited:`.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                objc2::AllocAnyThread::alloc(),
                NSRect::ZERO,
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&area);

        if !menu.is_empty() {
            let items = NSMenu::new(mtm);
            for (i, title) in menu.iter().enumerate() {
                let item = NSMenuItem::new(mtm);
                item.setTitle(&NSString::from_str(title));
                item.setTag(i as isize);
                // SAFETY: the view implements `menuItemChosen:` and outlives
                // the menu (it owns it); the target is held weakly.
                unsafe {
                    item.setAction(Some(sel!(menuItemChosen:)));
                    item.setTarget(Some(view.as_ref() as &AnyObject));
                }
                items.addItem(&item);
            }
            let _ = view.ivars().menu.set(items);
        }

        panel.setContentView(Some(&view));

        let bar = Self { panel, view };
        bar.set_state(MiniBarState::Idle);
        Ok(bar)
    }

    pub(crate) fn show(&self) {
        // Brings it forward without activating the app or making it key.
        self.panel.orderFrontRegardless();
    }

    pub(crate) fn hide(&self) {
        self.panel.orderOut(None);
    }

    pub(crate) fn set_state(&self, state: MiniBarState) {
        let ivars = self.view.ivars();
        if ivars.state.replace(state) != state {
            *ivars.levels.borrow_mut() = [0.0; 3];
            ivars.step.set(BARS / 2);
        }
        self.view.redraw();
        if state == MiniBarState::Working {
            self.view.sweep_later();
        }
    }

    /// The live microphone level (0–1), shown while it listens.
    pub(crate) fn set_level(&self, level: f32) {
        let ivars = self.view.ivars();
        {
            let mut levels = ivars.levels.borrow_mut();
            levels.rotate_left(1);
            levels[2] = level;
        }
        if ivars.state.get() == MiniBarState::Recording {
            self.view.redraw();
        }
    }

    pub(crate) fn set_tooltip(&self, text: &str) {
        let text = NSString::from_str(text);
        self.view.setToolTip(Some(&text));
        self.view.setAccessibilityHelp(Some(&text));
    }
}

/// The screen with this point, or `None`.
fn screen_at(mtm: MainThreadMarker, point: NSPoint) -> Option<Retained<NSScreen>> {
    NSScreen::screens(mtm).iter().find(|s| {
        let f = s.frame();
        point.x >= f.origin.x
            && point.x <= f.origin.x + f.size.width
            && point.y >= f.origin.y
            && point.y <= f.origin.y + f.size.height
    })
}

/// Whether the bar's center would be on a screen with its corner here.
fn on_a_screen(mtm: MainThreadMarker, origin: NSPoint) -> bool {
    screen_at(mtm, center(origin)).is_some()
}

fn center(origin: NSPoint) -> NSPoint {
    NSPoint::new(origin.x + WIDTH / 2.0, origin.y + HEIGHT / 2.0)
}

/// The bottom middle of the main screen, just above the Dock.
fn default_origin(mtm: MainThreadMarker) -> Option<NSPoint> {
    let vis = NSScreen::mainScreen(mtm)?.visibleFrame();
    Some(NSPoint::new(
        (vis.origin.x + (vis.size.width - WIDTH) / 2.0).round(),
        (vis.origin.y + BOTTOM_MARGIN).round(),
    ))
}

/// Pulls a dropped bar fully inside the visible part of the screen its
/// center is on (not under the menu bar or Dock).
fn keep_on_screen(mtm: MainThreadMarker, origin: NSPoint) -> NSPoint {
    let Some(screen) = screen_at(mtm, center(origin)).or_else(|| NSScreen::mainScreen(mtm)) else {
        return origin;
    };
    let vis = screen.visibleFrame();
    NSPoint::new(
        origin
            .x
            .min(vis.origin.x + vis.size.width - WIDTH)
            .max(vis.origin.x)
            .round(),
        origin
            .y
            .min(vis.origin.y + vis.size.height - HEIGHT)
            .max(vis.origin.y)
            .round(),
    )
}
