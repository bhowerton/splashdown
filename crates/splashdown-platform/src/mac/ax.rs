//! Accessibility (AX) API: focused app and field, selected text, password
//! fields, and permission checks.

use std::collections::HashSet;
use std::sync::Mutex;

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_foundation::url::CFURL;
use objc2_app_kit::NSWorkspace;

use super::ffi::{self, AX_SUCCESS};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

use crate::{Access, AppInfo, Permissions, Selection};

/// Bound how long a hung app can block an AX call.
const AX_TIMEOUT_SECS: f32 = 0.25;

/// Chromium browsers only build their accessibility tree when an assistive
/// app asks for it via `AXEnhancedUserInterface`. Setting it on other apps
/// can disturb window animations, so it's limited to these.
const CHROMIUM_BUNDLE_IDS: &[&str] = &[
    "com.google.Chrome",
    "com.google.Chrome.canary",
    "com.brave.Browser",
    "com.microsoft.edgemac",
    "company.thebrowser.Browser",
    "com.vivaldi.Vivaldi",
    "com.operasoftware.Opera",
    "org.chromium.Chromium",
];

/// Whether `bundle_id` is a Chromium-based browser.
pub(crate) fn is_chromium(bundle_id: &str) -> bool {
    CHROMIUM_BUNDLE_IDS.contains(&bundle_id)
}

/// An owned AX element. Always wraps an `AXUIElementRef` (never another CF
/// type), which the AX calls below rely on.
pub(crate) struct AxElement(CFType);
// SAFETY: the only state is a retained, immutable AXUIElementRef (a pid plus
// an opaque token). CF retain/release are atomic, the element has no thread
// affinity, and the AX client calls made through `&self` are messages to the
// target process, not main-thread-only APIs. Apple doesn't formally document
// AX as thread-safe, but this is how assistive apps commonly use it. The one
// mutation, the messaging timeout, happens in the constructors before the
// element can be shared.
unsafe impl Send for AxElement {}
// SAFETY: see `Send` above; `&self` methods only read attributes or send
// messages, and there's no Rust-side interior mutability.
unsafe impl Sync for AxElement {}

impl AxElement {
    /// Takes ownership of a reference returned by an AX `Create` function.
    ///
    /// # Safety
    ///
    /// `r` must be null or an `AXUIElementRef` the caller owns (+1, Create
    /// rule); ownership passes to the returned element.
    unsafe fn from_create(r: CFTypeRef) -> Option<Self> {
        // SAFETY: non-null here, and the caller guarantees it's an owned
        // AXUIElementRef, so wrapping without a retain balances the +1.
        (!r.is_null()).then(|| Self(unsafe { CFType::wrap_under_create_rule(r) }))
    }

    fn system_wide() -> Option<Self> {
        // SAFETY: AXUIElementCreateSystemWide has no preconditions and returns
        // a new AXUIElementRef (Create rule) or null.
        let el = unsafe { Self::from_create(ffi::AXUIElementCreateSystemWide()) }?;
        // SAFETY: `el` is a valid AXUIElementRef that nothing else sees yet.
        unsafe { ffi::AXUIElementSetMessagingTimeout(el.raw(), AX_TIMEOUT_SECS) };
        Some(el)
    }

    fn application(pid: i32) -> Option<Self> {
        // SAFETY: AXUIElementCreateApplication accepts any pid and returns a
        // new AXUIElementRef (Create rule) or null.
        let el = unsafe { Self::from_create(ffi::AXUIElementCreateApplication(pid)) }?;
        // SAFETY: `el` is a valid AXUIElementRef that nothing else sees yet.
        unsafe { ffi::AXUIElementSetMessagingTimeout(el.raw(), AX_TIMEOUT_SECS) };
        Some(el)
    }

    fn raw(&self) -> CFTypeRef {
        self.0.as_CFTypeRef()
    }

