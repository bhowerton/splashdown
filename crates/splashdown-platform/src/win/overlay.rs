//! The status pill: a borderless, topmost, layered tool window that never
//! activates and lets the mouse through, so the user's focused field keeps
//! focus. Painted with GDI; the pill shape is a window region.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DEFAULT_CHARSET,
    DeleteDC, DeleteObject, Ellipse, EndPaint, FW_SEMIBOLD, FillRect, GetDC, GetMonitorInfoW,
    GetStockObject, GetTextExtentPoint32W, HDC, HFONT, HGDIOBJ, InvalidateRect,
    MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
    MonitorFromWindow, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ReleaseDC, RoundRect, SRCCOPY,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT, TextOutW, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, HTTRANSPARENT,
    HWND_TOPMOST, IsWindowVisible, KillTimer, LWA_ALPHA, MA_NOACTIVATE, RegisterClassExW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WM_ERASEBKGND,
    WM_MOUSEACTIVATE, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

use crate::{CaretRect, OverlayState, PlatformError, Result, Tone};

// Sizes in device-independent pixels (scaled by the monitor's DPI).
/// Space between the text cursor and the pill.
const CARET_GAP: f64 = 12.0;
/// The least a caret's line is taken to be: some fields report the caret as
/// a sliver at the top of its line, and the pill would cover the line being
/// typed.
const MIN_LINE: f64 = 24.0;
const HEIGHT: f64 = 38.0;
const PAD_X: f64 = 14.0;
const DOT: f64 = 10.0;
const GAP: f64 = 10.0;
const BARS: usize = 9;
const BAR_W: f64 = 3.0;
const BAR_GAP: f64 = 2.5;
const BAR_MAX_H: f64 = 18.0;
const BAR_MIN_H: f64 = 3.0;
const FONT_PX: f64 = 14.0;
/// Distance from the bottom of the monitor's work area.
const BOTTOM_OFFSET: f64 = 72.0;

/// Timer that turns the "working" spinner.
const SPIN_TIMER: usize = 1;
const SPIN_MS: u32 = 90;
const SPIN_DOTS: u8 = 8;

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
const PURPLE: Rgb = Rgb(148, 115, 250);
const BLUE: Rgb = Rgb(115, 148, 250);
const GREEN: Rgb = Rgb(82, 207, 102);
const AMBER: Rgb = Rgb(250, 176, 5);
const GRAY: Rgb = Rgb(179, 179, 184);
const BACKGROUND: Rgb = Rgb(23, 23, 28);
const TEXT: Rgb = Rgb(245, 245, 245);

/// Appended to the label while auto-send is on.
const SEND_BADGE: &str = "  ·  ↵ Send";

/// Live text longer than this shows only its end ("…on Friday").
const LIVE_TEXT_CHARS: usize = 54;
/// A question longer than this is cut short.
const QUESTION_CHARS: usize = 70;

/// What the pill shows, plus its current geometry in physical pixels.
struct Model {
    dot: Rgb,
    working: bool,
    spin: u8,
    base_label: String,
    live: Option<String>,
    send: bool,
    /// Waveform color, or `None` when the current state has no waveform.
    wave: Option<Rgb>,
    levels: VecDeque<f32>,
    scale: f64,
    font: Option<HFONT>,
    width: i32,
    height: i32,
    label_x: i32,
    label_y: i32,
}

impl Model {
    fn new() -> Self {
        Self {
            dot: RED,
            working: false,
            spin: 0,
            base_label: String::new(),
            live: None,
            send: false,
            wave: None,
            levels: VecDeque::from(vec![0.0; BARS]),
            scale: 1.0,
            font: None,
            width: 0,
            height: 0,
            label_x: 0,
            label_y: 0,
        }
    }

    fn px(&self, v: f64) -> i32 {
        (v * self.scale).round() as i32
    }

    /// Live text or the state's label, plus the auto-send badge.
    fn label(&self) -> String {
        let mut text = self.live.clone().unwrap_or_else(|| self.base_label.clone());
        if self.send {
            text.push_str(SEND_BADGE);
        }
        text
    }

