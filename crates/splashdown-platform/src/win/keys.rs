//! System-wide key listener: a `WH_KEYBOARD_LL` hook on its own thread,
//! which runs a message loop (the hook is called from it).

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
    VK_RCONTROL, VK_RETURN, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED,
    LLKHF_UP, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_QUIT, WM_USER,
};

use super::INJECTED_TAG;
use crate::{KeyEvent, KeyHandler, PhysicalKey, PlatformError, Result};

/// Scan code of the right Shift key (Shift has no extended-key flag).
const SCAN_RIGHT_SHIFT: u32 = 0x36;
/// AltGr sends a fake Left Ctrl before Right Alt; its scan code carries this
/// bit, which no real key has.
const SCAN_ALTGR_FAKE_CTRL: u32 = 0x200;

pub(crate) struct KeyListener {
    thread_id: u32,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl KeyListener {
    pub(crate) fn start(handler: KeyHandler) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("splashdown-keys".into())
            .spawn(move || run_hook(handler, tx))
            .map_err(|e| PlatformError::Failed(e.to_string()))?;

        match rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(PlatformError::Failed("key listener thread died".into())),
        }
    }
}

impl Drop for KeyListener {
    fn drop(&mut self) {
        // SAFETY: plain values. The thread created its message queue before
        // reporting its id, so the post reaches it; if the thread has
        // already exited, the call just fails.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct HookState {
    handler: KeyHandler,
    /// Modifiers that are down, with whether their key-down was swallowed.
    /// Windows auto-repeats held modifiers, but the gesture recognizer
    /// expects one key-down per press (as macOS reports them), so repeats
    /// are answered here without reaching it.
    held: [Option<bool>; MODIFIERS],
}

const MODIFIERS: usize = 8;

thread_local! {
    /// Set on the hook thread only; the hook procedure runs on that thread.
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

fn run_hook(handler: KeyHandler, ready: mpsc::Sender<Result<u32>>) {
    STATE.with(|s| {
        *s.borrow_mut() = Some(HookState {
            handler,
            held: [None; MODIFIERS],
        });
    });

    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid out-pointer and PM_NOREMOVE leaves the queue
    // as it is. The call makes sure this thread has a message queue, so
    // `Drop` can post WM_QUIT to it.
    let _ = unsafe { PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE) };

    // SAFETY: a null name asks for this executable's own module handle,
    // which needs no cleanup.
    let module = unsafe { GetModuleHandleW(None) }
        .ok()
        .map(|m| HINSTANCE(m.0));
    // SAFETY: `hook_proc` has the HOOKPROC signature and is a plain function,
    // valid for the life of the process. Thread id 0 (all threads) is what
    // low-level hooks require; they're called on this thread, in GetMessageW.
    let hook = match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0) } {
        Ok(hook) => hook,
        Err(e) => {
            let _ = ready.send(Err(PlatformError::Failed(format!(
                "couldn't install the keyboard hook: {e}"
            ))));
            return;
        }
    };

    // SAFETY: takes no arguments.
    let thread_id = unsafe { GetCurrentThreadId() };
    if ready.send(Ok(thread_id)).is_ok() {
        loop {
            // SAFETY: `msg` is a valid out-pointer; no window filter, so
            // this also receives thread messages such as WM_QUIT.
            let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            // 0 is WM_QUIT, -1 an error.
            if got.0 <= 0 {
                break;
            }
            // SAFETY: `msg` was just filled in by GetMessageW.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    // SAFETY: `hook` was installed above by this thread and is removed once.
    let _ = unsafe { UnhookWindowsHookEx(hook) };
    STATE.with(|s| s.borrow_mut().take());
}

/// The low-level keyboard hook. It must return quickly (within
/// `LowLevelHooksTimeout`) or Windows skips it and may remove it, so it only
/// asks the handler, which is documented to be quick.
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, `lparam` points to a KBDLLHOOKSTRUCT that
        // stays valid for the duration of this call.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // A panic must not unwind into Windows; treat it as "don't swallow".
        if catch_unwind(AssertUnwindSafe(|| on_key(info))).unwrap_or(false) {
            return LRESULT(1);
        }
    }
    // SAFETY: passes this hook's own arguments on unchanged, as the hook
    // chain expects (the hook handle argument is ignored).
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Returns whether to swallow the event.
fn on_key(info: &KBDLLHOOKSTRUCT) -> bool {
    // Our own Ctrl+V, typing and Return (see `clipboard.rs`).
    if info.dwExtraInfo == INJECTED_TAG {
        return false;
    }
    let Some(ev) = translate(info) else {
        return false;
    };
    STATE.with(|s| {
        // Re-entry can't happen on one thread, but never panic in a hook.
        let Ok(mut guard) = s.try_borrow_mut() else {
            return false;
        };
        let Some(state) = guard.as_mut() else {
            return false;
        };
        match modifier_slot(ev.key) {
            Some(slot) if ev.down => {
                if let Some(swallowed) = state.held[slot] {
                    return swallowed; // auto-repeat
                }
                let swallow = (state.handler)(ev);
                state.held[slot] = Some(swallow);
                swallow
            }
            Some(slot) => {
                state.held[slot] = None;
                (state.handler)(ev)
            }
            None => (state.handler)(ev),
        }
    })
}

