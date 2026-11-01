//! UI Automation: the focused app and field, selected text, password
//! fields and page URLs; plus the permission answers for Windows.

use windows::Win32::Foundation::ERROR_NO_MORE_ITEMS;
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HWND, LPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_QWORD, RRF_RT_REG_SZ,
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextRange, IUIAutomationValuePattern, TextPatternRangeEndpoint_End,
    TextPatternRangeEndpoint_Start, TextUnit_Character, UIA_DocumentControlTypeId,
    UIA_TextPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, EnumWindows, GetForegroundWindow, GetIconInfo, GetWindowTextW,
    GetWindowThreadProcessId, HICON, ICONINFO, IsIconic, IsWindowVisible, PrivateExtractIconsW,
    SW_RESTORE, SetForegroundWindow, ShowWindow,
};
use windows::core::{BOOL, Interface, PCWSTR, PWSTR};

use super::{from_wide, wide};
use crate::{Access, AppInfo, Permissions, PlatformError, Selection};

/// Bound how long a hung app can block a UI Automation call (ms).
const UIA_TRANSACTION_TIMEOUT: u32 = 500;
const UIA_CONNECTION_TIMEOUT: u32 = 1_000;

/// Longer field contents aren't read (large documents).
const MAX_FIELD_CHARS: usize = 20_000;

thread_local! {
    /// This thread's UI Automation client, if the thread is (or could join)
    /// the multithreaded apartment. `None` on single-threaded-apartment
    /// threads (a UI thread), where we don't use UI Automation at all.
    static UIA: Option<IUIAutomation> = create_client();
}

fn create_client() -> Option<IUIAutomation> {
    // SAFETY: no reserved pointer. Joins the MTA, or fails with
    // RPC_E_CHANGED_MODE on a thread that's already single-threaded. It's
    // deliberately never balanced with CoUninitialize: the thread may use
    // COM until it exits, and keeping the MTA alive is what lets threads
    // that never initialized COM release our elements safely (see `Element`).
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_err() {
        tracing::debug!("UI Automation unavailable on this thread: {hr:?}");
        return None;
    }
    // SAFETY: CUIAutomation is the documented in-process class implementing
    // IUIAutomation; no aggregation.
    let uia: IUIAutomation =
        match unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) } {
            Ok(uia) => uia,
            Err(e) => {
                tracing::warn!("couldn't create the UI Automation client: {e}");
                return None;
            }
        };
    if let Ok(uia2) = uia.cast::<IUIAutomation2>() {
        // SAFETY: plain values on a live client object.
        unsafe {
            let _ = uia2.SetTransactionTimeout(UIA_TRANSACTION_TIMEOUT);
            let _ = uia2.SetConnectionTimeout(UIA_CONNECTION_TIMEOUT);
        }
    }
    Some(uia)
}

/// Runs `f` with this thread's UI Automation client, if it has one.
fn with_uia<T>(f: impl FnOnce(&IUIAutomation) -> Option<T>) -> Option<T> {
    UIA.with(|uia| uia.as_ref().and_then(f))
}

/// A UI Automation element that may move between threads.
struct Element(IUIAutomationElement);
// SAFETY: elements are only obtained and used on threads that joined the
// multithreaded apartment (`with_uia` / `create_client` refuse other
// threads), and COM lets MTA interface pointers be used from any MTA thread
// without marshaling. A thread that never initialized COM is implicitly in
// the MTA while it exists, which our threads keep alive by never
// uninitializing, so dropping (releasing) an element there is also fine.
unsafe impl Send for Element {}
// SAFETY: see `Send`; `&self` only calls the element's thread-safe COM
// methods and there's no Rust-side interior mutability.
unsafe impl Sync for Element {}

pub(crate) struct FocusElement {
    element: Option<Element>,
    /// The foreground window, as an integer so this type stays `Send`.
    window: isize,
}

// The app keeps a `Focus` on its session thread, which needs `Send`; the app
// itself can't be cross-checked from a Mac, so assert it here.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<FocusElement>();
};

fn window_pid(window: HWND) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: `window` is a window handle (a stale one just fails) and `pid`
    // a valid out-pointer.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

