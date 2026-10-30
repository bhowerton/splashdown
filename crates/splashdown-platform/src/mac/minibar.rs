//! The mini bar: a small round button that stays on screen, for starting
//! and finishing dictation with the mouse (spec §2.4).
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

use std::cell::{Cell, OnceCell};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityButtonRole, NSAppearance, NSAppearanceCustomization,
    NSAppearanceNameDarkAqua, NSBackingStoreType, NSBox, NSBoxType, NSColor, NSControlSize,
    NSEvent, NSEventModifierFlags, NSImage, NSImageScaling, NSImageView, NSMenu, NSMenuItem,
    NSPanel, NSProgressIndicator, NSProgressIndicatorStyle, NSScreen, NSStatusWindowLevel,
    NSTitlePosition, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::{MiniBarEvent, MiniBarHandler, MiniBarPosition, MiniBarState, PlatformError, Result};

/// The button's diameter.
const SIZE: f64 = 40.0;
const GLYPH: f64 = 18.0;
const SPIN: f64 = 18.0;
/// The mouse must move this far before a press counts as a drag.
const DRAG_SLOP: f64 = 3.0;
/// Default place: this far in from the right edge of the screen…
const EDGE_MARGIN: f64 = 12.0;
/// …and this far up it (a fraction of its height).
const DEFAULT_HEIGHT: f64 = 0.3;

struct Ivars {
    handler: MiniBarHandler,
    /// Where the mouse and the panel were when the button went down.
    press: Cell<Option<(NSPoint, NSPoint)>>,
    dragged: Cell<bool>,
    menu: OnceCell<Retained<NSMenu>>,
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

        /// Every click inside lands here, not on the glyph or spinner.
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
        });
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(SIZE, SIZE));
        // SAFETY: NSView's designated initializer, on a freshly allocated
        // instance whose ivars are set.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn show_menu(&self, event: &NSEvent) {
        if let Some(menu) = self.ivars().menu.get() {
            NSMenu::popUpContextMenu_withEvent_forView(menu, event, self);
        }
    }
}

fn color(r: f64, g: f64, b: f64, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}

fn symbol(name: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
}

pub(crate) struct MiniBar {
    panel: Retained<NSPanel>,
    view: Retained<BarView>,
    background: Retained<NSBox>,
    glyph: Retained<NSImageView>,
    spinner: Retained<NSProgressIndicator>,
    /// The symbols for each state, looked up once.
    mic: Option<Retained<NSImage>>,
    mic_off: Option<Retained<NSImage>>,
    stop: Option<Retained<NSImage>>,
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
            NSRect::new(origin, NSSize::new(SIZE, SIZE)),
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
        // SAFETY: reads an immutable constant exported by AppKit.
        let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua });
        // The spinner is drawn for a dark background.
        panel.setAppearance(dark.as_deref());
        // SAFETY: on the main thread (`mtm`). Turning this off is what makes
        // owning the panel through `Retained` sound: closing it won't also
        // release it behind our back.
        unsafe { panel.setReleasedWhenClosed(false) };

        let view = BarView::new(mtm, handler);
        view.setAccessibilityElement(true);
        // SAFETY: reads an immutable constant exported by AppKit.
        view.setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
        view.setAccessibilityLabel(Some(&NSString::from_str("Splashdown dictation")));

        let background = NSBox::initWithFrame(
            NSBox::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(SIZE, SIZE)),
        );
        background.setBoxType(NSBoxType::Custom);
        background.setTitlePosition(NSTitlePosition::NoTitle);
        background.setBorderWidth(0.0);
        background.setCornerRadius(SIZE / 2.0);
        background.setContentViewMargins(NSSize::ZERO);
        view.addSubview(&background);

        let glyph = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(
                NSPoint::new((SIZE - GLYPH) / 2.0, (SIZE - GLYPH) / 2.0),
                NSSize::new(GLYPH, GLYPH),
            ),
        );
        glyph.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        view.addSubview(&glyph);

        let spinner = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(mtm),
            NSRect::new(
                NSPoint::new((SIZE - SPIN) / 2.0, (SIZE - SPIN) / 2.0),
                NSSize::new(SPIN, SPIN),
            ),
        );
        spinner.setStyle(NSProgressIndicatorStyle::Spinning);
        spinner.setControlSize(NSControlSize::Small);
        spinner.setDisplayedWhenStopped(false);
        view.addSubview(&spinner);

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

        let bar = Self {
            panel,
            view,
            background,
            glyph,
            spinner,
            mic: symbol("mic.fill"),
            mic_off: symbol("mic.slash.fill"),
            stop: symbol("stop.fill"),
        };
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
        let dark = color(0.09, 0.09, 0.11, 0.92);
        let (fill, image, tint) = match state {
            MiniBarState::Idle => (dark, &self.mic, color(0.96, 0.96, 0.96, 0.95)),
            MiniBarState::Recording => (
                color(0.94, 0.27, 0.27, 1.0),
                &self.stop,
                color(1.0, 1.0, 1.0, 1.0),
            ),
            MiniBarState::Working => (dark, &None, color(1.0, 1.0, 1.0, 1.0)),
            MiniBarState::NotReady => (dark, &self.mic_off, color(0.70, 0.70, 0.72, 0.8)),
        };
        self.background.setFillColor(&fill);
        self.glyph.setImage(image.as_deref());
        self.glyph.setContentTintColor(Some(&tint));
        let working = state == MiniBarState::Working;
        self.glyph.setHidden(working);
        // SAFETY: on the main thread (we hold a main-thread-only view); the
        // sender argument is unused and may be nil.
        unsafe {
            if working {
                self.spinner.startAnimation(None);
            } else {
                self.spinner.stopAnimation(None);
            }
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
    NSPoint::new(origin.x + SIZE / 2.0, origin.y + SIZE / 2.0)
}

/// By the right edge of the main screen, a little below the middle.
fn default_origin(mtm: MainThreadMarker) -> Option<NSPoint> {
    let vis = NSScreen::mainScreen(mtm)?.visibleFrame();
    Some(NSPoint::new(
        (vis.origin.x + vis.size.width - SIZE - EDGE_MARGIN).round(),
        (vis.origin.y + vis.size.height * DEFAULT_HEIGHT).round(),
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
            .min(vis.origin.x + vis.size.width - SIZE)
            .max(vis.origin.x)
            .round(),
        origin
            .y
            .min(vis.origin.y + vis.size.height - SIZE)
            .max(vis.origin.y)
            .round(),
    )
}
