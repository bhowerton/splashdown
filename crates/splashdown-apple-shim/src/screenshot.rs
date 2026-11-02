//! The screen as an image, for computer use, through the Swift shim's
//! ScreenCaptureKit call.

use std::ffi::c_void;

use base64::Engine;

use crate::ffi;

/// A screenshot, scaled for a model.
#[derive(Debug, Clone)]
pub struct Screenshot {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Screen points per image pixel: multiply an image coordinate by it
    /// (and add the display's origin) for the screen's.
    pub points_per_pixel: f64,
    /// The captured display's top-left corner, in screen points.
    pub origin: (f64, f64),
}

impl Screenshot {
    /// Where a point in the image is on the screen, in points (what mouse
    /// events take).
    pub fn to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.origin.0 + x * self.points_per_pixel,
            self.origin.1 + y * self.points_per_pixel,
        )
    }

    /// The other way round.
    pub fn to_image(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (x - self.origin.0) / self.points_per_pixel,
            (y - self.origin.1) / self.points_per_pixel,
        )
    }
}

/// Why there's no screenshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenshotError {
    /// Screen Recording isn't allowed (System Settings → Privacy &
    /// Security).
    NoPermission,
    Failed(String),
}

/// The display under the mouse, at most `max_long_edge` pixels on its long
/// edge. macOS asks for Screen Recording the first time.
pub async fn screenshot(max_long_edge: u32) -> Result<Screenshot, ScreenshotError> {
    capture(max_long_edge, None).await
}

/// Part of the display under the mouse in its full detail, scaled to at
/// most `max_long_edge` pixels: `region` is (x, y, width, height) in
/// points from the display's top-left corner. Its `points_per_pixel` and
/// `origin` describe the region.
pub async fn zoom(
    max_long_edge: u32,
    region: (f64, f64, f64, f64),
) -> Result<Screenshot, ScreenshotError> {
    let mut shot = capture(max_long_edge, Some(region)).await?;
    shot.origin = (shot.origin.0 + region.0, shot.origin.1 + region.1);
    Ok(shot)
}

async fn capture(
    max_long_edge: u32,
    region: Option<(f64, f64, f64, f64)>,
) -> Result<Screenshot, ScreenshotError> {
    let edge = i32::try_from(max_long_edge).unwrap_or(i32::MAX);
    let (x, y, w, h) = region.unwrap_or((0.0, 0.0, 0.0, 0.0));
    // SAFETY: the shim replies exactly once through the callback.
    let (status, text) = ffi::call(|ctx: *mut c_void, cb| unsafe {
        ffi::splashdown_screenshot(edge, x, y, w, h, ctx, cb)
    })
    .await;
    match status {
        ffi::OK => parse(&text).ok_or_else(|| ScreenshotError::Failed("unreadable reply".into())),
        ffi::REFUSED => Err(ScreenshotError::NoPermission),
        _ => Err(ScreenshotError::Failed(text)),
    }
}

fn parse(text: &str) -> Option<Screenshot> {
    let (header, data) = text.split_once('\n')?;
    let mut f = header.split_whitespace();
    let width = f.next()?.parse().ok()?;
    let height = f.next()?.parse().ok()?;
    let points_per_pixel = f.next()?.parse().ok()?;
    let origin = (f.next()?.parse().ok()?, f.next()?.parse().ok()?);
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .ok()?;
    Some(Screenshot {
        jpeg,
        width,
        height,
        points_per_pixel,
        origin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_reply_and_maps_coordinates() {
        let shot = parse("1280 831 1.18125 0 -25\n/9j/AA==").unwrap();
        assert_eq!((shot.width, shot.height), (1280, 831));
        assert_eq!(shot.jpeg, vec![0xff, 0xd8, 0xff, 0x00]);
        let (x, y) = shot.to_screen(640.0, 100.0);
        assert!((x - 756.0).abs() < 1e-9 && (y - 93.125).abs() < 1e-9);
        let (ix, iy) = shot.to_image(x, y);
        assert!((ix - 640.0).abs() < 1e-9 && (iy - 100.0).abs() < 1e-9);
        assert!(parse("garbage").is_none());
    }
}