fn window_title(window: HWND) -> Option<String> {
    let mut buf = [0u16; 512];
    // SAFETY: `window` is a window handle and the buffer's length is passed
    // with it.
    let len = unsafe { GetWindowTextW(window, &mut buf) };
    let title = from_wide(&buf[..usize::try_from(len).unwrap_or(0)]);
    (!title.trim().is_empty()).then_some(title)
}

/// Full path of a process's executable.
pub(crate) fn exe_path(pid: u32) -> Option<String> {
    // SAFETY: plain values; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `process` is open with the access this needs; `buf` holds
    // `len` UTF-16 units and `len` is a valid in/out pointer.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    // SAFETY: `process` was opened above and is closed once.
    let _ = unsafe { CloseHandle(process) };
    ok.ok()?;
    Some(from_wide(&buf[..(len as usize).min(buf.len())]))
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// The executable's "File description" (e.g. "Google Chrome"), the closest
/// Windows has to an app's display name.
pub(crate) fn file_description(path: &str) -> Option<String> {
    let path_w = wide(path);
    // SAFETY: `path_w` is a NUL-terminated path that outlives the call.
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(path_w.as_ptr()), None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` is writable for the `size` bytes passed.
    unsafe {
        GetFileVersionInfoW(
            PCWSTR(path_w.as_ptr()),
            None,
            size,
            data.as_mut_ptr().cast(),
        )
    }
    .ok()?;

    let query = |sub: &str| -> Option<(*const u8, usize)> {
        let sub = wide(sub);
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        // SAFETY: `data` is the version block filled in above and `sub` a
        // NUL-terminated query; `ptr` and `len` are valid out-pointers. The
        // result points into `data`.
        let ok = unsafe {
            VerQueryValueW(
                data.as_ptr().cast(),
                PCWSTR(sub.as_ptr()),
                &mut ptr,
                &mut len,
            )
        };
        (ok.as_bool() && !ptr.is_null()).then_some((ptr.cast_const().cast::<u8>(), len as usize))
    };

    // The first language/code page pair, else US English / Unicode.
    let translation = query("\\VarFileInfo\\Translation")
        .filter(|&(_, len)| len >= 4)
        .map(|(ptr, _)| {
            // SAFETY: at least 4 bytes inside `data`; read unaligned since
            // VerQueryValueW makes no alignment promise.
            let pair = unsafe { std::ptr::read_unaligned(ptr.cast::<[u16; 2]>()) };
            format!("{:04x}{:04x}", pair[0], pair[1])
        })
        .unwrap_or_else(|| "040904b0".into());
    let (ptr, len) = query(&format!("\\StringFileInfo\\{translation}\\FileDescription"))?;
    // `len` counts UTF-16 units, including the terminator.
    let units: Vec<u16> = (0..len)
        // SAFETY: the string's `len` units lie inside `data`; read
        // unaligned for the same reason as above.
        .map(|i| unsafe { std::ptr::read_unaligned(ptr.cast::<u16>().add(i)) })
        .collect();
    let name = from_wide(&units).trim().to_string();
    (!name.is_empty()).then_some(name)
}

pub(crate) fn capture_focus() -> Option<(AppInfo, FocusElement)> {
    // SAFETY: takes no arguments; may return a null window.
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return None;
    }
    let window_pid = window_pid(window)?;
    let window_exe = exe_path(window_pid);

    // SAFETY: a plain call on this thread's live client.
    let element = with_uia(|uia| unsafe { uia.GetFocusedElement() }.ok());
    // SAFETY: a property read on a live element.
    let element_pid = element
        .as_ref()
        .and_then(|el| unsafe { el.CurrentProcessId() }.ok())
        .and_then(|p| u32::try_from(p).ok());
    // Store apps' windows belong to ApplicationFrameHost.exe while their
    // content (and focus) is in the app's own process.
    let frame_host = window_exe
        .as_deref()
        .is_some_and(|p| file_name(p).eq_ignore_ascii_case("ApplicationFrameHost.exe"));
    let (pid, element, exe) = match element_pid {
        Some(p) if p == window_pid => (p, element, window_exe),
        Some(p) if frame_host => (p, element, exe_path(p)),
        // Focus is somewhere other than the foreground window (or
        // unknown): only trust the window.
        _ => (window_pid, None, window_exe),
    };

    let id = exe
        .as_deref()
        .map(file_name)
        .unwrap_or_default()
        .to_string();
    let name = exe
        .as_deref()
        .and_then(file_description)
        .unwrap_or_else(|| id.strip_suffix(".exe").unwrap_or(&id).to_string());
    let url = element.as_ref().and_then(|el| page_url(el, pid));

    Some((
        AppInfo {
            id,
            name,
            pid: pid as i32,
            window_title: window_title(window),
            url,
            // Not read on Windows yet (no working folder or open file).
            document: None,
        },
        FocusElement {
            element: element.map(Element),
            window: window.0 as isize,
        },
    ))
}

/// The URL of the web page containing `el`: walk up to the enclosing
/// document and read its value, which Chromium browsers and Firefox set to
/// the page address. Best effort; other apps' documents hold their text,
/// which the URL check filters out.
fn page_url(el: &IUIAutomationElement, pid: u32) -> Option<String> {
    const MAX_DEPTH: usize = 40;
    with_uia(|uia| {
        // SAFETY: plain calls on this thread's live client.
        let walker = unsafe { uia.RawViewWalker() }.ok()?;
        let mut node = el.clone();
        for _ in 0..MAX_DEPTH {
            // SAFETY: property reads on a live element.
            let (node_pid, kind) = unsafe {
                (
                    node.CurrentProcessId().ok()?,
                    node.CurrentControlType().ok()?,
                )
            };
            if u32::try_from(node_pid).ok() != Some(pid) {
                return None;
            }
            if kind == UIA_DocumentControlTypeId {
                // SAFETY: pattern and property reads on a live element.
                let value = unsafe {
                    node.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                        .and_then(|p| p.CurrentValue())
                };
                if let Ok(value) = value {
                    let value = value.to_string();
                    if looks_like_url(&value) {
                        return Some(value);
                    }
                }
            }
            // SAFETY: `node` is a live element.
            node = unsafe { walker.GetParentElement(&node) }.ok()?;
        }
        None
    })
}

fn looks_like_url(s: &str) -> bool {
    s.len() < 8_192 && !s.contains(char::is_whitespace) && s.contains("://")
}

/// The foreground window's app, named like `capture_focus` names it.
pub(crate) fn frontmost_app() -> Option<(i32, String, String)> {
    // SAFETY: takes no arguments; may return a null window.
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return None;
    }
    let pid = window_pid(window)?;
    let path = exe_path(pid);
    let id = path
        .as_deref()
        .map(|p| file_name(p).to_string())
        .unwrap_or_default();
    let name = path
        .map(|path| {
            file_description(&path).unwrap_or_else(|| {
                let file = file_name(&path);
                file.strip_suffix(".exe").unwrap_or(file).to_string()
            })
        })
        .unwrap_or_default();
    Some((pid as i32, name, id))
}

/// Brings the app's frontmost visible window forward. Windows allows this
/// because the foreground process (us) is the one asking.
pub(crate) fn activate_app(pid: i32) -> crate::Result<()> {
    let window = u32::try_from(pid)
        .ok()
        .and_then(top_window)
        .ok_or_else(|| PlatformError::Failed("that app has no open window".into()))?;
    // SAFETY: `window` came from EnumWindows just now; one closed since
    // makes these calls fail, nothing worse.
    let shown = unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window).as_bool()
    };
    if shown {
        Ok(())
    } else {
        Err(PlatformError::Failed("couldn't switch to that app".into()))
    }
}

/// The first visible top-level window of `pid` in Z order (its frontmost).
fn top_window(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` points at the `Search` below, which outlives
        // EnumWindows.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        // SAFETY: `window` is a live handle from the enumeration.
        let visible = unsafe { IsWindowVisible(window) }.as_bool();
        if visible && window_pid(window) == Some(search.pid) {
            search.found = Some(window);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut search = Search { pid, found: None };
    // SAFETY: `visit` matches WNDENUMPROC and `search` lives until
    // EnumWindows returns. Stopping early reports an error, which is ignored.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&raw mut search as isize)) };
    search.found
}

pub(crate) fn is_current(app: &AppInfo, focus: &FocusElement) -> bool {
    // SAFETY: takes no arguments; may return a null window.
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() || window.0 as isize != focus.window {
        return false;
    }
    let Some(original) = &focus.element else {
        // Same window; nothing finer to compare.
        return window_pid(window).is_some();
    };
    with_uia(|uia| {
        // SAFETY: plain calls on this thread's live client and elements.
        let now = unsafe { uia.GetFocusedElement() }.ok()?;
        // SAFETY: a property read on a live element.
        if unsafe { now.CurrentProcessId() }.ok() != Some(app.pid) {
            return Some(false);
        }
        // SAFETY: both are live elements.
        unsafe { uia.CompareElements(&original.0, &now) }
            .ok()
            .map(|same| same.as_bool())
    })
    // Same window, but the field can't be read now: assume it's unchanged.
    .unwrap_or(true)
}

/// The element, when this thread may use it (see `Element`).
fn usable(focus: &FocusElement) -> Option<&IUIAutomationElement> {
    let el = focus.element.as_ref()?;
    with_uia(|_| Some(()))?;
    Some(&el.0)
}

pub(crate) fn selection(focus: &FocusElement) -> Selection {
    let Some(el) = usable(focus) else {
        return Selection::Unknown;
    };
    // SAFETY: pattern reads on a live element, on an MTA thread.
    let Ok(ranges) = (unsafe {
        el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .and_then(|p| p.GetSelection())
    }) else {
        return Selection::Unknown;
    };
    // SAFETY: a live range array.
    let count = unsafe { ranges.Length() }.unwrap_or(0);
    let mut text = String::new();
    for i in 0..count {
        // SAFETY: `i` is within the array's length; -1 means no limit.
        if let Ok(s) = unsafe { ranges.GetElement(i).and_then(|r| r.GetText(-1)) } {
            text.push_str(&s.to_string());
        }
    }
    if text.is_empty() {
        Selection::Empty
    } else {
        Selection::Text(text)
    }
}

pub(crate) fn field_text(focus: &FocusElement) -> Option<String> {
    if is_secure_field(focus) {
        return None;
    }
    let el = usable(focus)?;
    let limit = i32::try_from(MAX_FIELD_CHARS + 1).unwrap_or(i32::MAX);
    // SAFETY: pattern reads on a live element, on an MTA thread.
    let text = unsafe {
        el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .and_then(|p| p.DocumentRange())
            .and_then(|r| r.GetText(limit))
            .or_else(|_| {
                el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                    .and_then(|p| p.CurrentValue())
            })
    }
    .ok()?
    .to_string();
    (text.chars().count() <= MAX_FIELD_CHARS).then_some(text)
}

/// The field's text pattern, when it has one and this thread may use it.
fn text_pattern(focus: &FocusElement) -> Option<IUIAutomationTextPattern> {
    let el = usable(focus)?;
    // SAFETY: a pattern read on a live element, on an MTA thread.
    unsafe { el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }.ok()
}

/// Where the insertion point is, in UTF-16 units from the start of the
/// field's text (the start of the selection, if there is one).
pub(crate) fn cursor_offset(focus: &FocusElement) -> Option<usize> {
    if is_secure_field(focus) {
        return None;
    }
    let pattern = text_pattern(focus)?;
    let limit = i32::try_from(MAX_FIELD_CHARS + 1).unwrap_or(i32::MAX);
    // SAFETY: calls on a live pattern and the ranges it returns; index 0 is
    // checked against the array's length first.
    let before = unsafe {
        let selection = pattern.GetSelection().ok()?;
        if selection.Length().ok()? < 1 {
            return None;
        }
        let caret = selection.GetElement(0).ok()?;
        // The document's start up to the caret.
        let before = pattern.DocumentRange().ok()?;
        before
            .MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &caret,
                TextPatternRangeEndpoint_Start,
            )
            .ok()?;
        before.GetText(limit).ok()?.to_string()
    };
    (before.chars().count() <= MAX_FIELD_CHARS).then(|| before.encode_utf16().count())
}

/// Selects `length` UTF-16 units starting at `location` in the field's text.
/// Text ranges move by characters, so the offsets are converted, and the
/// result is checked against the text it should hold: a field that counts
/// differently (line breaks, emoji) gets nothing selected rather than the
/// wrong text.
pub(crate) fn select_range(focus: &FocusElement, location: usize, length: usize) -> bool {
    let Some(text) = field_text(focus) else {
        return false;
    };
    let Some(pattern) = text_pattern(focus) else {
        return false;
    };
    let units: Vec<u16> = text.encode_utf16().collect();
    let (Some(head), Some(wanted)) = (
        units.get(..location),
        units.get(location..location.saturating_add(length)),
    ) else {
        return false;
    };
    let chars = |u: &[u16]| char::decode_utf16(u.iter().copied()).count();
    let (Ok(start), Ok(count)) = (i32::try_from(chars(head)), i32::try_from(chars(wanted))) else {
        return false;
    };
    let wanted = String::from_utf16_lossy(wanted);
    // SAFETY: calls on a live pattern and the range it returns.
    let range = unsafe {
        let Ok(range) = pattern.DocumentRange() else {
            return false;
        };
        let collapsed = range
            .MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &range,
                TextPatternRangeEndpoint_Start,
            )
            .is_ok();
        let moved = range.Move(TextUnit_Character, start).ok() == Some(start);
        let grown = range
            .MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, count)
            .ok()
            == Some(count);
        if !(collapsed && moved && grown) {
            return false;
        }
        range
    };
    if !range_holds(&range, &wanted) {
        tracing::debug!("the field counts characters differently; not selecting");
        return false;
    }
    // SAFETY: a live range.
    unsafe { range.Select() }.is_ok()
}

