//! The mini bar: a small pill with a microphone and a short sound wave, a
//! topmost tool window that stays on screen, for starting and finishing
//! dictation with the mouse. Both turn orange under the pointer; the wave
//! becomes the sound's level while it listens, and a row of dots that
//! sweeps while it works.
//!
//! Unlike the overlay it takes clicks, but it never activates: it's created
//! with `WS_EX_NOACTIVATE` and answers `WM_MOUSEACTIVATE` with
//! `MA_NOACTIVATE`, so the foreground window and its focused field stay as
//! they were. Dragging uses mouse capture rather than the system's move
//! loop, which would activate the window. Painted with GDI; the pill's
//! shape is a window region.
//!
//! Not done on Windows yet: the tooltip and screen-reader name (they need a
//! tooltip control and a UI Automation provider), and the right-click menu
//! (`TrackPopupMenu` needs the window in the foreground to close properly).

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    Arc, BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen,
    CreateRoundRectRgn, CreateSolidBrush, DeleteDC, DeleteObject, EndPaint, FillRect,
    GetMonitorInfoW, GetStockObject, HDC, HGDIOBJ, InvalidateRect, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, NULL_PEN,
    PAINTSTRUCT, PS_SOLID, RoundRect, SRCCOPY, SelectObject, SetWindowRgn, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
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
/// The pill.
const WIDTH: f64 = 50.0;
const HEIGHT: f64 = 16.0;
/// The microphone, and the space between it and the wave.
const MIC_W: f64 = 7.0;
const MIC_H: f64 = 10.0;
const SPACE: f64 = 5.0;
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
/// The orange line around it while it listens.
const RING: f64 = 1.5;
/// The mouse must move this far before a press counts as a drag.
const DRAG_SLOP: f64 = 3.0;
/// Default place: the bottom middle of the work area, this far above the
/// taskbar.
const BOTTOM_MARGIN: f64 = 10.0;

/// The pointer left the window (asked for with `TrackMouseEvent`). The
/// bindings keep it with the common controls, which aren't used here.
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Timer that moves the sweep which shows it's working.
const SPIN_TIMER: usize = 1;
const SPIN_MS: u32 = 110;
/// The sweep goes to the last dot and back.
const SWEEP_STEPS: usize = 2 * BARS - 2;

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

const ORANGE: Rgb = Rgb(255, 154, 61);
const BACKGROUND: Rgb = Rgb(23, 23, 28);
const TEXT: Rgb = Rgb(245, 245, 245);

/// What the bar shows, its size, and the drag in progress.
struct Model {
    state: MiniBarState,
    /// Where the sweep is, while it works.
    step: usize,
    /// The pointer is over the pill.
    hover: bool,
    /// The last levels heard (0–1), newest last: the middle bar shows the
    /// newest, the ones beside it the ones before.
    levels: [f32; 3],
    scale: f64,
    /// Width and height in physical pixels.
    width: i32,
    height: i32,
    /// Cursor and window position when the button went down.
    press: Option<(POINT, POINT)>,
    dragged: bool,
}

impl Model {
    fn px(&self, v: f64) -> i32 {
        (v * self.scale).round() as i32
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.width = self.px(WIDTH);
        self.height = self.px(HEIGHT);
    }

