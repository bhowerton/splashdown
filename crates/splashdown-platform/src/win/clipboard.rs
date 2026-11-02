//! Clipboard snapshot/restore, paste and typing via `SendInput`, and the
//! copy-based selection fallback.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, E_ACCESSDENIED, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MapVirtualKeyW,
    SendInput, VIRTUAL_KEY, VK_B, VK_BACK, VK_C, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_HOME,
    VK_I, VK_INSERT, VK_LEFT, VK_LWIN, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE,
    VK_MEDIA_PREV_TRACK, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_UP,
    VK_V, VK_Z, VkKeyScanW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
};
use windows::core::w;

use super::{INJECTED_TAG, from_wide};
use crate::{PlatformError, Result};

/// How long to wait after Ctrl+V before restoring the clipboard. Apps read
/// the clipboard when they handle the key; too short and they paste the
/// old contents.
const PASTE_SETTLE: Duration = Duration::from_millis(250);
/// How long to wait for Ctrl+C to update the clipboard.
const COPY_TIMEOUT: Duration = Duration::from_millis(150);

/// Standard clipboard formats (`CF_*`) that need special handling.
const CF_UNICODETEXT: u32 = 13;

/// Shortcuts are matched by virtual-key code on Windows, and `SendInput`
/// sends the code itself (VK_V is "V" on Dvorak and AZERTY alike), so the
/// layout doesn't matter and there's nothing to look up.
pub(crate) fn refresh_shortcut_keys() {}

// ---------------------------------------------------------------- clipboard

/// Proof that this thread has the clipboard open; closes it when dropped.
struct Open;

impl Open {
    /// Opens the clipboard, retrying briefly while another app holds it.
    fn new() -> Option<Self> {
        for _ in 0..20 {
            // SAFETY: no pointers. With no owner window, EmptyClipboard
            // leaves the clipboard unowned; SetClipboardData still works and
            // nobody sends us WM_RENDERFORMAT, since we never delay-render.
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Some(Open);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    fn empty(&self) -> bool {
        // SAFETY: the clipboard is open on this thread (`self`).
        unsafe { EmptyClipboard() }.is_ok()
    }

    /// The bytes of an HGLOBAL-backed format.
    fn read(&self, format: u32) -> Option<Vec<u8>> {
        // SAFETY: the clipboard is open on this thread (`self`).
        let handle = unsafe { GetClipboardData(format) }.ok()?;
        let global = HGLOBAL(handle.0);
        // SAFETY: callers only ask for formats whose data is an HGLOBAL
        // (`is_memory_format`); the clipboard owns it and keeps it alive
        // while it's open. GlobalSize and GlobalLock validate the handle and
        // return 0 / null if it isn't a memory block after all.
        let size = unsafe { GlobalSize(global) };
        if size == 0 {
            return None;
        }
        // SAFETY: as above.
        let ptr = unsafe { GlobalLock(global) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: a locked block of `size` bytes stays valid until unlocked;
        // we copy it out first.
        let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), size) }.to_vec();
        // SAFETY: balances the GlobalLock above.
        let _ = unsafe { GlobalUnlock(global) };
        Some(bytes)
    }

    /// Puts `bytes` on the clipboard as `format`.
    fn write(&self, format: u32, bytes: &[u8]) -> bool {
        // SAFETY: plain values; returns a new movable block or an error.
        let Ok(global) = (unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }) else {
            return false;
        };
        // SAFETY: `global` is the live block just allocated.
        let ptr = unsafe { GlobalLock(global) };
        if ptr.is_null() {
            // SAFETY: we still own `global`, and free it once.
            let _ = unsafe { GlobalFree(Some(global)) };
            return false;
        }
        // SAFETY: the block holds at least `bytes.len()` bytes and can't
        // overlap `bytes`, which we own separately; then unlock it.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), bytes.len());
            let _ = GlobalUnlock(global);
        }
        // SAFETY: the clipboard is open on this thread (`self`). On success
        // the system owns `global`; on failure we still do and free it.
        if unsafe { SetClipboardData(format, Some(HANDLE(global.0))) }.is_ok() {
            true
        } else {
            // SAFETY: see above.
            let _ = unsafe { GlobalFree(Some(global)) };
            false
        }
    }

    fn formats(&self) -> Vec<u32> {
        let mut formats = Vec::new();
        let mut format = 0;
        loop {
            // SAFETY: the clipboard is open on this thread (`self`).
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                return formats;
            }
            formats.push(format);
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: balances the successful OpenClipboard in `Open::new`.
        let _ = unsafe { CloseClipboard() };
    }
}