fn range_holds(range: &IUIAutomationTextRange, wanted: &str) -> bool {
    // SAFETY: a live range; -1 means no length limit.
    unsafe { range.GetText(-1) }.is_ok_and(|t| t == wanted)
}

pub(crate) fn is_secure_field(focus: &FocusElement) -> bool {
    usable(focus)
        // SAFETY: a property read on a live element, on an MTA thread.
        .and_then(|el| unsafe { el.CurrentIsPassword() }.ok())
        .is_some_and(|b| b.as_bool())
}

/// Windows has no system-wide secure input mode; password fields are
/// caught one by one with `is_secure_field`.
pub(crate) fn secure_input_active() -> bool {
    false
}

pub(crate) fn permissions() -> Permissions {
    Permissions {
        // Hooks, UI Automation and SendInput need no grant on Windows.
        // Elevated target apps are handled when inserting (UIPI).
        accessibility: true,
        microphone: microphone_access(),
    }
}

/// Reading the microphone privacy status properly needs WinRT
/// (`AppCapability`), so this reads the settings the privacy page writes:
/// the device-wide switch, "Microphone access" for apps, and "Let desktop
/// apps access your microphone". Anything other than an explicit "Deny"
/// counts as granted; desktop apps are never asked.
fn microphone_access() -> Access {
    const STORE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";
    let denied = consent_denied(HKEY_LOCAL_MACHINE, STORE)
        || consent_denied(HKEY_CURRENT_USER, STORE)
        || consent_denied(HKEY_CURRENT_USER, &format!("{STORE}\\NonPackaged"));
    if denied {
        Access::Denied
    } else {
        Access::Granted
    }
}