    /// Copies an attribute; `Err` carries the AXError.
    fn attribute(&self, name: &str) -> Result<CFType, i32> {
        let name = CFString::new(name);
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: `self` is a live AXUIElementRef, `name` a live CFString,
        // and `value` a valid out-pointer for one CFTypeRef.
        let err = unsafe {
            ffi::AXUIElementCopyAttributeValue(self.raw(), name.as_concrete_TypeRef(), &mut value)
        };
        if err != AX_SUCCESS {
            return Err(err);
        }
        if value.is_null() {
            return Err(-1);
        }
        // SAFETY: on success the Copy function hands us a non-null +1
        // reference (Create rule), released when the CFType drops.
        Ok(unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn element_attribute(&self, name: &str) -> Option<AxElement> {
        let v = self.attribute(name).ok()?;
        // Keep the invariant that AxElement only ever holds an AX element,
        // whatever the app returned for this attribute.
        // SAFETY: AXUIElementGetTypeID has no preconditions.
        (v.type_of() == unsafe { ffi::AXUIElementGetTypeID() }).then_some(AxElement(v))
    }

    /// `AXBoundsForRange`: where a text range is drawn on screen.
    fn bounds_for_range(&self, range: ffi::CfRange) -> Option<crate::CaretRect> {
        // SAFETY: `range` is a live CFRange for the call; AXValueCreate copies
        // it into a new AXValue (Create rule), owned below.
        let param =
            unsafe { ffi::AXValueCreate(ffi::AX_VALUE_CF_RANGE, (&raw const range).cast()) };
        if param.is_null() {
            return None;
        }
        // SAFETY: non-null and owned by us (Create rule); released on drop.
        let param = unsafe { CFType::wrap_under_create_rule(param) };
        let name = CFString::new("AXBoundsForRange");
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: `self` is a live AXUIElementRef, `name` and `param` live CF
        // objects, and `value` a valid out-pointer for one CFTypeRef.
        let err = unsafe {
            ffi::AXUIElementCopyParameterizedAttributeValue(
                self.raw(),
                name.as_concrete_TypeRef(),
                param.as_CFTypeRef(),
                &mut value,
            )
        };
        if err != AX_SUCCESS || value.is_null() {
            return None;
        }
        // SAFETY: on success the Copy function returns a +1 reference.
        let value = unsafe { CFType::wrap_under_create_rule(value) };
        let mut rect = ffi::CgRect::default();
        // SAFETY: AXValueGetValue checks the value holds a CGRect before
        // writing into `rect`, which has CGRect's layout.
        let ok = unsafe {
            ffi::AXValueGetValue(
                value.as_CFTypeRef(),
                ffi::AX_VALUE_CG_RECT,
                (&raw mut rect).cast(),
            )
        };
        (ok && rect.height > 0.0 && rect.x.is_finite() && rect.y.is_finite()).then_some(
            crate::CaretRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            },
        )
    }

    fn string_attribute(&self, name: &str) -> Result<String, i32> {
        let v = self.attribute(name)?;
        v.downcast::<CFString>().map(|s| s.to_string()).ok_or(-1)
    }

    fn set_bool(&self, name: &str, value: bool) {
        let name = CFString::new(name);
        let v = if value {
            CFBoolean::true_value()
        } else {
            CFBoolean::false_value()
        };
        // SAFETY: all three arguments are live CF objects for the call; the
        // function retains the value if it keeps it.
        unsafe {
            ffi::AXUIElementSetAttributeValue(
                self.raw(),
                name.as_concrete_TypeRef(),
                v.as_CFTypeRef(),
            )
        };
    }

    /// The element's children, in order.
    fn children(&self) -> Vec<AxElement> {
        let Some(list) = self
            .attribute("AXChildren")
            .ok()
            .and_then(|v| v.downcast::<CFArray>())
        else {
            return Vec::new();
        };
        // SAFETY: AXUIElementGetTypeID has no preconditions.
        let element_type = unsafe { ffi::AXUIElementGetTypeID() };
        list.iter()
            .filter_map(|item| {
                // SAFETY: an array item is a live CF object owned by `list`;
                // the get rule retains it for the new CFType.
                let v = unsafe { CFType::wrap_under_get_rule(*item as CFTypeRef) };
                (v.type_of() == element_type).then_some(AxElement(v))
            })
            .collect()
    }

    /// Whether this is the same element as `other`.
    fn same_as(&self, other: &AxElement) -> bool {
        self.0 == other.0
    }

    fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        // SAFETY: `self` is a live AXUIElementRef and `pid` a valid
        // out-pointer.
        (unsafe { ffi::AXUIElementGetPid(self.raw(), &mut pid) } == AX_SUCCESS).then_some(pid)
    }
}