/// Whether a format's data is a plain memory block we can copy. GDI handles
/// (bitmaps, metafiles, palettes), owner-drawn formats and the private and
/// GDI-object ranges aren't; images survive as CF_DIB / CF_DIBV5, from
/// which Windows synthesizes CF_BITMAP again.
fn is_memory_format(format: u32) -> bool {
    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_PALETTE: u32 = 9;
    const CF_ENHMETAFILE: u32 = 14;
    const CF_OWNERDISPLAY: u32 = 0x80;
    const CF_DSPBITMAP: u32 = 0x82;
    const CF_DSPMETAFILEPICT: u32 = 0x83;
    const CF_DSPENHMETAFILE: u32 = 0x8E;
    !matches!(
        format,
        CF_BITMAP
            | CF_METAFILEPICT
            | CF_PALETTE
            | CF_ENHMETAFILE
            | CF_OWNERDISPLAY
            | CF_DSPBITMAP
            | CF_DSPMETAFILEPICT
            | CF_DSPENHMETAFILE
            | 0x200..=0x3FF
    )
}

/// OLE's formats for the app that copied, which point into that app's
/// running copy rather than holding data: put back later, they can confuse
/// a paste in Explorer or Office. The data's other formats are kept.
fn is_live_object(format: u32) -> bool {
    let mut name = [0u16; 64];
    // SAFETY: a buffer of the given length; registered formats only.
    let len = unsafe { GetClipboardFormatNameW(format, &mut name) };
    let name = String::from_utf16_lossy(&name[..usize::try_from(len).unwrap_or(0)]);
    matches!(name.as_str(), "DataObject" | "Ole Private Data")
}

/// Everything copyable on the clipboard, format by format. `None` when the
/// clipboard couldn't be opened, in which case it's left alone afterward.
struct Snapshot(Option<Vec<(u32, Vec<u8>)>>);

fn snapshot() -> Snapshot {
    let Some(open) = Open::new() else {
        return Snapshot(None);
    };
    let items = open
        .formats()
        .into_iter()
        .filter(|&f| is_memory_format(f) && !is_live_object(f))
        .filter_map(|f| open.read(f).map(|bytes| (f, bytes)))
        .collect();
    Snapshot(Some(items))
}

fn restore(snap: &Snapshot) {
    let Some(items) = &snap.0 else {
        return;
    };
    let Some(open) = Open::new() else {
        return;
    };
    if open.empty() {
        for (format, bytes) in items {
            open.write(*format, bytes);
        }
        // It's the user's item, already in their clipboard history once:
        // putting it back mustn't add it again.
        transient_markers(&open);
    }
}

/// Formats that tell clipboard managers, Windows' clipboard history and
/// cloud clipboard to ignore our temporary contents.
fn transient_markers(open: &Open) {
    let zero = 0u32.to_ne_bytes();
    for (name, data) in [
        (
            w!("ExcludeClipboardContentFromMonitorProcessing"),
            &zero[..],
        ),
        (w!("CanIncludeInClipboardHistory"), &zero[..]),
        (w!("CanUploadToCloudClipboard"), &zero[..]),
    ] {
        // SAFETY: `name` is a NUL-terminated literal.
        let format = unsafe { RegisterClipboardFormatW(name) };
        if format != 0 {
            open.write(format, data);
        }
    }
}

/// Writes `text` as the only clipboard contents. `transient` marks it so
/// clipboard managers and history skip it.
fn write_text(text: &str, transient: bool) -> bool {
    let Some(open) = Open::new() else {
        return false;
    };
    if !open.empty() {
        return false;
    }
    let bytes: Vec<u8> = text
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_ne_bytes)
        .collect();
    let ok = open.write(CF_UNICODETEXT, &bytes);
    if transient {
        transient_markers(&open);
    }
    ok
}

/// The text on the clipboard, if any (for a preset's clipboard context).
pub(crate) fn clipboard_text() -> Option<String> {
    read_text()
}