fn consent_denied(root: HKEY, subkey: &str) -> bool {
    reg_string(root, subkey, Some("Value")).is_some_and(|v| v.eq_ignore_ascii_case("Deny"))
}

/// Apps using a microphone now. Windows' privacy records keep, per app,
/// when it last started and stopped using one; a stop time of 0 means it
/// still is. Packaged apps are listed by package family name
/// (`MSTeams_8wekyb3d8bbwe`), others by their executable's file name
/// (`Zoom.exe`).
pub(crate) fn microphone_users() -> Vec<String> {
    const STORE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let mut users = Vec::new();
    for name in reg_subkeys(HKEY_CURRENT_USER, STORE) {
        if name.eq_ignore_ascii_case("NonPackaged") {
            let apps = format!("{STORE}\\NonPackaged");
            for app in reg_subkeys(HKEY_CURRENT_USER, &apps) {
                // The path, with # for each backslash.
                let file = app.rsplit('#').next().unwrap_or(&app).to_string();
                if !file.eq_ignore_ascii_case(&me) && in_use(&format!("{apps}\\{app}")) {
                    users.push(file);
                }
            }
        } else if in_use(&format!("{STORE}\\{name}")) {
            users.push(name);
        }
    }
    users
}

/// Whether the app with this consent record is using the microphone now.
fn in_use(subkey: &str) -> bool {
    reg_qword(HKEY_CURRENT_USER, subkey, "LastUsedTimeStop") == Some(0)
        && reg_qword(HKEY_CURRENT_USER, subkey, "LastUsedTimeStart").is_some_and(|t| t > 0)
}

