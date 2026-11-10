//! System-wide key listener: a CGEventTap on its own run-loop thread. It
//! also sees the mouse's side buttons, which can be the dictation key.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, mpsc};

use core_foundation::base::TCFType;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};

use super::ffi;
use crate::{KeyEvent, KeyHandler, PhysicalKey, PlatformError, Result};

/// Device-specific modifier bits in CGEventFlags (IOKit `NX_DEVICE*KEYMASK`),
/// which tell left and right modifiers apart.
const DEVICE_LCTL: u64 = 0x0000_0001;
const DEVICE_LSHIFT: u64 = 0x0000_0002;
const DEVICE_RSHIFT: u64 = 0x0000_0004;
const DEVICE_LCMD: u64 = 0x0000_0008;
const DEVICE_RCMD: u64 = 0x0000_0010;
const DEVICE_LALT: u64 = 0x0000_0020;
const DEVICE_RALT: u64 = 0x0000_0040;
const DEVICE_RCTL: u64 = 0x0000_2000;
const FLAG_FN: u64 = 0x0080_0000;

/// macOS virtual key codes (`kVK_*`).
mod vk {
    pub const ESCAPE: i64 = 0x35;
    pub const RETURN: i64 = 0x24;
    pub const KEYPAD_ENTER: i64 = 0x4C;
    pub const RIGHT_COMMAND: i64 = 0x36;
    pub const LEFT_COMMAND: i64 = 0x37;
    pub const LEFT_SHIFT: i64 = 0x38;
    pub const LEFT_OPTION: i64 = 0x3A;
    pub const LEFT_CONTROL: i64 = 0x3B;
    pub const RIGHT_SHIFT: i64 = 0x3C;
    pub const RIGHT_OPTION: i64 = 0x3D;
    pub const RIGHT_CONTROL: i64 = 0x3E;
    pub const FUNCTION: i64 = 0x3F;
}

pub(crate) struct KeyListener {
    run_loop: SendRunLoop,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// `CFRunLoopStop` may be called from any thread.
struct SendRunLoop(CFRunLoop);
// SAFETY: it's only a retained CFRunLoopRef. CF retain/release are atomic,
// and the only thing done with it off its own thread is `CFRunLoopStop`,
// which Apple documents as callable from any thread.
unsafe impl Send for SendRunLoop {}

impl KeyListener {
    pub(crate) fn start(handler: KeyHandler) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("splashdown-keys".into())
            .spawn(move || run_tap(handler, tx))
            .map_err(|e| PlatformError::Failed(e.to_string()))?;

        match rx.recv() {
            Ok(Ok(run_loop)) => Ok(Self {
                run_loop,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(PlatformError::Failed("key listener thread died".into())),
        }
    }
}

impl Drop for KeyListener {
    fn drop(&mut self) {
        self.run_loop.0.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run_tap(handler: KeyHandler, ready: mpsc::Sender<Result<SendRunLoop>>) {
    let handler = RefCell::new(handler);
    // The tap's mach port, so the callback can re-enable a disabled tap.
    let port: Arc<AtomicPtr<c_void>> = Arc::default();
    let port_in_cb = port.clone();

    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::FlagsChanged,
            // Side buttons (and the middle button, which we ignore).
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
        ],
        move |_proxy, event_type, event| {
            match event_type {
                // macOS disables taps that are slow or when secure input
                // toggles; turn it straight back on.
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
                    let p = port_in_cb.load(Ordering::Acquire);
                    if !p.is_null() {
                        // SAFETY: `p` is the tap's CFMachPortRef, stored
                        // before the run loop started. The callback only runs
                        // inside that run loop, and `tap` (which owns the
                        // port) outlives it.
                        unsafe { ffi::CGEventTapEnable(p, true) };
                    }
                    return CallbackResult::Keep;
                }
                _ => {}
            }
            let Some(ev) = translate(event_type, event) else {
                return CallbackResult::Keep;
            };
            let swallow = match handler.try_borrow_mut() {
                Ok(mut h) => h(ev),
                Err(_) => false,
            };
            if swallow {
                CallbackResult::Drop
            } else {
                CallbackResult::Keep
            }
        },
    );

