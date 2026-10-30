//! An installed app's display name and icon, for showing presets' app
//! rules as "Mail" with its icon instead of `com.apple.mail`; and the app
//! in front, and switching back to it (for the scratchpad).

use objc2::AllocAnyThread;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace,
    NSGraphicsContext, NSRunningApplication, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSFileManager, NSPoint, NSRect, NSSize, NSString};

use crate::{AppDisplay, PlatformError, Result};

pub(crate) fn frontmost_app() -> Option<(i32, String)> {
    let front = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let name = front
        .localizedName()
        .or_else(|| front.bundleIdentifier())
        .map(|s| s.to_string())
        .unwrap_or_default();
    Some((front.processIdentifier(), name))
}

pub(crate) fn activate_app(pid: i32) -> Result<()> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .ok_or_else(|| PlatformError::Failed("that app has quit".into()))?;
    // No options: just its frontmost window, as clicking it would.
    if app.activateWithOptions(NSApplicationActivationOptions::empty()) {
        Ok(())
    } else {
        Err(PlatformError::Failed("couldn't switch to that app".into()))
    }
}

/// Pixels per side of the icon PNG (shown at 20 points, so 2× for Retina).
const ICON_PX: isize = 40;

pub(crate) fn app_display(bundle_id: &str) -> Option<AppDisplay> {
    let workspace = NSWorkspace::sharedWorkspace();
    let url = workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id))?;
    let path = url.path()?;
    let name = NSFileManager::defaultManager()
        .displayNameAtPath(&path)
        .to_string();
    let name = name.strip_suffix(".app").unwrap_or(&name).to_string();
    let icon = icon_png(&workspace, &path);
    Some(AppDisplay {
        name,
        icon_png: icon,
    })
}

/// Draws the app's icon into a small bitmap and encodes it as PNG.
fn icon_png(workspace: &NSWorkspace, path: &NSString) -> Option<Vec<u8>> {
    let image = workspace.iconForFile(path);
    // SAFETY: a null `planes` pointer asks AppKit to allocate the pixel
    // buffer; the other arguments describe an 8-bit RGBA bitmap.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            ICON_PX,
            ICON_PX,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    let side = ICON_PX as f64;
    image.drawInRect(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side)));
    context.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    // SAFETY: an empty dictionary is a valid (no options) properties value.
    let data = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(data.to_vec())
}