    fn draw(&self, dc: HDC) {
        let (w, h) = (self.width, self.height);
        if w <= 0 || h <= 0 {
            return;
        }
        // SAFETY: every GDI object created here is selected out and deleted
        // before returning; `dc` is the paint DC, valid for this WM_PAINT.
        unsafe {
            // Draw off-screen, then copy, so updates don't flicker.
            let mem = CreateCompatibleDC(Some(dc));
            let bitmap = CreateCompatibleBitmap(dc, w, h);
            let old_bitmap = SelectObject(mem, bitmap.into());
            let old_pen = SelectObject(mem, GetStockObject(NULL_PEN));

            let rect = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            if self.state == MiniBarState::Recording {
                // An orange line around it: orange all over, then the
                // pill's own color a little inside.
                fill(mem, ORANGE, &rect);
                let ring = self.px(RING).max(1);
                fill_round(
                    mem,
                    BACKGROUND,
                    RECT {
                        left: ring,
                        top: ring,
                        right: w - ring,
                        bottom: h - ring,
                    },
                    h - 2 * ring,
                );
            } else {
                fill(mem, BACKGROUND, &rect);
            }

            let lit = if self.step < BARS {
                self.step
            } else {
                SWEEP_STEPS - self.step
            };
            let wave = BARS as f64 * BAR_W + (BARS - 1) as f64 * GAP;
            let mic_x = (WIDTH - (MIC_W + SPACE + wave)) / 2.0;
            let quiet = |share: f64| BACKGROUND.mix(TEXT, share);
            self.draw_mic(
                mem,
                mic_x,
                (HEIGHT - MIC_H) / 2.0,
                match self.state {
                    MiniBarState::Recording => ORANGE,
                    MiniBarState::Idle if self.hover => ORANGE,
                    MiniBarState::NotReady => quiet(0.3),
                    MiniBarState::Idle | MiniBarState::Working => quiet(0.85),
                },
            );

            let x0 = mic_x + MIC_W + SPACE;
            let cy = f64::from(h) / 2.0;
            for (i, share) in ENVELOPE.iter().enumerate() {
                let (height, color) = match self.state {
                    MiniBarState::Recording => {
                        // Speech RMS is roughly 0.01–0.2; a square-root
                        // curve makes quiet speech visible without loud
                        // speech pinning the bars (as the overlay's does).
                        let level = f64::from(self.levels[2 - i.abs_diff(BARS / 2)]);
                        let norm = (level * 25.0).sqrt().min(1.0) * share;
                        (BAR_MIN_H + norm * (BAR_MAX_H - BAR_MIN_H), ORANGE)
                    }
                    MiniBarState::Working => {
                        let bright = match i.abs_diff(lit) {
                            0 => 1.0,
                            1 => 0.55,
                            _ => 0.28,
                        };
                        (DOT_H, quiet(bright))
                    }
                    MiniBarState::NotReady => (REST[i], quiet(0.25)),
                    MiniBarState::Idle if self.hover => (REST[i], ORANGE),
                    MiniBarState::Idle => (REST[i], quiet(0.7)),
                };
                let left = self.px(x0 + i as f64 * (BAR_W + GAP));
                let half_h = height * self.scale / 2.0;
                fill_round(
                    mem,
                    color,
                    RECT {
                        left,
                        top: (cy - half_h).round() as i32,
                        right: left + self.px(BAR_W).max(1),
                        bottom: (cy + half_h).round() as i32,
                    },
                    self.px(BAR_W),
                );
            }

            let _ = BitBlt(dc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old_pen);
            SelectObject(mem, old_bitmap);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(mem);
        }
    }

    /// Draws the microphone with its top left corner at (`x`, `y`), in
    /// device-independent pixels: a capsule for its head, the cup that
    /// holds it, a stem and a foot.
    ///
    /// # Safety
    ///
    /// `dc` must be a valid device context with the null pen selected.
    unsafe fn draw_mic(&self, dc: HDC, x: f64, y: f64, color: Rgb) {
        let at = |dx: f64, dy: f64| (self.px(x + dx), self.px(y + dy));
        let rect = |(left, top): (i32, i32), (right, bottom): (i32, i32)| RECT {
            left,
            top,
            right: right.max(left + 1),
            bottom: bottom.max(top + 1),
        };
        // SAFETY: the caller guarantees `dc`; the pen is selected out and
        // deleted before returning.
        unsafe {
            let head = rect(at(2.0, 0.0), at(5.0, 6.0));
            fill_round(dc, color, head, head.right - head.left);
            fill(dc, color, &rect(at(3.0, 7.5), at(4.0, 9.0)));
            fill(dc, color, &rect(at(2.0, 9.0), at(5.0, 10.0)));

            // The cup: the lower half of a circle around the head. Arcs run
            // counterclockwise, so from its left end round to its right.
            let (left, top) = at(0.5, 1.5);
            let (right, bottom) = at(6.5, 7.5);
            let middle = (top + bottom) / 2;
            let pen = CreatePen(PS_SOLID, self.px(1.0).max(1), color.colorref());
            let old = SelectObject(dc, pen.into());
            let _ = Arc(dc, left, top, right, bottom, left, middle, right, middle);
            SelectObject(dc, old);
            let _ = DeleteObject(pen.into());
        }
    }
}