    fn set_scale(&mut self, scale: f64) {
        if self.font.is_some() && (scale - self.scale).abs() < f64::EPSILON {
            return;
        }
        self.scale = scale;
        if let Some(old) = self.font.take() {
            // SAFETY: the font was created by us and isn't selected into any
            // DC (each paint restores the DC's previous font).
            let _ = unsafe { DeleteObject(old.into()) };
        }
        // SAFETY: plain values and a NUL-terminated face name; returns a new
        // font (or null, which GDI treats as "use the default font").
        let font = unsafe {
            CreateFontW(
                -self.px(FONT_PX),
                0,
                0,
                0,
                FW_SEMIBOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            )
        };
        self.font = (!font.is_invalid()).then_some(font);
    }

    /// Recomputes the pill's size for the current label and scale.
    fn measure(&mut self) {
        let text: Vec<u16> = self.label().encode_utf16().collect();
        let mut extent = SIZE::default();
        // SAFETY: the screen DC is released below; the font (ours, or the
        // default) is selected only for the measurement and then restored.
        unsafe {
            let dc = GetDC(None);
            let old = self.font.map(|f| SelectObject(dc, f.into()));
            let _ = GetTextExtentPoint32W(dc, &text, &mut extent);
            if let Some(old) = old {
                SelectObject(dc, old);
            }
            ReleaseDC(None, dc);
        }
        let wave_w = if self.wave.is_some() {
            GAP + 2.0 + BARS as f64 * (BAR_W + BAR_GAP) - BAR_GAP
        } else {
            0.0
        };
        self.height = self.px(HEIGHT);
        self.label_x = self.px(PAD_X + DOT + GAP);
        self.label_y = (self.height - extent.cy) / 2;
        self.width = self.label_x + extent.cx + self.px(wave_w + PAD_X);
    }

    fn bars_x(&self, label_w: i32) -> f64 {
        f64::from(self.label_x + label_w) + (GAP + 2.0) * self.scale
    }