fn read_text() -> Option<String> {
    let open = Open::new()?;
    let bytes = open.read(CF_UNICODETEXT)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_ne_bytes(*b))
        .collect();
    Some(from_wide(&units))
}

fn sequence() -> u32 {
    // SAFETY: takes no arguments.
    unsafe { GetClipboardSequenceNumber() }
}

// ---------------------------------------------------------------- keys

fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    // SAFETY: plain values.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) };
    // No extended-key flag for Return: the main one, not keypad Enter. The
    // arrows and their neighbours need it, or apps that read scan codes
    // (Remote Desktop, virtual machines) see the keypad's 6 for →.
    let mut flags = if up {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    if [
        VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_INSERT, VK_DELETE,
    ]
    .contains(&vk)
    {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    keyboard(KEYBDINPUT {
        wVk: vk,
        wScan: u16::try_from(scan).unwrap_or(0),
        dwFlags: flags,
        time: 0,
        dwExtraInfo: INJECTED_TAG,
    })
}

fn unicode_input(unit: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    keyboard(KEYBDINPUT {
        wVk: VIRTUAL_KEY(0),
        wScan: unit,
        dwFlags: flags,
        time: 0,
        dwExtraInfo: INJECTED_TAG,
    })
}

/// A media key (play/pause, next, previous), for voice actions: whichever
/// app is playing answers it.
pub(crate) fn press_media_key(key: crate::MediaKey) -> Result<()> {
    let vk = match key {
        crate::MediaKey::PlayPause => VK_MEDIA_PLAY_PAUSE,
        crate::MediaKey::Next => VK_MEDIA_NEXT_TRACK,
        crate::MediaKey::Previous => VK_MEDIA_PREV_TRACK,
    };
    let input = |up: bool| {
        keyboard(KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: if up {
                KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
            } else {
                KEYEVENTF_EXTENDEDKEY
            },
            time: 0,
            dwExtraInfo: INJECTED_TAG,
        })
    };
    send(&[input(false), input(true)])
}

fn keyboard(ki: KEYBDINPUT) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki },
    }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    // SAFETY: `inputs` is a slice of fully initialized keyboard INPUTs and
    // the size argument is the size of one INPUT, as required.
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(PlatformError::Failed(
            "Windows blocked the synthetic key presses".into(),
        ))
    }
}