fn modifier_slot(key: PhysicalKey) -> Option<usize> {
    Some(match key {
        PhysicalKey::LeftShift => 0,
        PhysicalKey::RightShift => 1,
        PhysicalKey::LeftControl => 2,
        PhysicalKey::RightControl => 3,
        PhysicalKey::LeftAlt => 4,
        PhysicalKey::RightAlt => 5,
        PhysicalKey::LeftMeta => 6,
        PhysicalKey::RightMeta => 7,
        _ => return None,
    })
}

fn translate(info: &KBDLLHOOKSTRUCT) -> Option<KeyEvent> {
    let down = !info.flags.contains(LLKHF_UP);
    let extended = info.flags.contains(LLKHF_EXTENDED);
    let Ok(code) = u16::try_from(info.vkCode) else {
        return Some(KeyEvent {
            key: PhysicalKey::Other,
            down,
        });
    };
    let key = match VIRTUAL_KEY(code) {
        // Low-level hooks normally report the sided codes; the generic
        // ones are resolved from the scan code or extended flag.
        VK_LSHIFT => PhysicalKey::LeftShift,
        VK_RSHIFT => PhysicalKey::RightShift,
        VK_SHIFT if info.scanCode == SCAN_RIGHT_SHIFT => PhysicalKey::RightShift,
        VK_SHIFT => PhysicalKey::LeftShift,
        // Not a key the user pressed: ignore it entirely.
        VK_LCONTROL | VK_CONTROL if info.scanCode & SCAN_ALTGR_FAKE_CTRL != 0 => return None,
        VK_LCONTROL => PhysicalKey::LeftControl,
        VK_RCONTROL => PhysicalKey::RightControl,
        VK_CONTROL if extended => PhysicalKey::RightControl,
        VK_CONTROL => PhysicalKey::LeftControl,
        VK_LMENU => PhysicalKey::LeftAlt,
        VK_RMENU => PhysicalKey::RightAlt,
        VK_MENU if extended => PhysicalKey::RightAlt,
        VK_MENU => PhysicalKey::LeftAlt,
        VK_LWIN => PhysicalKey::LeftMeta,
        VK_RWIN => PhysicalKey::RightMeta,
        VK_ESCAPE => PhysicalKey::Escape,
        // Keypad Enter is VK_RETURN with the extended flag.
        VK_RETURN => PhysicalKey::Return,
        // '0'–'9' on the main row (the keypad has its own codes).
        VIRTUAL_KEY(c @ 0x30..=0x39) => PhysicalKey::Digit((c - 0x30) as u8),
        // 'A'–'Z': the layout's letter, wherever it sits.
        VIRTUAL_KEY(c @ 0x41..=0x5A) => {
            PhysicalKey::Letter(char::from(c as u8).to_ascii_lowercase())
        }
        _ => PhysicalKey::Other,
    };
    Some(KeyEvent { key, down })
}
