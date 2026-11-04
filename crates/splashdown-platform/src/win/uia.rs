//! UI Automation: the focused field, its text and selection, password
//! fields, page URLs and the text around the field; plus the permission
//! answers for Windows. The Windows twin of `mac/ax.rs` (apps are in
//! `apps.rs`, the microphone in `mic.rs`).

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayUnaccessData};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextPattern2, IUIAutomationTextRange, IUIAutomationTreeWalker,
    IUIAutomationValuePattern, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
    TextUnit_Character, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
    UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_HyperlinkControlTypeId,
    UIA_ImageControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuControlTypeId,
    UIA_RadioButtonControlTypeId, UIA_ScrollBarControlTypeId, UIA_SliderControlTypeId,
    UIA_TextControlTypeId, UIA_TextPattern2Id, UIA_TextPatternId, UIA_ToolBarControlTypeId,
    UIA_ValuePatternId, UIA_WindowControlTypeId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowTextW, GetWindowThreadProcessId,
};
use windows::core::{BOOL, Interface};

use super::apps::*;
use super::from_wide;
use super::mic::*;
use crate::{AppInfo, Permissions, Selection};

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

pub(super) fn window_pid(window: HWND) -> Option<u32> {
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

/// Where the text cursor is on screen, in physical pixels: the system
/// caret that classic Win32 fields show, or else the insertion point UI
/// Automation reports (browsers, Office, modern apps). `None` when neither
/// says.
pub(crate) fn caret_rect(focus: &FocusElement) -> Option<crate::CaretRect> {
    system_caret(focus.window).or_else(|| automation_caret(focus))
}

/// The caret of the foreground window's thread, from Win32.
fn system_caret(window: isize) -> Option<crate::CaretRect> {
    let window = HWND(window as *mut _);
    if window.is_invalid() {
        return None;
    }
    // SAFETY: a stale window handle just fails; `info` is a sized
    // out-pointer and `at` a valid point.
    unsafe {
        let thread = GetWindowThreadProcessId(window, None);
        if thread == 0 {
            return None;
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(thread, &mut info).ok()?;
        let rc = info.rcCaret;
        if info.hwndCaret.is_invalid() || rc.bottom <= rc.top {
            return None;
        }
        let mut at = POINT {
            x: rc.left,
            y: rc.top,
        };
        if !ClientToScreen(info.hwndCaret, &mut at).as_bool() {
            return None;
        }
        Some(crate::CaretRect {
            x: f64::from(at.x),
            y: f64::from(at.y),
            width: f64::from(rc.right - rc.left),
            height: f64::from(rc.bottom - rc.top),
        })
    }
}

/// The insertion point from UI Automation: the caret range where the field
/// offers one, else the start of its selection.
fn automation_caret(focus: &FocusElement) -> Option<crate::CaretRect> {
    let el = usable(focus)?;
    // SAFETY: pattern and range calls on a live element and the ranges it
    // returns, on an MTA thread; index 0 is checked against the array's
    // length first.
    unsafe {
        let caret = el
            .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
            .ok()
            .and_then(|p| {
                let mut active = BOOL::default();
                p.GetCaretRange(&mut active).ok()
            });
        let range = match caret {
            Some(range) => range,
            None => {
                let selection = el
                    .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                    .and_then(|p| p.GetSelection())
                    .ok()?;
                if selection.Length().ok()? < 1 {
                    return None;
                }
                selection.GetElement(0).ok()?
            }
        };
        // A selection's first line, at its start.
        if let Some(r) = first_rect(&range) {
            return Some(crate::CaretRect { width: 1.0, ..r });
        }
        // An empty range has no rectangle: the character after it (the
        // caret is at its left edge), or else the one before (its right).
        let after = range.Clone().ok()?;
        if after.ExpandToEnclosingUnit(TextUnit_Character).is_ok()
            && let Some(r) = first_rect(&after)
        {
            return Some(crate::CaretRect { width: 1.0, ..r });
        }
        let before = range.Clone().ok()?;
        before
            .MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1)
            .ok()?;
        let r = first_rect(&before)?;
        Some(crate::CaretRect {
            x: r.x + r.width,
            width: 1.0,
            ..r
        })
    }
}

/// The first of a range's rectangles (one per line), if it has any.
///
/// # Safety
///
/// `range` must be a live range used on an MTA thread.
unsafe fn first_rect(range: &IUIAutomationTextRange) -> Option<crate::CaretRect> {
    // SAFETY: the caller's guarantee for the range. The array is ours to
    // destroy; its data is read only between access and unaccess, and only
    // as many doubles as its single dimension holds.
    unsafe {
        let array = range.GetBoundingRectangles().ok()?;
        if array.is_null() {
            return None;
        }
        let mut data = std::ptr::null_mut();
        let mut rect = None;
        if (*array).cDims == 1 && SafeArrayAccessData(array, &mut data).is_ok() {
            let count = (*array).rgsabound[0].cElements as usize;
            if count >= 4 && !data.is_null() {
                let v = std::slice::from_raw_parts(data as *const f64, 4);
                if v[3] > 0.0 {
                    rect = Some(crate::CaretRect {
                        x: v[0],
                        y: v[1],
                        width: v[2],
                        height: v[3],
                    });
                }
            }
            let _ = SafeArrayUnaccessData(array);
        }
        let _ = SafeArrayDestroy(array);
        rect
    }
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

/// Nothing to request on Windows.
pub(crate) fn request_accessibility() {}

// ---------------------------------------------------------------- the window's text

/// The text around the focused field in its window: from the nearest
/// container holding at least `min_chars` of readable text besides the
/// field itself, the last `max_chars` of it. Each level up reads only
/// what's beside the level below, so a field deep in a web page still
/// reaches the thread. Bounded in nodes and time.
pub(crate) fn surrounding_text(
    focus: &FocusElement,
    min_chars: usize,
    max_chars: usize,
) -> Option<String> {
    if is_secure_field(focus) {
        return None;
    }
    let field = usable(focus)?.clone();
    with_uia(|uia| {
        // SAFETY: plain calls on this thread's live client.
        let walker = unsafe { uia.ControlViewWalker() }.ok()?;
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_millis(WINDOW_MILLIS);
        let mut budget = WINDOW_NODES;
        let mut pieces: Vec<String> = Vec::new();
        // The element read last, whose text is already in `pieces`.
        let mut below = field.clone();
        // SAFETY: `field` is a live element.
        let mut node = unsafe { walker.GetParentElement(&field) }.ok();
        let mut levels = 0;
        while let Some(current) = node.take() {
            levels += 1;
            // SAFETY: a property read on a live element.
            let kind = unsafe { current.CurrentControlType() }.unwrap_or_default();
            let (mut before, mut after) = (Vec::new(), Vec::new());
            let mut passed = false;
            // SAFETY: `current` is a live element.
            let mut child = unsafe { walker.GetFirstChildElement(&current) }.ok();
            while let Some(c) = child {
                // SAFETY: comparing two live elements.
                if unsafe { uia.CompareElements(&c, &below) }.is_ok_and(|same| same.as_bool()) {
                    passed = true;
                } else {
                    let side = if passed { &mut after } else { &mut before };
                    collect(uia, &walker, &c, &field, side, &mut budget, deadline, 0);
                }
                // SAFETY: `c` is a live element.
                child = unsafe { walker.GetNextSiblingElement(&c) }.ok();
            }
            before.append(&mut pieces);
            before.append(&mut after);
            pieces = before;
            let chars: usize = pieces.iter().map(|p| p.chars().count()).sum();
            if chars >= min_chars
                || kind == UIA_WindowControlTypeId
                || levels >= WINDOW_LEVELS
                || budget == 0
                || std::time::Instant::now() >= deadline
            {
                break;
            }
            // SAFETY: `current` is a live element.
            node = unsafe { walker.GetParentElement(&current) }.ok();
            below = current;
        }
        let text = pieces.join("\n");
        tracing::debug!(
            "window text: {} characters, {levels} levels up, {} nodes, {} ms",
            text.chars().count(),
            WINDOW_NODES - budget,
            started.elapsed().as_millis()
        );
        Some(crate::keep_end(&text, max_chars)).filter(|t| !t.trim().is_empty())
    })
}

/// Limits on reading the window: levels up from the field, elements read,
/// and time (web pages answer slowly).
const WINDOW_LEVELS: usize = 40;
const WINDOW_NODES: usize = 5000;
const WINDOW_MILLIS: u64 = 600;

/// Appends the readable text under `el` (not the field's own) to `pieces`.
#[allow(clippy::too_many_arguments)]
fn collect(
    uia: &IUIAutomation,
    walker: &IUIAutomationTreeWalker,
    el: &IUIAutomationElement,
    field: &IUIAutomationElement,
    pieces: &mut Vec<String>,
    budget: &mut usize,
    deadline: std::time::Instant,
    depth: usize,
) {
    if *budget == 0 || depth > 40 || std::time::Instant::now() >= deadline {
        return;
    }
    // SAFETY: comparing two live elements.
    if unsafe { uia.CompareElements(el, field) }.is_ok_and(|same| same.as_bool()) {
        return;
    }
    *budget -= 1;
    // SAFETY: property reads on a live element.
    let kind = unsafe { el.CurrentControlType() }.unwrap_or_default();
    let skip = [
        UIA_ButtonControlTypeId,
        UIA_MenuBarControlTypeId,
        UIA_MenuControlTypeId,
        UIA_ToolBarControlTypeId,
        UIA_ImageControlTypeId,
        UIA_ScrollBarControlTypeId,
        UIA_CheckBoxControlTypeId,
        UIA_RadioButtonControlTypeId,
        UIA_SliderControlTypeId,
    ];
    if skip.contains(&kind) {
        return;
    }
    // SAFETY: property and pattern reads on a live element.
    let password = unsafe { el.CurrentIsPassword() }.is_ok_and(|p| p.as_bool());
    if password {
        return;
    }
    if kind == UIA_TextControlTypeId || kind == UIA_HyperlinkControlTypeId {
        // SAFETY: a property read on a live element.
        if let Ok(name) = unsafe { el.CurrentName() } {
            let name = name.to_string();
            if !name.trim().is_empty() {
                pieces.push(name.trim().to_string());
            }
        }
        return;
    }
    if kind == UIA_EditControlTypeId {
        // SAFETY: a pattern read on a live element.
        let value = unsafe {
            el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|p| p.CurrentValue())
        };
        if let Ok(value) = value {
            let value = value.to_string();
            if !value.trim().is_empty() {
                pieces.push(value.trim().to_string());
            }
        }
        return;
    }
    // SAFETY: `el` is a live element.
    let mut child = unsafe { walker.GetFirstChildElement(el) }.ok();
    while let Some(c) = child {
        collect(uia, walker, &c, field, pieces, budget, deadline, depth + 1);
        // SAFETY: `c` is a live element.
        child = unsafe { walker.GetNextSiblingElement(&c) }.ok();
    }
}
