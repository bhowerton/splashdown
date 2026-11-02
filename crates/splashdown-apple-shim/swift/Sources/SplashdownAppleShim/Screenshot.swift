// The screen as an image, for computer use: the main display through
// ScreenCaptureKit (macOS 14+), scaled down for the model and encoded as
// JPEG. Needs the Screen Recording permission; macOS asks the first time.

import CoreGraphics
import Foundation
import ImageIO
import ScreenCaptureKit
import UniformTypeIdentifiers

/// Captures the display under the mouse (or the main one) at most
/// `maxLongEdge` pixels on its long edge; with a region (`regionW` > 0,
/// in points from the display's corner), just that region, in its full
/// detail, scaled to the same size (computer use's zoom). Replies OK with
/// "<width> <height> <points per pixel> <display x> <display y>\n<base64 JPEG>":
/// points per pixel turn the image's coordinates back into the screen's,
/// and the display's origin places them on the whole desktop. REFUSED
/// without the permission, OS_TOO_OLD before macOS 14.
@_cdecl("splashdown_screenshot")
public func splashdown_screenshot(
    _ maxLongEdge: Int32,
    _ regionX: Double,
    _ regionY: Double,
    _ regionW: Double,
    _ regionH: Double,
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownCallback
) {
    let reply = Reply(ctx: ctx, callback: callback)
    guard #available(macOS 14.0, *) else {
        reply.send(OS_TOO_OLD, "Taking a screenshot needs macOS 14 or later")
        return
    }
    let mouse = CGEvent(source: nil)?.location ?? .zero
    Task {
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(
                false, onScreenWindowsOnly: true)
            let main = CGMainDisplayID()
            guard let display = content.displays.first(where: { $0.frame.contains(mouse) })
                ?? content.displays.first(where: { $0.displayID == main })
                ?? content.displays.first
            else {
                reply.send(FAILED, "no display to capture")
                return
            }
            // Points, as the display's frame and mouse events count them.
            let region = regionW > 0 && regionH > 0
                ? CGRect(x: regionX, y: regionY, width: regionW, height: regionH)
                : nil
            let points = region?.size ?? display.frame.size
            let longEdge = max(points.width, points.height)
            let scale = min(1.0, Double(maxLongEdge) / Double(longEdge))
            let width = Int((points.width * scale).rounded())
            let height = Int((points.height * scale).rounded())
            let config = SCStreamConfiguration()
            config.width = width
            config.height = height
            config.showsCursor = true
            if let region { config.sourceRect = region }
            let filter = SCContentFilter(display: display, excludingWindows: [])
            let image = try await SCScreenshotManager.captureImage(
                contentFilter: filter, configuration: config)
            let data = NSMutableData()
            guard let destination = CGImageDestinationCreateWithData(
                data, UTType.jpeg.identifier as CFString, 1, nil)
            else {
                reply.send(FAILED, "couldn't encode the screenshot")
                return
            }
            CGImageDestinationAddImage(
                destination, image,
                [kCGImageDestinationLossyCompressionQuality: 0.8] as CFDictionary)
            guard CGImageDestinationFinalize(destination) else {
                reply.send(FAILED, "couldn't encode the screenshot")
                return
            }
            let pointsPerPixel = Double(points.width) / Double(image.width)
            let header = "\(image.width) \(image.height) \(pointsPerPixel) \(display.frame.origin.x) \(display.frame.origin.y)"
            reply.send(OK, header + "\n" + (data as Data).base64EncodedString())
        } catch {
            let denied = (error as NSError).domain == "com.apple.ScreenCaptureKit.SCStreamErrorDomain"
                && (error as NSError).code == -3801
            reply.send(denied ? REFUSED : FAILED, error.localizedDescription)
        }
    }
}