    let tap = match tap {
        Ok(tap) => tap,
        Err(()) => {
            // The only reason the system refuses an active key tap.
            let _ = ready.send(Err(PlatformError::Permission("Accessibility")));
            return;
        }
    };
    port.store(
        tap.mach_port().as_concrete_TypeRef() as *mut c_void,
        Ordering::Release,
    );

    let Ok(source) = tap.mach_port().create_runloop_source(0) else {
        let _ = ready.send(Err(PlatformError::Failed(
            "couldn't create run loop source".into(),
        )));
        return;
    };
    let run_loop = CFRunLoop::get_current();
    // SAFETY: reads an immutable constant exported by CoreFoundation.
    run_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });
    tap.enable();

    if ready.send(Ok(SendRunLoop(run_loop))).is_err() {
        return;
    }
    CFRunLoop::run_current();
    // `tap` drops here, after the run loop stops.
}

/// ANSI number-row key codes (`kVK_ANSI_0`–`9`).
/// Letter keys by their position on the US layout (kVK_ANSI_*).
fn letter(code: i64) -> Option<char> {
    const LETTERS: [(i64, char); 26] = [
        (0x00, 'a'),
        (0x0B, 'b'),
        (0x08, 'c'),
        (0x02, 'd'),
        (0x0E, 'e'),
        (0x03, 'f'),
        (0x05, 'g'),
        (0x04, 'h'),
        (0x22, 'i'),
        (0x26, 'j'),
        (0x28, 'k'),
        (0x25, 'l'),
        (0x2E, 'm'),
        (0x2D, 'n'),
        (0x1F, 'o'),
        (0x23, 'p'),
        (0x0C, 'q'),
        (0x0F, 'r'),
        (0x01, 's'),
        (0x11, 't'),
        (0x20, 'u'),
        (0x09, 'v'),
        (0x0D, 'w'),
        (0x07, 'x'),
        (0x10, 'y'),
        (0x06, 'z'),
    ];
    LETTERS.iter().find(|(c, _)| *c == code).map(|(_, l)| *l)
}

fn digit(code: i64) -> Option<u8> {
    Some(match code {
        0x1D => 0,
        0x12 => 1,
        0x13 => 2,
        0x14 => 3,
        0x15 => 4,
        0x17 => 5,
        0x16 => 6,
        0x1A => 7,
        0x1C => 8,
        0x19 => 9,
        _ => return None,
    })
}

/// Mouse button numbers as CGEvent counts them (0 = left).
const MOUSE_BACK: i64 = 3;
const MOUSE_FORWARD: i64 = 4;

/// Whether `key` is down, for the keys that can be the dictation key:
/// the modifiers and the mouse's side buttons. The session's combined
/// state, so a key sent by a remapping tool counts as the tap saw it.
pub(crate) fn key_is_down(key: PhysicalKey) -> Option<bool> {
    const COMBINED_SESSION_STATE: i32 = 0;
    let code = match key {
        PhysicalKey::LeftShift => vk::LEFT_SHIFT,
        PhysicalKey::RightShift => vk::RIGHT_SHIFT,
        PhysicalKey::LeftControl => vk::LEFT_CONTROL,
        PhysicalKey::RightControl => vk::RIGHT_CONTROL,
        PhysicalKey::LeftAlt => vk::LEFT_OPTION,
        PhysicalKey::RightAlt => vk::RIGHT_OPTION,
        PhysicalKey::LeftMeta => vk::LEFT_COMMAND,
        PhysicalKey::RightMeta => vk::RIGHT_COMMAND,
        PhysicalKey::Fn => {
            // SAFETY: takes a state id and has no other preconditions.
            let flags = unsafe { ffi::CGEventSourceFlagsState(COMBINED_SESSION_STATE) };
            return Some(flags & FLAG_FN != 0);
        }
        PhysicalKey::MouseBack | PhysicalKey::MouseForward => {
            let button = if key == PhysicalKey::MouseBack {
                MOUSE_BACK
            } else {
                MOUSE_FORWARD
            };
            // SAFETY: takes a state id and a button number, any of which
            // it answers for; no other preconditions.
            return Some(unsafe {
                ffi::CGEventSourceButtonState(COMBINED_SESSION_STATE, button as u32)
            });
        }
        _ => return None,
    };
    // SAFETY: takes a state id and a virtual key code, any of which it
    // answers for; no other preconditions.
    Some(unsafe { ffi::CGEventSourceKeyState(COMBINED_SESSION_STATE, code as u16) })
}