/// Waits (briefly) until the user lets go of physical modifier keys, so a
/// synthetic Ctrl+V isn't seen as Ctrl+Shift+V or Ctrl+Alt+V. Keys our hook
/// swallowed never reach the key state, so they don't count.
fn wait_for_modifiers_released() {
    let deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < deadline {
        let held = [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            // SAFETY: plain value; the high bit means the key is down now.
            .any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0);
        if !held {
            return;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Fails if the foreground app runs at a higher integrity level (as
/// administrator) than we do. Windows (UIPI) would silently drop our key
/// events, so we report it and the caller copies the text instead.
fn check_not_blocked() -> Result<()> {
    if process_elevated(None) == Some(true) {
        return Ok(());
    }
    // SAFETY: takes no arguments; may return a null window.
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return Ok(());
    }
    let mut pid = 0u32;
    // SAFETY: `window` is a window handle (a stale one just fails) and `pid`
    // a valid out-pointer.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    if pid == 0 || process_elevated(Some(pid)) != Some(true) {
        return Ok(());
    }
    Err(PlatformError::Failed(
        "the app runs as administrator, so Windows blocks typing into it".into(),
    ))
}

/// Whether a process (ours for `None`) runs elevated. A token we're not
/// allowed to read belongs to a more privileged process, which counts as
/// elevated here; other failures are `None`.
fn process_elevated(pid: Option<u32>) -> Option<bool> {
    let (process, owned) = match pid {
        // SAFETY: returns a pseudo-handle that needs no closing.
        None => (unsafe { GetCurrentProcess() }, false),
        // SAFETY: plain values; the handle is closed below.
        Some(pid) => match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
            Ok(h) => (h, true),
            Err(_) => return None,
        },
    };
    let mut token = HANDLE::default();
    // SAFETY: `process` is a live process handle and `token` a valid
    // out-pointer; the token is closed below.
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
    let result = match opened {
        Err(e) if e.code() == E_ACCESSDENIED => Some(true),
        Err(_) => None,
        Ok(()) => {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            // SAFETY: `token` is open with TOKEN_QUERY; the buffer is a
            // TOKEN_ELEVATION of exactly the size passed, and `len` a valid
            // out-pointer.
            let ok = unsafe {
                GetTokenInformation(
                    token,
                    TokenElevation,
                    Some((&raw mut elevation).cast()),
                    std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                    &mut len,
                )
            };
            // SAFETY: `token` was opened above and is closed once.
            let _ = unsafe { CloseHandle(token) };
            ok.ok().map(|()| elevation.TokenIsElevated != 0)
        }
    };
    if owned {
        // SAFETY: `process` was opened above and is closed once.
        let _ = unsafe { CloseHandle(process) };
    }
    result
}

/// Presses `key`, with `modifier` held if given.
fn post_key(modifier: Option<VIRTUAL_KEY>, key: VIRTUAL_KEY) -> Result<()> {
    wait_for_modifiers_released();
    let mut inputs = Vec::with_capacity(4);
    if let Some(m) = modifier {
        inputs.push(key_input(m, false));
    }
    inputs.push(key_input(key, false));
    inputs.push(key_input(key, true));
    if let Some(m) = modifier {
        inputs.push(key_input(m, true));
    }
    send(&inputs)
}

pub(crate) fn paste_text(text: &str) -> Result<()> {
    check_not_blocked()?;
    let saved = snapshot();
    if !write_text(text, true) {
        return Err(PlatformError::Failed(
            "couldn't write to the clipboard".into(),
        ));
    }
    let ours = sequence();
    // Terminals that take Ctrl+V as a key of its own (it would insert ^V)
    // paste with Shift+Insert, as they document.
    let sent = if foreground_class().is_some_and(|c| SHIFT_INSERT_CLASSES.contains(&c.as_str())) {
        post_key(Some(VK_SHIFT), VK_INSERT)
    } else {
        post_key(Some(VK_CONTROL), VK_V)
    };
    if sent.is_ok() {
        std::thread::sleep(PASTE_SETTLE);
    }
    // Only restore if nobody (the user, another app) changed it meanwhile.
    if sequence() == ours {
        restore(&saved);
    }
    sent
}

/// A key combination ("ctrl+shift+n", "right", "f5"); Command or Super is
/// the Windows key. Letters and symbols go by the keyboard layout's key
/// for them.
pub(crate) fn press_combo(combo: &splashdown_core::computer::Combo) -> Result<()> {
    use splashdown_core::computer::Key;
    check_not_blocked()?;
    wait_for_modifiers_released();
    let unknown = || PlatformError::Failed(format!("no key for {:?}", combo.key));
    let vk = match combo.key {
        Key::Named(name) => VIRTUAL_KEY(match name {
            "return" => 0x0D,
            "tab" => 0x09,
            "space" => 0x20,
            "backspace" => 0x08,
            "escape" => 0x1B,
            "delete" => 0x2E,
            "home" => 0x24,
            "end" => 0x23,
            "pageup" => 0x21,
            "pagedown" => 0x22,
            "left" => 0x25,
            "up" => 0x26,
            "right" => 0x27,
            "down" => 0x28,
            f => f
                .strip_prefix('f')
                .and_then(|n| n.parse::<u16>().ok())
                .filter(|n| (1..=12).contains(n))
                .map(|n| 0x6F + n)
                .ok_or_else(unknown)?,
        }),
        Key::Char(c) => {
            let unit = u16::try_from(u32::from(c)).map_err(|_| unknown())?;
            // SAFETY: a plain value; -1 when the layout has no key for it.
            let scan = unsafe { VkKeyScanW(unit) };
            if scan == -1 {
                return Err(unknown());
            }
            VIRTUAL_KEY((scan & 0xFF) as u16)
        }
    };
    let mods: Vec<VIRTUAL_KEY> = [
        (combo.mods.ctrl, VK_CONTROL),
        (combo.mods.alt, VK_MENU),
        (combo.mods.shift, VK_SHIFT),
        (combo.mods.cmd, VK_LWIN),
    ]
    .into_iter()
    .filter_map(|(on, vk)| on.then_some(vk))
    .collect();
    let mut inputs: Vec<INPUT> = mods.iter().map(|&m| key_input(m, false)).collect();
    inputs.push(key_input(vk, false));
    inputs.push(key_input(vk, true));
    inputs.extend(mods.iter().rev().map(|&m| key_input(m, true)));
    send(&inputs)
}

pub(crate) fn press(action: crate::KeyAction) -> Result<()> {
    check_not_blocked()?;
    match action {
        crate::KeyAction::Return => post_key(None, VK_RETURN),
        crate::KeyAction::Undo => post_key(Some(VK_CONTROL), VK_Z),
        crate::KeyAction::Backspace => post_key(None, VK_BACK),
        crate::KeyAction::DeleteWord => post_key(Some(VK_CONTROL), VK_BACK),
        crate::KeyAction::Bold => post_key(Some(VK_CONTROL), VK_B),
        crate::KeyAction::Italic => post_key(Some(VK_CONTROL), VK_I),
        crate::KeyAction::Right => post_key(None, VK_RIGHT),
    }
}

pub(crate) fn type_text(text: &str, soft_breaks: bool) -> Result<()> {
    check_not_blocked()?;
    wait_for_modifiers_released();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            if soft_breaks {
                send(&[
                    key_input(VK_SHIFT, false),
                    key_input(VK_RETURN, false),
                    key_input(VK_RETURN, true),
                    key_input(VK_SHIFT, true),
                ])?;
            } else {
                send(&[key_input(VK_RETURN, false), key_input(VK_RETURN, true)])?;
            }
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        let chars: Vec<char> = line.chars().collect();
        // Chunks of whole characters, so surrogate pairs stay together.
        for chunk in chars.chunks(20) {
            let mut inputs = Vec::with_capacity(chunk.len() * 4);
            for c in chunk {
                let mut buf = [0u16; 2];
                for &unit in c.encode_utf16(&mut buf).iter() {
                    inputs.push(unicode_input(unit, false));
                    inputs.push(unicode_input(unit, true));
                }
            }
            send(&inputs)?;
            // Give the target app time to consume each chunk in order.
            std::thread::sleep(Duration::from_millis(4));
        }
    }
    Ok(())
}