    fn draw(&self, dc: HDC) {
        let (w, h) = (self.width, self.height);
        if w <= 0 || h <= 0 {
            return;
        }
        let text: Vec<u16> = self.label().encode_utf16().collect();
        // SAFETY: every GDI object created here is selected out and deleted
        // before returning; `dc` is the paint DC, valid for this WM_PAINT.
        unsafe {
            // Draw off-screen, then copy, so updates don't flicker.
            let mem = CreateCompatibleDC(Some(dc));
            let bitmap = CreateCompatibleBitmap(dc, w, h);
            let old_bitmap = SelectObject(mem, bitmap.into());
            let old_pen = SelectObject(mem, GetStockObject(NULL_PEN));

            let background = CreateSolidBrush(BACKGROUND.colorref());
            FillRect(
                mem,
                &RECT {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: h,
                },
                background,
            );
            let _ = DeleteObject(background.into());

            let cx = f64::from(self.px(PAD_X)) + DOT * self.scale / 2.0;
            let cy = f64::from(h) / 2.0;
            if self.working {
                // A ring of dots, brightest at the current step.
                let ring = 5.5 * self.scale;
                let r = 1.4 * self.scale;
                for i in 0..SPIN_DOTS {
                    let age = (self.spin + SPIN_DOTS - i) % SPIN_DOTS;
                    let t = 1.0 - f64::from(age) / f64::from(SPIN_DOTS);
                    let angle = f64::from(i) * std::f64::consts::TAU / f64::from(SPIN_DOTS);
                    let (x, y) = (cx + ring * angle.sin(), cy - ring * angle.cos());
                    fill_ellipse(mem, BACKGROUND.mix(TEXT, 0.15 + 0.85 * t), x, y, r);
                }
            } else {
                fill_ellipse(mem, self.dot, cx, cy, DOT * self.scale / 2.0);
            }

            let mut extent = SIZE::default();
            let old_font = self.font.map(|f| SelectObject(mem, f.into()));
            let _ = GetTextExtentPoint32W(mem, &text, &mut extent);
            SetBkMode(mem, TRANSPARENT);
            SetTextColor(mem, TEXT.colorref());
            let _ = TextOutW(mem, self.label_x, self.label_y, &text);
            if let Some(old) = old_font {
                SelectObject(mem, old);
            }

            if let Some(color) = self.wave {
                let brush = CreateSolidBrush(color.colorref());
                let old_brush = SelectObject(mem, brush.into());
                let x0 = self.bars_x(extent.cx);
                for (i, level) in self.levels.iter().enumerate() {
                    // Speech RMS is roughly 0.01–0.2; a square-root curve
                    // makes quiet speech visible without loud speech pinning
                    // the bars.
                    let norm = (f64::from(*level) * 25.0).sqrt().min(1.0);
                    let bar_h = (BAR_MIN_H + norm * (BAR_MAX_H - BAR_MIN_H)) * self.scale;
                    let x = x0 + i as f64 * (BAR_W + BAR_GAP) * self.scale;
                    let top = (cy - bar_h / 2.0).round() as i32;
                    let bottom = (cy + bar_h / 2.0).round() as i32;
                    let bw = self.px(BAR_W).max(1);
                    let left = x.round() as i32;
                    let _ = RoundRect(mem, left, top, left + bw + 1, bottom + 1, bw, bw);
                }
                SelectObject(mem, old_brush);
                let _ = DeleteObject(brush.into());
            }

            let _ = BitBlt(dc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
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

thread_local! {
    /// The pill's state, on the UI thread that owns the window. The window
    /// procedure reads it to paint, so it can't live in `Overlay` itself.
    static MODEL: RefCell<Option<Model>> = const { RefCell::new(None) };
}

/// The window procedure. Runs on the UI thread, sometimes re-entrantly
/// from our own calls (SetWindowPos, UpdateWindow), so it never holds a
/// mutable borrow of the model and never panics.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Clicks go to whatever is underneath.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
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
            MODEL.with(|m| {
                if let Ok(mut m) = m.try_borrow_mut()
                    && let Some(m) = m.as_mut()
                {
                    m.spin = (m.spin + 1) % SPIN_DOTS;
                }
            });
            // SAFETY: `hwnd` is our live window.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            LRESULT(0)
        }
        // SAFETY: the default handling for everything else, with this
        // message's own arguments.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub(crate) struct Overlay {
    /// Also makes `Overlay` neither `Send` nor `Sync`: it belongs to the
    /// thread that created it.
    hwnd: HWND,
    /// The text cursor to show the pill by, in physical pixels; `None`
    /// for the usual place.
    anchor: Cell<Option<CaretRect>>,
}

impl Overlay {
    pub(crate) fn new() -> Result<Self> {
        let failed = |what: &str, e: windows::core::Error| {
            PlatformError::Failed(format!("couldn't create the overlay ({what}): {e}"))
        };
        // SAFETY: a null name asks for this executable's own module handle.
        let module = unsafe { GetModuleHandleW(None) }.map_err(|e| failed("module", e))?;
        let class = w!("SplashdownOverlay");
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

        // SAFETY: the class was registered above with a valid window
        // procedure; no parent, menu or creation data.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE
                    | WS_EX_LAYERED
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST,
                class,
                w!("Splashdown"),
                WS_POPUP,
                0,
                0,
                1,
                1,
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

        MODEL.with(|m| *m.borrow_mut() = Some(Model::new()));
        Ok(Self {
            hwnd,
            anchor: Cell::new(None),
        })
    }

    fn update(&self, f: impl FnOnce(&mut Model)) {
        MODEL.with(|m| {
            if let Ok(mut m) = m.try_borrow_mut()
                && let Some(m) = m.as_mut()
            {
                f(m);
            }
        });
    }

    fn read<T>(&self, f: impl FnOnce(&Model) -> T) -> Option<T> {
        MODEL.with(|m| m.try_borrow().ok().and_then(|m| m.as_ref().map(f)))
    }

    pub(crate) fn show(&self, state: &OverlayState) {
        let (dot, text, wave) = match state {
            OverlayState::Listening { preset } => {
                (RED, with_preset("Listening", preset), Some(RED))
            }
            OverlayState::Instruction => (PURPLE, "Instruction".to_string(), Some(PURPLE)),
            OverlayState::Transform { words } => (
                BLUE,
                format!(
                    "Transform · {words} word{}",
                    if *words == 1 { "" } else { "s" }
                ),
                Some(BLUE),
            ),
            OverlayState::HandsFree { preset } => (
                RED,
                with_preset("Hands-free · tap to finish", preset),
                Some(RED),
            ),
            OverlayState::Question { text } => (
                RED,
                format!("{} · tap to answer", head_chars(text, QUESTION_CHARS)),
                Some(RED),
            ),
            OverlayState::Working { label } => (BLUE, label.clone(), None),
            OverlayState::Reading { label } => (GREEN, label.clone(), None),
            OverlayState::Message { tone, text } => {
                let c = match tone {
                    Tone::Info => GRAY,
                    Tone::Success => GREEN,
                    Tone::Warning => AMBER,
                };
                (c, text.clone(), None)
            }
        };
        let working = matches!(state, OverlayState::Working { .. });
        self.update(|m| {
            m.dot = dot;
            m.working = working;
            // A new recording or a final message ends any auto-send badge.
            if matches!(
                state,
                OverlayState::Listening { .. } | OverlayState::Message { .. }
            ) {
                m.send = false;
            }
            m.base_label = text;
            m.live = None;
            m.wave = wave;
        });
        // SAFETY: `self.hwnd` is our live window; timer 1 has no callback,
        // so it arrives as WM_TIMER in `wndproc`.
        unsafe {
            if working {
                SetTimer(Some(self.hwnd), SPIN_TIMER, SPIN_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), SPIN_TIMER);
            }
        }
        self.layout(true);
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

    pub(crate) fn set_level(&self, level: f32) {
        self.update(|m| {
            m.levels.pop_front();
            m.levels.push_back(level);
        });
        if self.read(|m| m.wave.is_some()) == Some(true) {
            self.repaint();
        }
    }

    pub(crate) fn set_live_text(&self, text: Option<&str>) {
        if self.read(|m| m.wave.is_none()) != Some(false) {
            return;
        }
        let live = text
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| tail_chars(t, LIVE_TEXT_CHARS));
        self.update(|m| m.live = live);
        self.layout(false);
        self.repaint();
    }

    pub(crate) fn set_send(&self, on: bool) {
        self.update(|m| m.send = on);
        self.layout(false);
        self.repaint();
    }

    pub(crate) fn set_alpha(&self, alpha: f64) {
        let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        // SAFETY: `self.hwnd` is our live layered window.
        let _ = unsafe { SetLayeredWindowAttributes(self.hwnd, COLORREF(0), alpha, LWA_ALPHA) };
    }

    pub(crate) fn is_visible(&self) -> bool {
        // SAFETY: `self.hwnd` is our live window.
        unsafe { IsWindowVisible(self.hwnd) }.as_bool()
    }

    /// Screen readers would need a UI Automation notification event raised
    /// from a provider on this window (`UiaRaiseNotificationEvent`), which
    /// this window doesn't implement yet, so nothing is announced on
    /// Windows; the pill's text is visual only.
    /// Shows the pill by the text cursor from now on (`None`: the usual
    /// place, at the bottom of the screen).
    pub(crate) fn set_anchor(&self, caret: Option<CaretRect>) {
        self.anchor.set(caret);
    }

    pub(crate) fn announce(&self, _text: &str) {}

    pub(crate) fn hide(&self) {
        self.update(|m| {
            m.send = false;
            m.working = false;
            m.levels.iter_mut().for_each(|l| *l = 0.0);
        });
        // SAFETY: `self.hwnd` is our live window.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), SPIN_TIMER);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// Resizes the pill for its label and puts it just below the text
    /// cursor when that's known, else at the bottom center of the monitor
    /// with the focused window. `reposition` also moves it to that monitor
    /// (when it's shown); otherwise it stays centered where it is.
    fn layout(&self, reposition: bool) {
        let caret = self.anchor.get();
        // SAFETY: plain calls; a null foreground window falls back to the
        // primary monitor, and `info` is a valid, sized out-pointer.
        let (work, scale, caret) = unsafe {
            // The caret's monitor, if it's on one.
            let by_caret = caret.and_then(|c| {
                let point = POINT {
                    x: c.x.round() as i32,
                    y: (c.y + c.height / 2.0).round() as i32,
                };
                let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONULL);
                (!monitor.is_invalid()).then_some((monitor, c))
            });
            let anchor = if reposition {
                GetForegroundWindow()
            } else {
                self.hwnd
            };
            let monitor = by_caret.map_or_else(
                || MonitorFromWindow(anchor, MONITOR_DEFAULTTOPRIMARY),
                |(m, _)| m,
            );
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            (
                info.rcWork,
                f64::from(dpi_x.max(96)) / 96.0,
                by_caret.map(|(_, c)| c),
            )
        };

        let Some((width, height)) = self.update_and_read(|m| {
            m.set_scale(scale);
            m.measure();
            (m.width, m.height)
        }) else {
            return;
        };
        let (x, y) = match caret {
            Some(caret) => near_caret(caret, work, width, height, scale),
            None => (
                work.left + (work.right - work.left - width) / 2,
                work.bottom - (BOTTOM_OFFSET * scale).round() as i32 - height,
            ),
        };
        // SAFETY: `self.hwnd` is our live window. The region is handed to
        // the system on success (it then owns and frees it).
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE,
            );
            let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, height, height);
            if !region.is_invalid() && SetWindowRgn(self.hwnd, Some(region), true) == 0 {
                let _ = DeleteObject(HGDIOBJ(region.0));
            }
        }
    }

    fn update_and_read<T>(&self, f: impl FnOnce(&mut Model) -> T) -> Option<T> {
        MODEL.with(|m| m.try_borrow_mut().ok().and_then(|mut m| m.as_mut().map(f)))
    }

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