/// Fills `rect` with the null pen selected (no outline).
///
/// # Safety
///
/// `dc` must be a valid device context.
unsafe fn fill(dc: HDC, color: Rgb, rect: &RECT) {
    // SAFETY: the caller guarantees `dc`; the brush is deleted before
    // returning.
    unsafe {
        let brush = CreateSolidBrush(color.colorref());
        FillRect(dc, rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

/// Fills `rect` with corners rounded by `round` (their width and height),
/// with the null pen selected (no outline).
///
/// # Safety
///
/// `dc` must be a valid device context.
unsafe fn fill_round(dc: HDC, color: Rgb, rect: RECT, round: i32) {
    // SAFETY: the caller guarantees `dc`; the brush is selected out and
    // deleted before returning.
    unsafe {
        let brush = CreateSolidBrush(color.colorref());
        let old = SelectObject(dc, brush.into());
        let _ = RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right + 1,
            rect.bottom + 1,
            round,
            round,
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
            // The pointer has come over the pill: say so, and ask to be
            // told when it leaves.
            if update(|m| !std::mem::replace(&mut m.hover, true)) == Some(true) {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: `track` is fully initialized and `hwnd` is our
                // live window.
                unsafe {
                    let _ = TrackMouseEvent(&mut track);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
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
            let ended =
                update(|m| m.press.take().map(|_| (m.dragged, (m.width, m.height)))).flatten();
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
        WM_MOUSELEAVE => {
            update(|m| m.hover = false);
            // SAFETY: `hwnd` is our live window.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
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
            update(|m| m.step = (m.step + 1) % SWEEP_STEPS);
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

/// The bottom middle of the primary monitor's work area, just above the
/// taskbar.
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
    let width = (WIDTH * scale).round() as i32;
    let height = (HEIGHT * scale).round() as i32;
    let x = work.left + (work.right - work.left - width) / 2;
    let y = work.bottom - height - (BOTTOM_MARGIN * scale).round() as i32;
    (POINT { x, y }, scale)
}

/// Pulls a dropped bar fully inside the work area of the monitor its
/// center is nearest to.
fn keep_on_screen(origin: POINT, (width, height): (i32, i32)) -> POINT {
    let center = POINT {
        x: origin.x + width / 2,
        y: origin.y + height / 2,
    };
    let Some((work, _)) = monitor_at(center, true) else {
        return origin;
    };
    POINT {
        x: origin.x.min(work.right - width).max(work.left),
        y: origin.y.min(work.bottom - height).max(work.top),
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
                let center = POINT {
                    x: p.x + (WIDTH * scale / 2.0).round() as i32,
                    y: p.y + (HEIGHT * scale / 2.0).round() as i32,
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
            step: BARS / 2,
            hover: false,
            levels: [0.0; 3],
            scale: 1.0,
            width: 0,
            height: 0,
            press: None,
            dragged: false,
        };
        model.set_scale(scale);
        let (width, height) = (model.width, model.height);

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
                width,
                height,
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
            let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, height, height);
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
        update(|m| {
            if m.state != state {
                m.levels = [0.0; 3];
                m.step = BARS / 2;
            }
            m.state = state;
        });
        // The dots stay still when motion is to be kept down.
        let sweep = state == MiniBarState::Working && !crate::reduce_motion();
        // SAFETY: `self.hwnd` is our live window; timer 1 has no callback,
        // so it arrives as WM_TIMER in `wndproc`.
        unsafe {
            if sweep {
                SetTimer(Some(self.hwnd), SPIN_TIMER, SPIN_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), SPIN_TIMER);
            }
        }
        self.repaint();
    }

    /// The live microphone level (0–1), shown while it listens.
    pub(crate) fn set_level(&self, level: f32) {
        let listening = update(|m| {
            m.levels.rotate_left(1);
            m.levels[2] = level;
            m.state == MiniBarState::Recording
        });
        if listening == Some(true) {
            // SAFETY: `self.hwnd` is our live window.
            let _ = unsafe { InvalidateRect(Some(self.hwnd), None, false) };
        }
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