impl PartialEq for AxElement {
    fn eq(&self, other: &Self) -> bool {
        // CFEqual: AX elements compare equal when they're the same UI object.
        self.0 == other.0
    }
}

/// The control at a point on the screen (in points), for computer use's
/// checks before a click: its role ("AXButton"), whether it's a password
/// field, the label people see on it or the nearest one around it, and
/// the app it belongs to.
pub(crate) fn element_at(x: f64, y: f64) -> Option<crate::ElementInfo> {
    let system = AxElement::system_wide()?;
    let mut raw: CFTypeRef = std::ptr::null();
    // SAFETY: `system` is a live AXUIElementRef and `raw` a valid
    // out-pointer; on success it holds a +1 reference (Copy rule).
    let err = unsafe {
        ffi::AXUIElementCopyElementAtPosition(system.raw(), x as f32, y as f32, &mut raw)
    };
    if err != AX_SUCCESS {
        return None;
    }
    // SAFETY: on success `raw` is an AXUIElementRef we own.
    let element = unsafe { AxElement::from_create(raw) }?;
    let role = element.string_attribute("AXRole").unwrap_or_default();
    let secure = element.string_attribute("AXSubrole").ok().as_deref() == Some("AXSecureTextField");
    let pid = element.pid().unwrap_or(0);
    // The text on a button may be a child of it: look up a few levels.
    let mut label = String::new();
    let mut current = Some(element);
    for _ in 0..3 {
        let Some(el) = current else { break };
        for name in ["AXTitle", "AXDescription", "AXValue"] {
            if let Ok(text) = el.string_attribute(name)
                && !text.trim().is_empty()
                && text.chars().count() <= 80
            {
                label = text.trim().to_string();
                break;
            }
        }
        if !label.is_empty() {
            break;
        }
        current = el.element_attribute("AXParent");
    }
    let app = objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|a| a.localizedName())
        .map(|n| n.to_string())
        .unwrap_or_default();
    Some(crate::ElementInfo {
        role,
        secure,
        label,
        app,
    })
}

pub(crate) struct FocusElement {
    element: Option<AxElement>,
    /// The window it was in, for when the app swaps the element itself.
    window: Option<AxElement>,
}

/// Apps we've already asked to expose their accessibility tree.
static PREPARED_PIDS: Mutex<Option<HashSet<i32>>> = Mutex::new(None);

pub(crate) fn capture_focus() -> Option<(AppInfo, FocusElement)> {
    let workspace = NSWorkspace::sharedWorkspace();
    let front = workspace.frontmostApplication()?;
    let pid = front.processIdentifier();
    let id = front
        .bundleIdentifier()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let name = front
        .localizedName()
        .map(|s| s.to_string())
        .unwrap_or_else(|| id.clone());

    let app_el = AxElement::application(pid);
    if let Some(app_el) = &app_el {
        prepare_app(pid, &id, app_el);
    }
    let window = app_el
        .as_ref()
        .and_then(|a| a.element_attribute("AXFocusedWindow"));
    let window_title = window
        .as_ref()
        .and_then(|w| w.string_attribute("AXTitle").ok())
        .filter(|t| !t.is_empty());
    // Terminal reports its working folder, editors their open file.
    let document = window.as_ref().and_then(|w| {
        let value = w.attribute("AXDocument").ok()?;
        let url = match value.downcast::<CFURL>() {
            Some(url) => url.get_string().to_string(),
            None => value.downcast::<CFString>()?.to_string(),
        };
        crate::file_url_path(&url)
    });

    let element = AxElement::system_wide()
        .and_then(|sw| sw.element_attribute("AXFocusedUIElement"))
        // Only trust it if it belongs to the frontmost app.
        .filter(|el| el.pid() == Some(pid));
    let url = element.as_ref().and_then(page_url);

    Some((
        AppInfo {
            id,
            name,
            pid,
            window_title,
            url,
            document,
        },
        FocusElement { element, window },
    ))
}