/// Where the pill goes for a caret: just below its line, or above it when
/// there's no room below, kept inside the monitor's work area. Physical
/// pixels throughout; `scale` is the monitor's DPI over 96.
fn near_caret(caret: CaretRect, work: RECT, width: i32, height: i32, scale: f64) -> (i32, i32) {
    let line = caret.height.max(MIN_LINE * scale);
    let gap = CARET_GAP * scale;
    let below = caret.y + line + gap;
    let y = if below + f64::from(height) <= f64::from(work.bottom) {
        below
    } else {
        caret.y - gap - f64::from(height)
    };
    let x = (caret.x - PAD_X * scale)
        .min(f64::from(work.right - width))
        .max(f64::from(work.left));
    let y = y
        .min(f64::from(work.bottom - height))
        .max(f64::from(work.top));
    (x.round() as i32, y.round() as i32)
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // SAFETY: our window, destroyed once, on the thread that created it.
        let _ = unsafe { DestroyWindow(self.hwnd) };
        MODEL.with(|m| m.borrow_mut().take());
    }
}

fn with_preset(label: &str, preset: &Option<String>) -> String {
    match preset {
        Some(p) => format!("{label} · {p}"),
        None => label.to_string(),
    }
}

/// The first `max` characters of `text` on one line, with an ellipsis
/// when anything was cut.
fn head_chars(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", text[..i].trim_end()),
        None => text,
    }
}

