//! The mouse and keyboard for computer use: CGEvents posted to the HID
//! stream, tagged so our own key listener ignores them. Coordinates are
//! screen points, as `CGEvent` uses them.

use std::time::Duration;

use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGKeyCode, CGMouseButton, EventField,
    ScrollEventUnit,
};
use core_graphics::geometry::CGPoint;
use splashdown_core::computer::{Button, Combo, Direction, Key, Mods};

use super::clipboard::{INJECTED_TAG, post_key, source};
use crate::{PlatformError, Result};

/// Between the events of a click or drag, so apps see them as a person's.
const GAP: Duration = Duration::from_millis(30);

fn failed(what: &str) -> PlatformError {
    PlatformError::Failed(format!("couldn't create the {what} event"))
}

fn flags(mods: Mods) -> CGEventFlags {
    let mut f = CGEventFlags::CGEventFlagNull;
    if mods.cmd {
        f |= CGEventFlags::CGEventFlagCommand;
    }
    if mods.ctrl {
        f |= CGEventFlags::CGEventFlagControl;
    }
    if mods.alt {
        f |= CGEventFlags::CGEventFlagAlternate;
    }
    if mods.shift {
        f |= CGEventFlags::CGEventFlagShift;
    }
    f
}

fn post(event: &CGEvent, mods: Mods) {
    event.set_flags(flags(mods));
    event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, INJECTED_TAG);
    event.post(CGEventTapLocation::HID);
}

fn mouse(kind: CGEventType, at: (f64, f64), button: CGMouseButton) -> Result<CGEvent> {
    CGEvent::new_mouse_event(source()?, kind, CGPoint::new(at.0, at.1), button)
        .map_err(|()| failed("mouse"))
}

/// Where the pointer is, in points.
pub(crate) fn cursor() -> Option<(f64, f64)> {
    let p = CGEvent::new(source().ok()?).ok()?.location();
    Some((p.x, p.y))
}

pub(crate) fn move_to(at: (f64, f64)) -> Result<()> {
    post(
        &mouse(CGEventType::MouseMoved, at, CGMouseButton::Left)?,
        Mods::default(),
    );
    std::thread::sleep(GAP);
    Ok(())
}

pub(crate) fn click(at: (f64, f64), button: Button, count: u8, mods: Mods) -> Result<()> {
    let (down, up, b) = match button {
        Button::Left => (
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGMouseButton::Left,
        ),
        Button::Right => (
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            CGMouseButton::Right,
        ),
        Button::Middle => (
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
            CGMouseButton::Center,
        ),
    };
    move_to(at)?;
    // A double click is two clicks, the second counting as the second.
    for n in 1..=i64::from(count.max(1)) {
        for kind in [down, up] {
            let event = mouse(kind, at, b)?;
            event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, n);
            post(&event, mods);
        }
        std::thread::sleep(GAP);
    }
    Ok(())
}

/// The left button alone, down or up, where the pointer is.
pub(crate) fn left_button(down: bool) -> Result<()> {
    let at = cursor().ok_or_else(|| failed("mouse"))?;
    let kind = if down {
        CGEventType::LeftMouseDown
    } else {
        CGEventType::LeftMouseUp
    };
    post(&mouse(kind, at, CGMouseButton::Left)?, Mods::default());
    std::thread::sleep(GAP);
    Ok(())
}

/// Presses at `from`, moves to `to` in steps (apps want drag events in
/// between), releases.
pub(crate) fn drag(from: (f64, f64), to: (f64, f64), mods: Mods) -> Result<()> {
    move_to(from)?;
    post(
        &mouse(CGEventType::LeftMouseDown, from, CGMouseButton::Left)?,
        mods,
    );
    const STEPS: u32 = 12;
    for i in 1..=STEPS {
        let t = f64::from(i) / f64::from(STEPS);
        let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        post(
            &mouse(CGEventType::LeftMouseDragged, at, CGMouseButton::Left)?,
            mods,
        );
        std::thread::sleep(Duration::from_millis(15));
    }
    post(
        &mouse(CGEventType::LeftMouseUp, to, CGMouseButton::Left)?,
        mods,
    );
    std::thread::sleep(GAP);
    Ok(())
}