/// The URL of the web page containing `el`: walk up to the enclosing
/// `AXWebArea` and read its `AXURL`. Works in Safari and Chromium browsers.
fn page_url(el: &AxElement) -> Option<String> {
    const MAX_DEPTH: usize = 40;
    let mut node = AxElement(el.0.clone());
    for _ in 0..MAX_DEPTH {
        if node.string_attribute("AXRole").ok().as_deref() == Some("AXWebArea") {
            let value = node.attribute("AXURL").ok()?;
            if let Some(url) = value.downcast::<CFURL>() {
                return Some(url.get_string().to_string());
            }
            return value.downcast::<CFString>().map(|s| s.to_string());
        }
        node = node.element_attribute("AXParent")?;
    }
    None
}

/// Turns on the accessibility tree in Electron and Chromium apps, once per
/// process. The first dictation in such an app may still miss the
/// selection while the tree builds; the copy fallback covers that.
fn prepare_app(pid: i32, bundle_id: &str, app: &AxElement) {
    let mut guard = PREPARED_PIDS.lock().unwrap_or_else(|e| e.into_inner());
    let prepared = guard.get_or_insert_with(HashSet::new);
    if !prepared.insert(pid) {
        return;
    }
    // Electron's switch; harmless elsewhere (the call just fails).
    app.set_bool("AXManualAccessibility", true);
    if CHROMIUM_BUNDLE_IDS.contains(&bundle_id) {
        app.set_bool("AXEnhancedUserInterface", true);
    }
}

pub(crate) fn is_current(app: &AppInfo, focus: &FocusElement) -> bool {
    let Some(front) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        return false;
    };
    if front.processIdentifier() != app.pid {
        return false;
    }
    let Some(original) = &focus.element else {
        return true;
    };
    // Same app; compare the focused field too when we know it.
    let Some(now) =
        AxElement::system_wide().and_then(|sw| sw.element_attribute("AXFocusedUIElement"))
    else {
        return true;
    };
    if now == *original {
        return true;
    }
    // Chrome and other web views rebuild their accessibility elements (on
    // first use, and as pages update), so the same field can come back as a
    // new element. Same window, and still a text field: it's current.
    let same_window = focus.window.as_ref().is_some_and(|w| {
        AxElement::application(app.pid)
            .and_then(|a| a.element_attribute("AXFocusedWindow"))
            .is_some_and(|now| now == *w)
    });
    same_window && now.attribute("AXSelectedTextRange").is_ok()
}

pub(crate) fn selection(focus: &FocusElement) -> Selection {
    let Some(el) = &focus.element else {
        return Selection::Unknown;
    };
    match el.string_attribute("AXSelectedText") {
        Ok(s) if s.is_empty() => Selection::Empty,
        Ok(s) => Selection::Text(s),
        Err(_) => Selection::Unknown,
    }
}

/// Longer field contents aren't read (large documents).
const MAX_FIELD_CHARS: usize = 20_000;

pub(crate) fn field_text(focus: &FocusElement) -> Option<String> {
    if is_secure_field(focus) {
        return None;
    }
    let text = focus.element.as_ref()?.string_attribute("AXValue").ok()?;
    (text.chars().count() <= MAX_FIELD_CHARS).then_some(text)
}

/// Where the insertion point is in the field's text, in UTF-16 units (the
/// start of the selection), if the app reports it.
pub(crate) fn cursor_offset(focus: &FocusElement) -> Option<usize> {
    let value = focus
        .element
        .as_ref()?
        .attribute("AXSelectedTextRange")
        .ok()?;
    let mut range = ffi::CfRange::default();
    // SAFETY: `value` is a live CF object returned for this attribute; the
    // call checks it's an AXValue holding a CFRange before writing `range`,
    // which has CFRange's layout, and returns false otherwise.
    let ok = unsafe {
        ffi::AXValueGetValue(
            value.as_CFTypeRef(),
            ffi::AX_VALUE_CF_RANGE,
            (&raw mut range).cast(),
        )
    };
    (ok && range.location >= 0).then_some(range.location as usize)
}