fn reg_qword(root: HKEY, subkey: &str, value: &str) -> Option<u64> {
    let subkey = wide(subkey);
    let value = wide(value);
    let mut data = 0u64;
    let mut size = std::mem::size_of::<u64>() as u32;
    // SAFETY: NUL-terminated strings that outlive the call; `data` is
    // writable for the `size` bytes passed.
    let err = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_QWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut size),
        )
    };
    (err == ERROR_SUCCESS).then_some(data)
}

/// The names of a registry key's subkeys (none if it can't be opened).
fn reg_subkeys(root: HKEY, subkey: &str) -> Vec<String> {
    let subkey = wide(subkey);
    let mut key = HKEY::default();
    // SAFETY: a NUL-terminated name that outlives the call and a writable
    // key handle, closed below.
    let opened =
        unsafe { RegOpenKeyExW(root, PCWSTR(subkey.as_ptr()), Some(0), KEY_READ, &mut key) };
    if opened != ERROR_SUCCESS {
        return Vec::new();
    }
    let mut names = Vec::new();
    let mut buf = [0u16; 512];
    for index in 0.. {
        let mut len = buf.len() as u32;
        // SAFETY: `buf` is writable for `len` characters; the rest is unused.
        let err = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if err == ERROR_NO_MORE_ITEMS {
            break;
        }
        if err == ERROR_SUCCESS {
            names.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
    }
    // SAFETY: opened above.
    let _ = unsafe { RegCloseKey(key) };
    names
}

/// A string value from the registry (`None` for the key's default value).
/// Expandable strings come back expanded.
fn reg_string(root: HKEY, subkey: &str, value: Option<&str>) -> Option<String> {
    let subkey = wide(subkey);
    let value = value.map(wide);
    let mut buf = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: `subkey` and `value` are NUL-terminated strings that outlive
    // the call (a null value name means the default value); `buf` is
    // writable for the `size` bytes passed, and `size` is a valid in/out
    // pointer.
    let err = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            value
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (err == ERROR_SUCCESS).then(|| from_wide(&buf))
}

/// "11 24H2 (build 26100)", for feedback reports. The registry's
/// ProductName still says "Windows 10" on Windows 11, so the build number
/// decides.
pub(crate) fn os_version() -> Option<String> {
    const KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
    let build = reg_string(HKEY_LOCAL_MACHINE, KEY, Some("CurrentBuild"))?;
    let major = if build.parse::<u32>().is_ok_and(|b| b >= 22_000) {
        "11"
    } else {
        "10"
    };
    Some(
        match reg_string(HKEY_LOCAL_MACHINE, KEY, Some("DisplayVersion")) {
            Some(release) => format!("{major} {release} (build {build})"),
            None => format!("{major} (build {build})"),
        },
    )
}

/// Browsers built on Chromium, by executable name. Their text fields
/// sometimes drop a paste, so the session checks and retries.
const CHROMIUM_EXES: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "brave.exe",
    "vivaldi.exe",
    "opera.exe",
    "arc.exe",
    "chromium.exe",
];

