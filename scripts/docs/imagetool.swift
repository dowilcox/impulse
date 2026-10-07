// Composes a documentation screenshot from Impulse's headless snapshot
// windows. Compiled and run by capture.py:
//
//     imagetool spec.json
//
// The spec (all geometry in points, top-left origin, relative to `base`):
//
//     {
//       "scale": 2,
//       "base": "window-0.png",
//       "overlays": [{"path": "window-3.png", "x": 400, "y": 40}],
//       "crop": {"x": 0, "y": 0, "w": 1280, "h": 800},      // optional
//       "radius": 12,                                       // corner radius
//       "shadow": true,                                     // window shadow
//       "border": true,                                     // hairline edge
//       "out": "docs/images/name.png"
//     }
//
// Or, to set finished images side by side (each keeps its own frame):
//
//     {"scale": 2, "beside": ["a.png", "b.png"], "gap": 24, "out": "…"}

import AppKit
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

struct Overlay: Decodable {
  let path: String
  let x: Double
  let y: Double
  /// Corner radius to round the overlay itself with (sheets, panels).
  let radius: Double?
  /// Shadow under the overlay (panels and sheets float).
  let shadow: Bool?
}

struct Rect: Decodable { let x, y, w, h: Double }

struct Spec: Decodable {
  let scale: Double
  let base: String?
  let beside: [String]?
  let gap: Double?
  let overlays: [Overlay]?
  let crop: Rect?
  let radius: Double?
  let shadow: Bool?
  let border: Bool?
  let out: String
}

func fail(_ message: String) -> Never {
  FileHandle.standardError.write(Data((message + "\n").utf8))
  exit(1)
}

func load(_ path: String) -> CGImage {
  guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
    let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
  else { fail("can't read \(path)") }
  return image
}

func context(width: Int, height: Int) -> CGContext {
  guard
    let ctx = CGContext(
      data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpace(name: CGColorSpace.sRGB)!,
      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
  else { fail("can't make a \(width)x\(height) bitmap") }
  return ctx
}

/// Draws `image` with its top-left at (x, y) in a top-left coordinate space
/// of `height` pixels, optionally rounded and shadowed.
func draw(_ image: CGImage, in ctx: CGContext, x: Double, y: Double, height: Double, radius: Double, shadow: Bool, scale: Double) {
  let rect = CGRect(x: x, y: height - y - Double(image.height), width: Double(image.width), height: Double(image.height))
  let path = CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil)
  ctx.saveGState()
  if shadow {
    // The shadow follows the image's own alpha (a toast panel is mostly
    // transparent around its pill), so draw it as one transparency layer.
    ctx.setShadow(
      offset: CGSize(width: 0, height: -14 * scale), blur: 44 * scale,
      color: CGColor(srgbRed: 0, green: 0, blue: 0, alpha: 0.45))
  }
  ctx.beginTransparencyLayer(auxiliaryInfo: nil)
  ctx.addPath(path)
  ctx.clip()
  ctx.draw(image, in: rect)
  ctx.endTransparencyLayer()
  ctx.restoreGState()
}

guard CommandLine.arguments.count == 2 else { fail("usage: imagetool spec.json") }
let specURL = URL(fileURLWithPath: CommandLine.arguments[1])
let spec: Spec
do {
  spec = try JSONDecoder().decode(Spec.self, from: Data(contentsOf: specURL))
} catch {
  fail("bad spec: \(error)")
}
let s = spec.scale

func save(_ image: CGImage) {
  let outURL = URL(fileURLWithPath: spec.out)
  try? FileManager.default.createDirectory(at: outURL.deletingLastPathComponent(), withIntermediateDirectories: true)
  guard let dest = CGImageDestinationCreateWithURL(outURL as CFURL, UTType.png.identifier as CFString, 1, nil) else {
    fail("can't write \(spec.out)")
  }
  // 144 dpi so viewers that honor it show the image at point size.
  CGImageDestinationAddImage(dest, image, [kCGImagePropertyDPIWidth: 72 * s, kCGImagePropertyDPIHeight: 72 * s] as CFDictionary)
  guard CGImageDestinationFinalize(dest) else { fail("can't write \(spec.out)") }
}

if let beside = spec.beside {
  let images = beside.map(load)
  let gap = (spec.gap ?? 24) * s
  let width = images.reduce(0) { $0 + $1.width } + Int(gap) * (images.count - 1)
  let height = images.map(\.height).max() ?? 0
  let ctx = context(width: width, height: height)
  var x = 0.0
  for image in images {
    ctx.draw(image, in: CGRect(x: x, y: Double(height - image.height), width: Double(image.width), height: Double(image.height)))
    x += Double(image.width) + gap
  }
  save(ctx.makeImage()!)
  exit(0)
}

// 1. The base window with overlays composited at their positions.
guard let basePath = spec.base else { fail("spec needs base or beside") }
let base = load(basePath)
let full = context(width: base.width, height: base.height)
let fullHeight = Double(base.height)
full.draw(base, in: CGRect(x: 0, y: 0, width: base.width, height: base.height))
for overlay in spec.overlays ?? [] {
  draw(
    load(overlay.path), in: full, x: overlay.x * s, y: overlay.y * s, height: fullHeight,
    radius: (overlay.radius ?? 0) * s, shadow: overlay.shadow ?? false, scale: s)
}
var image = full.makeImage()!

// 2. Crop.
if let crop = spec.crop {
  let rect = CGRect(x: crop.x * s, y: crop.y * s, width: crop.w * s, height: crop.h * s).integral
  guard let cropped = image.cropping(to: rect) else { fail("crop outside the image") }
  image = cropped
}

// 3. Rounded corners, hairline edge and shadow, on a transparent margin.
let radius = (spec.radius ?? 0) * s
let shadow = spec.shadow ?? false
let margin = shadow ? 56 * s : 0
let outCtx = context(width: image.width + Int(2 * margin), height: image.height + Int(2 * margin))
let outHeight = Double(image.height) + 2 * margin
draw(image, in: outCtx, x: margin, y: margin, height: outHeight, radius: radius, shadow: shadow, scale: s)
if spec.border ?? true {
  let rect = CGRect(x: margin, y: margin, width: Double(image.width), height: Double(image.height))
    .insetBy(dx: 0.5 * s, dy: 0.5 * s)
  outCtx.addPath(CGPath(roundedRect: rect, cornerWidth: max(0, radius - 0.5 * s), cornerHeight: max(0, radius - 0.5 * s), transform: nil))
  outCtx.setStrokeColor(CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.12))
  outCtx.setLineWidth(1 * s)
  outCtx.strokePath()
}
save(outCtx.makeImage()!)