/// Selects `length` UTF-16 units of the field's text from `location`.
/// Returns whether the app did it (re-reading the selection for up to
/// 0.3 s, since some apps apply it late and some ignore it).
pub(crate) fn select_range(focus: &FocusElement, location: usize, length: usize) -> bool {
    let Some(el) = focus.element.as_ref() else {
        return false;
    };
    let range = ffi::CfRange {
        location: location as isize,
        length: length as isize,
    };
    // SAFETY: `range` is a live CFRange for the call; AXValueCreate copies
    // it into a new AXValue (Create rule), owned below.
    let value = unsafe { ffi::AXValueCreate(ffi::AX_VALUE_CF_RANGE, (&raw const range).cast()) };
    if value.is_null() {
        return false;
    }
    // SAFETY: non-null and owned by us (Create rule); released on drop.
    let value = unsafe { CFType::wrap_under_create_rule(value) };
    let name = CFString::new("AXSelectedTextRange");
    // SAFETY: all three arguments are live CF objects for the call; the
    // function retains the value if it keeps it.
    let err = unsafe {
        ffi::AXUIElementSetAttributeValue(
            el.raw(),
            name.as_concrete_TypeRef(),
            value.as_CFTypeRef(),
        )
    };
    if err != AX_SUCCESS {
        return false;
    }
    // Some apps (TextEdit) apply the new selection a moment later, and
    // some report success and ignore it: wait briefly for it to show.
    for _ in 0..SELECT_CHECKS {
        if cursor_offset(focus) == Some(location) {
            return true;
        }
        std::thread::sleep(SELECT_CHECK_EVERY);
    }
    tracing::debug!(
        "selection not applied: wanted {location}, have {:?}",
        cursor_offset(focus)
    );
    false
}

/// How long `select_range` waits for an app to apply a selection.
const SELECT_CHECKS: u32 = 20;
const SELECT_CHECK_EVERY: std::time::Duration = std::time::Duration::from_millis(15);

/// The insertion point's on-screen rectangle (top-left origin, points), if
/// the app reports it.
pub(crate) fn caret_rect(focus: &FocusElement) -> Option<crate::CaretRect> {
    let el = focus.element.as_ref()?;
    let location = cursor_offset(focus)? as isize;
    // A zero-length range gives the caret; some apps only answer for a
    // character, so try the one before it too.
    for range in [
        ffi::CfRange {
            location,
            length: 0,
        },
        ffi::CfRange {
            location: (location - 1).max(0),
            length: 1,
        },
    ] {
        if let Some(rect) = el.bounds_for_range(range) {
            return Some(rect);
        }
    }
    None
}

pub(crate) fn is_secure_field(focus: &FocusElement) -> bool {
    focus
        .element
        .as_ref()
        .and_then(|el| el.string_attribute("AXSubrole").ok())
        .is_some_and(|r| r == "AXSecureTextField")
}

pub(crate) fn secure_input_active() -> bool {
    // SAFETY: takes no arguments and has no preconditions.
    unsafe { ffi::IsSecureEventInputEnabled() != 0 }
}

pub(crate) fn permissions() -> Permissions {
    Permissions {
        // SAFETY: takes no arguments and has no preconditions.
        accessibility: unsafe { ffi::AXIsProcessTrusted() },
        microphone: microphone_access(),
    }
}

fn microphone_access() -> Access {
    // SAFETY: reads an immutable constant exported by AVFoundation, which is
    // linked, so it's initialized before any Rust code runs.
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        return Access::NotAsked;
    };
    // SAFETY: `audio` is a valid media type; this class method only reads
    // the privacy (TCC) status and may be called from any thread.
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
        AVAuthorizationStatus::Authorized => Access::Granted,
        AVAuthorizationStatus::NotDetermined => Access::NotAsked,
        _ => Access::Denied,
    }
}

pub(crate) fn request_accessibility() {
    // SAFETY: takes no arguments and has no preconditions.
    if unsafe { ffi::AXIsProcessTrusted() } {
        return;
    }
    // SAFETY: the framework constant is a non-null CFString that lives for the
    // whole process; the Get rule retains it for our wrapper.
    let key = unsafe { CFString::wrap_under_get_rule(ffi::kAXTrustedCheckOptionPrompt) };
    let options =
        CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())]);
    // SAFETY: `options` is a live CFDictionary for the duration of the call.
    unsafe { ffi::AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) };
}