pub(crate) fn is_chromium(app_id: &str) -> bool {
    CHROMIUM_EXES
        .iter()
        .any(|exe| exe.eq_ignore_ascii_case(app_id))
}

/// The display name of an app known by its executable name: from its "App
/// Paths" registration, else from a running copy. No icon yet; Settings
/// shows a letter instead.
pub(crate) fn app_display(app_id: &str) -> Option<crate::AppDisplay> {
    let key = format!("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{app_id}");
    let path = reg_string(HKEY_CURRENT_USER, &key, None)
        .or_else(|| reg_string(HKEY_LOCAL_MACHINE, &key, None))
        .map(|p| p.trim_matches('"').to_string())
        .or_else(|| running_exe(app_id))?;
    Some(crate::AppDisplay {
        name: file_description(&path)?,
        icon_png: icon_png(&path),
    })
}

/// Pixels per side of an app icon in Settings: sharp on high-DPI screens.
const ICON_PX: i32 = 64;

/// The executable's own icon as a PNG.
fn icon_png(exe: &str) -> Option<Vec<u8>> {
    let mut name = [0u16; 260];
    let wide: Vec<u16> = exe.encode_utf16().collect();
    name.get_mut(..wide.len())?.copy_from_slice(&wide);
    if wide.len() == name.len() {
        return None;
    }
    let mut icon = [HICON::default()];
    // SAFETY: `name` is a NUL-terminated path; one handle is written into
    // `icon`, destroyed below.
    let found =
        unsafe { PrivateExtractIconsW(&name, 0, ICON_PX, ICON_PX, Some(&mut icon), None, 0) };
    if found == 0 || found == u32::MAX || icon[0].is_invalid() {
        return None;
    }
    let pixels = icon_pixels(icon[0]);
    // SAFETY: extracted above, not used after this.
    let _ = unsafe { DestroyIcon(icon[0]) };
    encode_png(&pixels?, ICON_PX as u32)
}

