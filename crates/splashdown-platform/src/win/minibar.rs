//! The mini bar (spec §2.4): a small round, topmost tool window that stays
//! on screen, for starting and finishing dictation with the mouse.
//!
//! Unlike the overlay it takes clicks, but it never activates: it's created
//! with `WS_EX_NOACTIVATE` and answers `WM_MOUSEACTIVATE` with
//! `MA_NOACTIVATE`, so the foreground window and its focused field stay as
//! they were. Dragging uses mouse capture rather than the system's move
//! loop, which would activate the window. Painted with GDI; the round shape
//! is a window region.
//!
//! Not done on Windows yet: the tooltip and screen-reader name (they need a
//! tooltip control and a UI Automation provider), and the right-click menu
//! (`TrackPopupMenu` needs the window in the foreground to close properly).

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateEllipticRgn, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, Ellipse,
    EndPaint, FW_NORMAL, FillRect, GetMonitorInfoW, GetStockObject, HDC, HFONT, HGDIOBJ,
    InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY,
    MONITORINFO, MonitorFromPoint, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, RoundRect, SRCCOPY,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowRect, HWND_TOPMOST,
    KillTimer, LWA_ALPHA, MA_NOACTIVATE, RegisterClassExW, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WM_CAPTURECHANGED,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT,
    WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::w;

use crate::{MiniBarEvent, MiniBarHandler, MiniBarPosition, MiniBarState, PlatformError, Result};

// Sizes in device-independent pixels (scaled by the monitor's DPI).
/// The button's diameter.
const SIZE: f64 = 40.0;
const GLYPH_PX: f64 = 17.0;
const STOP: f64 = 12.0;
/// The mouse must move this far before a press counts as a drag.
const DRAG_SLOP: f64 = 3.0;
/// Default place: this far in from the right edge of the work area…
const EDGE_MARGIN: f64 = 12.0;
/// …and this far up it (a fraction of its height).
const DEFAULT_HEIGHT: f64 = 0.3;

/// Timer that turns the "working" spinner.
const SPIN_TIMER: usize = 1;
const SPIN_MS: u32 = 90;
const SPIN_DOTS: u8 = 8;

/// Segoe MDL2 Assets' microphone (Windows 10 and later).
const MIC_GLYPH: &str = "\u{E720}";

#[derive(Clone, Copy)]
struct Rgb(u8, u8, u8);

impl Rgb {
    fn colorref(self) -> COLORREF {
        COLORREF(u32::from(self.0) | (u32::from(self.1) << 8) | (u32::from(self.2) << 16))
    }