/// Scrolls `amount` wheel clicks, at `at` if given.
pub(crate) fn scroll(
    at: Option<(f64, f64)>,
    direction: Direction,
    amount: u32,
    mods: Mods,
) -> Result<()> {
    if let Some(at) = at {
        move_to(at)?;
    }
    let lines = i32::try_from(amount).unwrap_or(50);
    // Wheel 1 is vertical (positive: up), wheel 2 horizontal (positive:
    // left).
    let (vertical, horizontal) = match direction {
        Direction::Up => (lines, 0),
        Direction::Down => (-lines, 0),
        Direction::Left => (0, lines),
        Direction::Right => (0, -lines),
    };
    let event =
        CGEvent::new_scroll_event(source()?, ScrollEventUnit::LINE, 2, vertical, horizontal, 0)
            .map_err(|()| failed("scroll"))?;
    post(&event, mods);
    std::thread::sleep(GAP);
    Ok(())
}

/// The key's code on a US (ANSI) keyboard: shortcuts in macOS follow the
/// key's position for letters on most layouts, close enough for this
/// exploration.
fn keycode(key: &Key) -> Option<CGKeyCode> {
    Some(match key {
        Key::Named(name) => match *name {
            "return" => 0x24,
            "tab" => 0x30,
            "space" => 0x31,
            "backspace" => 0x33,
            "escape" => 0x35,
            "delete" => 0x75,
            "home" => 0x73,
            "end" => 0x77,
            "pageup" => 0x74,
            "pagedown" => 0x79,
            "left" => 0x7B,
            "right" => 0x7C,
            "down" => 0x7D,
            "up" => 0x7E,
            "f1" => 0x7A,
            "f2" => 0x78,
            "f3" => 0x63,
            "f4" => 0x76,
            "f5" => 0x60,
            "f6" => 0x61,
            "f7" => 0x62,
            "f8" => 0x64,
            "f9" => 0x65,
            "f10" => 0x6D,
            "f11" => 0x67,
            "f12" => 0x6F,
            _ => return None,
        },
        Key::Char(c) => match c {
            'a' => 0x00,
            's' => 0x01,
            'd' => 0x02,
            'f' => 0x03,
            'h' => 0x04,
            'g' => 0x05,
            'z' => 0x06,
            'x' => 0x07,
            'c' => 0x08,
            'v' => 0x09,
            'b' => 0x0B,
            'q' => 0x0C,
            'w' => 0x0D,
            'e' => 0x0E,
            'r' => 0x0F,
            'y' => 0x10,
            't' => 0x11,
            '1' => 0x12,
            '2' => 0x13,
            '3' => 0x14,
            '4' => 0x15,
            '6' => 0x16,
            '5' => 0x17,
            '=' | '+' => 0x18,
            '9' => 0x19,
            '7' => 0x1A,
            '-' | '_' => 0x1B,
            '8' => 0x1C,
            '0' => 0x1D,
            ']' => 0x1E,
            'o' => 0x1F,
            'u' => 0x20,
            '[' => 0x21,
            'i' => 0x22,
            'p' => 0x23,
            'l' => 0x25,
            'j' => 0x26,
            '\'' => 0x27,
            'k' => 0x28,
            ';' => 0x29,
            '\\' => 0x2A,
            ',' => 0x2B,
            '/' => 0x2C,
            'n' => 0x2D,
            'm' => 0x2E,
            '.' => 0x2F,
            '`' => 0x32,
            _ => return None,
        },
    })
}

pub(crate) fn key(combo: &Combo, repeat: u32) -> Result<()> {
    let code = keycode(&combo.key)
        .ok_or_else(|| PlatformError::Failed(format!("no key for {:?}", combo.key)))?;
    for _ in 0..repeat.max(1) {
        post_key(code, flags(combo.mods))?;
    }
    Ok(())
}

/// Holds a key (with its modifiers) for `seconds`.
pub(crate) fn hold(combo: &Combo, seconds: f64) -> Result<()> {
    let code = keycode(&combo.key)
        .ok_or_else(|| PlatformError::Failed(format!("no key for {:?}", combo.key)))?;
    let event = |down: bool| -> Result<()> {
        let e = CGEvent::new_keyboard_event(source()?, code, down).map_err(|()| failed("key"))?;
        post(&e, combo.mods);
        Ok(())
    };
    event(true)?;
    std::thread::sleep(Duration::from_secs_f64(seconds.clamp(0.0, 300.0)));
    event(false)
}