/// The last `max` characters of `text`, starting at a word boundary, with a
/// leading ellipsis when anything was cut.
fn tail_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let start = text.char_indices().nth(count - max).map_or(0, |(i, _)| i);
    let tail = &text[start..];
    let tail = tail.split_once(' ').map_or(tail, |(_, rest)| rest);
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::{CaretRect, RECT, near_caret, tail_chars};

    #[test]
    fn the_pill_goes_below_the_caret_or_above_it() {
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let caret = |x, y| CaretRect {
            x,
            y,
            width: 1.0,
            height: 18.0,
        };
        // Below the line (at least 24 px of it), 12 px down, at 150%.
        assert_eq!(
            near_caret(caret(500.0, 300.0), work, 300, 57, 1.5),
            (479, 354)
        );
        // No room below: above.
        assert_eq!(
            near_caret(caret(500.0, 1000.0), work, 300, 57, 1.5),
            (479, 925)
        );
        // Kept on the screen at the right edge.
        assert_eq!(near_caret(caret(1900.0, 300.0), work, 300, 57, 1.5).0, 1620);
    }

    #[test]
    fn tail_keeps_whole_words() {
        assert_eq!(tail_chars("short", 10), "short");
        assert_eq!(
            tail_chars("so I think we should ship it on Friday", 16),
            "…it on Friday"
        );
    }
}