    /// `self` moved toward `other` by `t` (0–1).
    fn mix(self, other: Rgb, t: f64) -> Rgb {
        let m = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

const RED: Rgb = Rgb(240, 69, 69);
const BACKGROUND: Rgb = Rgb(23, 23, 28);
const TEXT: Rgb = Rgb(245, 245, 245);
const GRAY: Rgb = Rgb(150, 150, 156);

/// What the bar shows, its size, and the drag in progress.
struct Model {
    state: MiniBarState,
    spin: u8,
    scale: f64,
    /// Diameter in physical pixels.
    size: i32,
    font: Option<HFONT>,
    /// Cursor and window position when the button went down.
    press: Option<(POINT, POINT)>,
    dragged: bool,
}

impl Model {
    fn px(&self, v: f64) -> i32 {
        (v * self.scale).round() as i32
    }

    fn set_scale(&mut self, scale: f64) {
        if self.font.is_some() && (scale - self.scale).abs() < f64::EPSILON {
            return;
        }
        self.scale = scale;
        self.size = self.px(SIZE);
        if let Some(old) = self.font.take() {
            // SAFETY: the font was created by us and isn't selected into any
            // DC (each paint restores the DC's previous font).
            let _ = unsafe { DeleteObject(old.into()) };
        }
        // SAFETY: plain values and a NUL-terminated face name; returns a new
        // font (or null, which GDI treats as "use the default font").
        let font = unsafe {
            CreateFontW(
                -self.px(GLYPH_PX),
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe MDL2 Assets"),
            )
        };
        self.font = (!font.is_invalid()).then_some(font);
    }

    fn draw(&self, dc: HDC) {
        let s = self.size;
        if s <= 0 {
            return;
        }
        // SAFETY: every GDI object created here is selected out and deleted
        // before returning; `dc` is the paint DC, valid for this WM_PAINT.
        unsafe {
            // Draw off-screen, then copy, so updates don't flicker.
            let mem = CreateCompatibleDC(Some(dc));
            let bitmap = CreateCompatibleBitmap(dc, s, s);
            let old_bitmap = SelectObject(mem, bitmap.into());
            let old_pen = SelectObject(mem, GetStockObject(NULL_PEN));

            let fill = match self.state {
                MiniBarState::Recording => RED,
                _ => BACKGROUND,
            };
            let brush = CreateSolidBrush(fill.colorref());
            let rect = RECT {
                left: 0,
                top: 0,
                right: s,
                bottom: s,
            };
            FillRect(mem, &rect, brush);
            let _ = DeleteObject(brush.into());

            let c = f64::from(s) / 2.0;
            match self.state {
                MiniBarState::Idle | MiniBarState::NotReady => {
                    let color = if self.state == MiniBarState::Idle {
                        TEXT
                    } else {
                        GRAY
                    };
                    let mut glyph: Vec<u16> = MIC_GLYPH.encode_utf16().collect();
                    let mut bounds = rect;
                    let old_font = self.font.map(|f| SelectObject(mem, f.into()));
                    SetBkMode(mem, TRANSPARENT);
                    SetTextColor(mem, color.colorref());
                    DrawTextW(
                        mem,
                        &mut glyph,
                        &mut bounds,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                    );
                    if let Some(old) = old_font {
                        SelectObject(mem, old);
                    }
                }
                MiniBarState::Recording => {
                    // A stop square: click to finish.
                    let half = STOP * self.scale / 2.0;
                    let brush = CreateSolidBrush(TEXT.colorref());
                    let old_brush = SelectObject(mem, brush.into());
                    let (l, t) = ((c - half).round() as i32, (c - half).round() as i32);
                    let (r, b) = ((c + half).round() as i32, (c + half).round() as i32);
                    let round = self.px(3.0);
                    let _ = RoundRect(mem, l, t, r + 1, b + 1, round, round);
                    SelectObject(mem, old_brush);
                    let _ = DeleteObject(brush.into());
                }
                MiniBarState::Working => {
                    // A ring of dots, brightest at the current step.
                    let ring = 6.5 * self.scale;
                    let r = 1.6 * self.scale;
                    for i in 0..SPIN_DOTS {
                        let age = (self.spin + SPIN_DOTS - i) % SPIN_DOTS;
                        let t = 1.0 - f64::from(age) / f64::from(SPIN_DOTS);
                        let angle = f64::from(i) * std::f64::consts::TAU / f64::from(SPIN_DOTS);
                        let (x, y) = (c + ring * angle.sin(), c - ring * angle.cos());
                        fill_ellipse(mem, BACKGROUND.mix(TEXT, 0.15 + 0.85 * t), x, y, r);
                    }
                }
            }

            let _ = BitBlt(dc, 0, 0, s, s, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old_pen);
            SelectObject(mem, old_bitmap);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(mem);
        }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        if let Some(font) = self.font.take() {
            // SAFETY: our font, not selected into any DC.
            let _ = unsafe { DeleteObject(font.into()) };
        }
    }
}

/// Fills a circle with the null pen selected (no outline).
///
/// # Safety
///
/// `dc` must be a valid device context.
unsafe fn fill_ellipse(dc: HDC, color: Rgb, cx: f64, cy: f64, r: f64) {
    // SAFETY: the caller guarantees `dc`; the brush is selected out and
    // deleted before returning.
    unsafe {
        let brush = CreateSolidBrush(color.colorref());
        let old = SelectObject(dc, brush.into());
        let _ = Ellipse(
            dc,
            (cx - r).round() as i32,
            (cy - r).round() as i32,
            (cx + r).round() as i32 + 1,
            (cy + r).round() as i32 + 1,
        );
        SelectObject(dc, old);
        let _ = DeleteObject(brush.into());
    }
}

/// The handler, shared so it can be called with no borrow held.
type SharedHandler = Rc<dyn Fn(MiniBarEvent)>;

thread_local! {
    /// The bar's state, on the UI thread that owns the window. The window
    /// procedure reads it, so it can't live in `MiniBar` itself.
    static MODEL: RefCell<Option<Model>> = const { RefCell::new(None) };
    /// Kept apart from the model so the handler can call back into the bar.
    static HANDLER: RefCell<Option<SharedHandler>> = const { RefCell::new(None) };
}

fn update<T>(f: impl FnOnce(&mut Model) -> T) -> Option<T> {
    MODEL.with(|m| m.try_borrow_mut().ok().and_then(|mut m| m.as_mut().map(f)))
}

/// Calls the handler with no borrow held, since it may use the bar.
fn emit(event: MiniBarEvent) {
    let handler = HANDLER.with(|h| h.borrow().clone());
    if let Some(handler) = handler {
        handler(event);
    }
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    // SAFETY: `p` is a valid out-pointer.
    let _ = unsafe { GetCursorPos(&mut p) };
    p
}

fn window_origin(hwnd: HWND) -> POINT {
    let mut r = RECT::default();
    // SAFETY: `hwnd` is our live window; `r` is a valid out-pointer.
    let _ = unsafe { GetWindowRect(hwnd, &mut r) };
    POINT {
        x: r.left,
        y: r.top,
    }
}

/// The window procedure. Runs on the UI thread, sometimes re-entrantly
/// from our own calls (SetWindowPos, UpdateWindow), so it never holds a
/// borrow of the model across a call out and never panics.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Take the click without becoming the active window.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_LBUTTONDOWN => {
            let start = (cursor(), window_origin(hwnd));
            update(|m| {
                m.press = Some(start);
                m.dragged = false;
            });
            // SAFETY: `hwnd` is our live window, under the pressed button.
            unsafe { SetCapture(hwnd) };
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let now = cursor();
            let to = update(|m| {
                let (start, origin) = m.press?;
                let (dx, dy) = (now.x - start.x, now.y - start.y);
                let slop = f64::from(m.px(DRAG_SLOP));
                if !m.dragged && f64::from(dx).hypot(f64::from(dy)) < slop {
                    return None;
                }
                m.dragged = true;
                Some((origin.x + dx, origin.y + dy))
            })
            .flatten();
            if let Some((x, y)) = to {
                // SAFETY: `hwnd` is our live window; moving doesn't activate it.
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        x,
                        y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let ended = update(|m| m.press.take().map(|_| (m.dragged, m.size))).flatten();
            // SAFETY: releases the capture taken on button-down (harmless
            // if it was already lost).
            let _ = unsafe { ReleaseCapture() };
            match ended {
                Some((false, _)) => emit(MiniBarEvent::Click),
                Some((true, size)) => {
                    let p = keep_on_screen(window_origin(hwnd), size);
                    // SAFETY: `hwnd` is our live window.
                    let _ = unsafe {
                        SetWindowPos(
                            hwnd,
                            None,
                            p.x,
                            p.y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        )
                    };
                    emit(MiniBarEvent::Moved(MiniBarPosition {
                        x: f64::from(p.x),
                        y: f64::from(p.y),
                    }));
                }
                None => {}
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            // Lost the mouse mid-press (another window took it): drop it.
            // SAFETY: plain query.
            if unsafe { GetCapture() } != hwnd {
                update(|m| m.press = None);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            // SAFETY: `hwnd` is our window, in its own WM_PAINT; `ps` is a
            // valid out-pointer and EndPaint below pairs with this call.
            let dc = unsafe { BeginPaint(hwnd, &mut ps) };
            if !dc.is_invalid() {
                MODEL.with(|m| {
                    if let Ok(m) = m.try_borrow()
                        && let Some(m) = m.as_ref()
                    {
                        m.draw(dc);
                    }
                });
            }
            // SAFETY: pairs with BeginPaint above.
            let _ = unsafe { EndPaint(hwnd, &ps) };
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == SPIN_TIMER => {
            update(|m| m.spin = (m.spin + 1) % SPIN_DOTS);
            // SAFETY: `hwnd` is our live window.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            LRESULT(0)
        }
        // SAFETY: the default handling for everything else, with this
        // message's own arguments.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The work area and DPI scale of the monitor nearest `point`, or `None`
/// when `nearest` is off and no monitor contains it.
fn monitor_at(point: POINT, nearest: bool) -> Option<(RECT, f64)> {
    let flag = if nearest {
        MONITOR_DEFAULTTONEAREST
    } else {
        MONITOR_DEFAULTTONULL
    };
    // SAFETY: plain calls; `info` is a valid, sized out-pointer.
    unsafe {
        let monitor = MonitorFromPoint(point, flag);
        if monitor.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        Some((info.rcWork, f64::from(dpi_x.max(96)) / 96.0))
    }
}

/// By the right edge of the primary monitor's work area, a little below
/// the middle.
fn default_origin() -> (POINT, f64) {
    // SAFETY: a plain query; (0, 0) is always on the primary monitor.
    let primary = unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: `primary` is a monitor handle; the out-pointers are valid.
    unsafe {
        let _ = GetMonitorInfoW(primary, &mut info);
        let _ = GetDpiForMonitor(primary, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    let scale = f64::from(dpi_x.max(96)) / 96.0;
    let work = info.rcWork;
    let size = (SIZE * scale).round() as i32;
    let x = work.right - size - (EDGE_MARGIN * scale).round() as i32;
    let y = work.bottom - (f64::from(work.bottom - work.top) * DEFAULT_HEIGHT).round() as i32;
    (POINT { x, y: y - size }, scale)
}

/// Pulls a dropped bar fully inside the work area of the monitor its
/// center is nearest to.
fn keep_on_screen(origin: POINT, size: i32) -> POINT {
    let center = POINT {
        x: origin.x + size / 2,
        y: origin.y + size / 2,
    };
    let Some((work, _)) = monitor_at(center, true) else {
        return origin;
    };
    POINT {
        x: origin.x.min(work.right - size).max(work.left),
        y: origin.y.min(work.bottom - size).max(work.top),
    }
}

pub(crate) struct MiniBar {
    /// Also makes `MiniBar` neither `Send` nor `Sync`: it belongs to the
    /// thread that created it.
    hwnd: HWND,
}

impl MiniBar {
    /// The right-click menu isn't shown on Windows yet, so `_menu` is
    /// unused.
    pub(crate) fn new(
        position: Option<MiniBarPosition>,
        _menu: &[&str],
        handler: MiniBarHandler,
    ) -> Result<Self> {
        let failed = |what: &str, e: windows::core::Error| {
            PlatformError::Failed(format!("couldn't create the mini bar ({what}): {e}"))
        };

        // A saved place still on a monitor, else the default.
        let saved = position.map(|p| POINT {
            x: p.x.round() as i32,
            y: p.y.round() as i32,
        });
        let (origin, scale) = saved
            .and_then(|p| {
                let (_, scale) = monitor_at(p, false)?;
                let half = (SIZE * scale / 2.0).round() as i32;
                let center = POINT {
                    x: p.x + half,
                    y: p.y + half,
                };
                monitor_at(center, false).map(|(_, scale)| (p, scale))
            })
            .unwrap_or_else(default_origin);

        // SAFETY: a null name asks for this executable's own module handle.
        let module = unsafe { GetModuleHandleW(None) }.map_err(|e| failed("module", e))?;
        let class = w!("SplashdownMiniBar");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: class,
            ..Default::default()
        };
        // SAFETY: `wc` is fully initialized and its strings are static. A
        // second registration fails harmlessly (the class already exists).
        unsafe { RegisterClassExW(&wc) };

        let mut model = Model {
            state: MiniBarState::Idle,
            spin: 0,
            scale: 1.0,
            size: 0,
            font: None,
            press: None,
            dragged: false,
        };
        model.set_scale(scale);
        let size = model.size;

        // SAFETY: the class was registered above with a valid window
        // procedure; no parent, menu or creation data.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class,
                w!("Splashdown"),
                WS_POPUP,
                origin.x,
                origin.y,
                size,
                size,
                None,
                None,
                Some(module.into()),
                None,
            )
        }
        .map_err(|e| failed("window", e))?;
        // SAFETY: `hwnd` is the layered window just created.
        unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) }
            .map_err(|e| failed("opacity", e))?;
        // SAFETY: `hwnd` is our live window. The region is handed to the
        // system on success (it then owns and frees it).
        unsafe {
            let region = CreateEllipticRgn(0, 0, size + 1, size + 1);
            if !region.is_invalid() && SetWindowRgn(hwnd, Some(region), true) == 0 {
                let _ = DeleteObject(HGDIOBJ(region.0));
            }
        }

        MODEL.with(|m| *m.borrow_mut() = Some(model));
        HANDLER.with(|h| *h.borrow_mut() = Some(Rc::from(handler)));
        Ok(Self { hwnd })
    }

    pub(crate) fn show(&self) {
        // SAFETY: `self.hwnd` is our live window. Neither call activates it.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        self.repaint();
    }

    pub(crate) fn hide(&self) {
        // SAFETY: `self.hwnd` is our live window.
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    pub(crate) fn set_state(&self, state: MiniBarState) {
        update(|m| m.state = state);
        // SAFETY: `self.hwnd` is our live window; timer 1 has no callback,
        // so it arrives as WM_TIMER in `wndproc`.
        unsafe {
            if state == MiniBarState::Working {
                SetTimer(Some(self.hwnd), SPIN_TIMER, SPIN_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), SPIN_TIMER);
            }
        }
        self.repaint();
    }

    /// Not shown on Windows yet (see the module docs).
    pub(crate) fn set_tooltip(&self, _text: &str) {}

    /// Paints now, rather than whenever the message loop gets to it.
    fn repaint(&self) {
        // SAFETY: `self.hwnd` is our live window. UpdateWindow sends
        // WM_PAINT straight to `wndproc`, while no borrow of the model is
        // held.
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            let _ = UpdateWindow(self.hwnd);
        }
    }
}

impl Drop for MiniBar {
    fn drop(&mut self) {
        // SAFETY: our window, destroyed once, on the thread that created it.
        let _ = unsafe { DestroyWindow(self.hwnd) };
        MODEL.with(|m| m.borrow_mut().take());
        HANDLER.with(|h| h.borrow_mut().take());
    }
}