fn translate(event_type: CGEventType, event: &CGEvent) -> Option<KeyEvent> {
    // Our own synthetic keys (⌘V, Return…) aren't the user's.
    if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA)
        == super::clipboard::INJECTED_TAG
    {
        return None;
    }
    if matches!(
        event_type,
        CGEventType::OtherMouseDown | CGEventType::OtherMouseUp
    ) {
        let key = match event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER) {
            MOUSE_BACK => PhysicalKey::MouseBack,
            MOUSE_FORWARD => PhysicalKey::MouseForward,
            _ => return None,
        };
        return Some(KeyEvent {
            key,
            down: matches!(event_type, CGEventType::OtherMouseDown),
        });
    }
    let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
    match event_type {
        CGEventType::KeyDown | CGEventType::KeyUp => {
            let key = match code {
                vk::ESCAPE => PhysicalKey::Escape,
                vk::RETURN | vk::KEYPAD_ENTER => PhysicalKey::Return,
                c => digit(c)
                    .map(PhysicalKey::Digit)
                    .or_else(|| letter(c).map(PhysicalKey::Letter))
                    .unwrap_or(PhysicalKey::Other),
            };
            Some(KeyEvent {
                key,
                down: matches!(event_type, CGEventType::KeyDown),
            })
        }
        CGEventType::FlagsChanged => {
            let flags = event.get_flags().bits();
            let (key, mask) = match code {
                vk::LEFT_SHIFT => (PhysicalKey::LeftShift, DEVICE_LSHIFT),
                vk::RIGHT_SHIFT => (PhysicalKey::RightShift, DEVICE_RSHIFT),
                vk::LEFT_CONTROL => (PhysicalKey::LeftControl, DEVICE_LCTL),
                vk::RIGHT_CONTROL => (PhysicalKey::RightControl, DEVICE_RCTL),
                vk::LEFT_OPTION => (PhysicalKey::LeftAlt, DEVICE_LALT),
                vk::RIGHT_OPTION => (PhysicalKey::RightAlt, DEVICE_RALT),
                vk::LEFT_COMMAND => (PhysicalKey::LeftMeta, DEVICE_LCMD),
                vk::RIGHT_COMMAND => (PhysicalKey::RightMeta, DEVICE_RCMD),
                vk::FUNCTION => (PhysicalKey::Fn, FLAG_FN),
                // Caps Lock and friends.
                _ => return None,
            };
            Some(KeyEvent {
                key,
                down: flags & mask != 0,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_that_can_start_dictation_can_be_asked_about() {
        // Nobody is holding these while the tests run; what matters is
        // that each is a key the system answers for.
        for key in [
            PhysicalKey::RightAlt,
            PhysicalKey::RightMeta,
            PhysicalKey::LeftControl,
            PhysicalKey::Fn,
            PhysicalKey::MouseBack,
        ] {
            assert!(key_is_down(key).is_some(), "{key:?}");
        }
        assert_eq!(key_is_down(PhysicalKey::Letter('a')), None);
        assert_eq!(key_is_down(PhysicalKey::Escape), None);
    }
}
