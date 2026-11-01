//! Where the app's small panels go: the call offer, questions and the
//! meeting pill, on the main screen (the one with the menu bar or taskbar).
//!
//! macOS works in points across all screens, whatever each one's scale, so
//! the position is worked out in points there. Mixing a Retina screen's
//! pixels with a window measured on another screen put panels off screen.

use tauri::WebviewWindow;

/// Where on the main screen's work area, in points.
pub enum Spot {
    /// Centered, `top` below the top.
    TopCenter { top: f64 },
    /// `top` below the top and `right` in from the right.
    TopRight { top: f64, right: f64 },
}

/// Moves `window` to `spot`.
pub fn place(window: &WebviewWindow, spot: Spot) {
    let (Ok(Some(monitor)), Ok(size), Ok(window_scale)) = (
        window.primary_monitor(),
        window.outer_size(),
        window.scale_factor(),
    ) else {
        return;
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let left = area.position.x as f64 / scale;
    let top_edge = area.position.y as f64 / scale;
    let width = area.size.width as f64 / scale;
    let own = size.to_logical::<f64>(window_scale);
    let (x, y) = match spot {
        Spot::TopCenter { top } => (left + (width - own.width) / 2.0, top_edge + top),
        Spot::TopRight { top, right } => (left + width - own.width - right, top_edge + top),
    };
    if cfg!(target_os = "macos") {
        let _ = window.set_position(tauri::LogicalPosition::new(x, y));
    } else {
        // Windows places in pixels, each screen with its own scale.
        let _ = window.set_position(tauri::PhysicalPosition::new(
            (x * scale).round() as i32,
            (y * scale).round() as i32,
        ));
    }
}
