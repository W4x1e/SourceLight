import AppKit
// Draws the macOS app icon for scripts/make-macos-icon.sh: the artwork in
// Apple's icon grid — a 1024 canvas, the shape 824 wide inset 100, corner
// radius ~185 — with a soft drop shadow and transparent corners.
// Usage: make-icon <artwork.png> <out-1024.png>
let src = NSImage(contentsOfFile: CommandLine.arguments[1])!
let size = 1024.0, inset = 100.0, box = size - 2 * inset, radius = 185.4
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let rect = NSRect(x: inset, y: inset, width: box, height: box)
let shape = NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius)
// Shadow, drawn under the shape.
NSGraphicsContext.saveGraphicsState()
let shadow = NSShadow(); shadow.shadowOffset = NSSize(width: 0, height: -10); shadow.shadowBlurRadius = 24
shadow.shadowColor = NSColor(white: 0, alpha: 0.35); shadow.set()
NSColor(red: 0.07, green: 0.07, blue: 0.08, alpha: 1).setFill(); shape.fill()
NSGraphicsContext.restoreGraphicsState()
// The artwork, clipped to the shape; its own dark ground fills it.
NSGraphicsContext.saveGraphicsState()
shape.addClip()
src.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1)
NSGraphicsContext.restoreGraphicsState()
// A hairline rim, as Apple's icons carry, so the dark shape holds its edge on a dark Dock.
NSColor(white: 1, alpha: 0.08).setStroke(); shape.lineWidth = 2; shape.stroke()
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: CommandLine.arguments[2]))