/// An icon's pixels, RGBA, top row first.
fn icon_pixels(icon: HICON) -> Option<Vec<u8>> {
    let mut info = ICONINFO::default();
    // SAFETY: `info` is writable; the bitmaps it gets are deleted below.
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;
    let mut pixels = vec![0u8; (ICON_PX * ICON_PX * 4) as usize];
    // The mask: white where an icon without alpha is see-through.
    let mut mask = vec![0u8; pixels.len()];
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ICON_PX,
            // Negative: top row first.
            biHeight: -ICON_PX,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: a memory DC for the copy, deleted right after; `pixels`
    // holds the 32-bit rows `bmi` describes.
    let (lines, mask_lines) = unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            info.hbmColor,
            0,
            ICON_PX as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        let mut mask_bmi = bmi;
        let mask_lines = GetDIBits(
            dc,
            info.hbmMask,
            0,
            ICON_PX as u32,
            Some(mask.as_mut_ptr().cast()),
            &mut mask_bmi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        let _ = DeleteObject(info.hbmColor.into());
        let _ = DeleteObject(info.hbmMask.into());
        (lines, mask_lines)
    };
    if lines != ICON_PX {
        return None;
    }
    // BGRA to RGBA. An icon with no alpha at all (an older one) is opaque
    // except where its mask is white.
    let has_alpha = pixels.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
    let masked = mask_lines == ICON_PX;
    for (p, m) in pixels
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(mask.as_chunks::<4>().0)
    {
        p.swap(0, 2);
        if !has_alpha {
            p[3] = if masked && m[0] != 0 { 0 } else { 255 };
        }
    }
    Some(pixels)
}

fn encode_png(rgba: &[u8], side: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, side, side);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(rgba).ok()?;
    Some(out)
}

/// The full path of a running app with this executable name, found through
/// the top-level windows.
fn running_exe(app_id: &str) -> Option<String> {
    struct Search<'a> {
        app_id: &'a str,
        found: Option<String>,
    }
    unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` points at the `Search` below, which outlives
        // EnumWindows.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        // SAFETY: `window` is a live handle from the enumeration.
        let visible = unsafe { IsWindowVisible(window) }.as_bool();
        if let Some(path) = visible
            .then(|| window_pid(window).and_then(exe_path))
            .flatten()
            && file_name(&path).eq_ignore_ascii_case(search.app_id)
        {
            search.found = Some(path);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut search = Search {
        app_id,
        found: None,
    };
    // SAFETY: `visit` matches WNDENUMPROC and `search` lives until
    // EnumWindows returns. Stopping early reports an error, which is ignored.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&raw mut search as isize)) };
    search.found
}

/// Nothing to request on Windows.
pub(crate) fn request_accessibility() {}