pub(crate) fn copy_to_clipboard(text: &str) -> Result<()> {
    if write_text(text, false) {
        Ok(())
    } else {
        Err(PlatformError::Failed(
            "couldn't write to the clipboard".into(),
        ))
    }
}

/// Console windows: classic conhost and Windows Terminal. Ctrl+C there
/// interrupts the running program instead of copying.
const CONSOLE_CLASSES: &[&str] = &[
    "ConsoleWindowClass",
    "CASCADIA_HOSTING_WINDOW_CLASS",
    "PseudoConsoleWindow",
    "mintty",
    "VirtualConsoleClass",
    "PuTTY",
];

/// Git Bash's mintty and PuTTY: Ctrl+V goes to the program running in them.
const SHIFT_INSERT_CLASSES: &[&str] = &["mintty", "PuTTY"];

/// The class name of the window in front.
fn foreground_class() -> Option<String> {
    // SAFETY: takes no arguments; may return a null window.
    let window: HWND = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return None;
    }
    let mut buf = [0u16; 128];
    // SAFETY: `window` is a window handle (a stale one just fails) and the
    // buffer's length is passed with it.
    let len = unsafe { GetClassNameW(window, &mut buf) };
    Some(from_wide(&buf[..usize::try_from(len).unwrap_or(0)]))
}

fn foreground_is_console() -> bool {
    foreground_class().is_some_and(|class| CONSOLE_CLASSES.contains(&class.as_str()))
}

pub(crate) fn copy_selection() -> Option<String> {
    if foreground_is_console() {
        return None;
    }
    if check_not_blocked().is_err() {
        return None;
    }
    let saved = snapshot();
    let sentinel = format!("splashdown-copy-probe-{}", std::process::id());
    write_text(&sentinel, true);
    let before = sequence();

    let mut copied = None;
    if post_key(Some(VK_CONTROL), VK_C).is_ok() {
        let started = Instant::now();
        while started.elapsed() < COPY_TIMEOUT {
            std::thread::sleep(Duration::from_millis(10));
            if sequence() != before {
                // The app may still be writing; reading retries the open.
                copied = read_text();
                break;
            }
        }
    }
    restore(&saved);
    copied.filter(|s| !s.is_empty() && *s != sentinel)
}