// ---------------------------------------------------------------- the window's text

/// Roles whose own text is worth reading: what people wrote and headings.
const TEXT_ROLES: &[&str] = &["AXStaticText", "AXTextArea", "AXTextField", "AXHeading"];
/// Limits on reading the window: levels up from the field, elements read,
/// and time (web pages answer slowly).
const WINDOW_LEVELS: usize = 40;
const WINDOW_NODES: usize = 5000;
const WINDOW_MILLIS: u64 = 600;
/// Roles skipped with everything in them: controls and chrome.
const SKIP_ROLES: &[&str] = &[
    "AXButton",
    "AXMenuBar",
    "AXMenu",
    "AXMenuButton",
    "AXPopUpButton",
    "AXToolbar",
    "AXImage",
    "AXSecureTextField",
    "AXScrollBar",
    "AXCheckBox",
    "AXRadioButton",
    "AXSlider",
];

/// The text around the focused field in its window: from the nearest
/// container holding at least `min_chars` of readable text besides the
/// field itself, the last `max_chars` of it (what's nearest the field, in a
/// thread). Each level up reads only what's beside the level below, so a
/// field deep in a web page (an email reply box) still reaches the thread.
/// Bounded in nodes and time, so a huge page can't stall it.
pub(crate) fn surrounding_text(
    focus: &FocusElement,
    min_chars: usize,
    max_chars: usize,
) -> Option<String> {
    let field = focus.element.as_ref()?;
    if is_secure_field(focus) {
        return None;
    }
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_millis(WINDOW_MILLIS);
    let mut budget = WINDOW_NODES;
    let mut pieces: Vec<String> = Vec::new();
    // The element read last, whose text is already in `pieces`.
    let mut below: Option<AxElement> = None;
    let mut node = field.element_attribute("AXParent");
    let mut levels = 0;
    while let Some(current) = node.take() {
        levels += 1;
        let role = current.string_attribute("AXRole").unwrap_or_default();
        let (mut before, mut after) = (Vec::new(), Vec::new());
        let mut passed = false;
        for child in current.children() {
            if below
                .as_ref()
                .map_or_else(|| child.same_as(field), |b| child.same_as(b))
            {
                passed = true;
                continue;
            }
            let side = if passed { &mut after } else { &mut before };
            collect(&child, field, side, &mut budget, deadline, 0);
        }
        before.append(&mut pieces);
        before.append(&mut after);
        pieces = before;
        let chars: usize = pieces.iter().map(|p| p.chars().count()).sum();
        if chars >= min_chars
            || role == "AXWindow"
            || levels >= WINDOW_LEVELS
            || budget == 0
            || std::time::Instant::now() >= deadline
        {
            break;
        }
        node = current.element_attribute("AXParent");
        below = Some(current);
    }
    let text = pieces.join("\n");
    tracing::debug!(
        "window text: {} characters, {levels} levels up, {} nodes, {} ms",
        text.chars().count(),
        WINDOW_NODES - budget,
        started.elapsed().as_millis()
    );
    Some(crate::keep_end(&text, max_chars)).filter(|t| !t.trim().is_empty())
}

/// Appends the readable text under `el` (not the field's own) to `pieces`.
fn collect(
    el: &AxElement,
    field: &AxElement,
    pieces: &mut Vec<String>,
    budget: &mut usize,
    deadline: std::time::Instant,
    depth: usize,
) {
    if *budget == 0 || depth > 40 || std::time::Instant::now() >= deadline || el.same_as(field) {
        return;
    }
    *budget -= 1;
    let role = el.string_attribute("AXRole").unwrap_or_default();
    if SKIP_ROLES.contains(&role.as_str()) {
        return;
    }
    if TEXT_ROLES.contains(&role.as_str()) {
        let text = el
            .string_attribute("AXValue")
            .ok()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| el.string_attribute("AXTitle").ok());
        if let Some(text) = text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
            pieces.push(text);
        }
        // A text area's children repeat its value.
        if role != "AXHeading" {
            return;
        }
    }
    for child in el.children() {
        collect(&child, field, pieces, budget, deadline, depth + 1);
    }
}
