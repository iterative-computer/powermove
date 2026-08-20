import AppKit

let size = 1024
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let rect = NSRect(x: 0, y: 0, width: size, height: size)
NSColor.clear.setFill(); rect.fill()

let tile = NSBezierPath(roundedRect: NSRect(x: 44, y: 44, width: 936, height: 936), xRadius: 218, yRadius: 218)
NSColor(calibratedRed: 0.043, green: 0.043, blue: 0.050, alpha: 1).setFill(); tile.fill()

let inner = NSBezierPath(roundedRect: NSRect(x: 72, y: 72, width: 880, height: 880), xRadius: 192, yRadius: 192)
NSColor(calibratedRed: 0.068, green: 0.068, blue: 0.078, alpha: 1).setFill(); inner.fill()

let glow = NSShadow(); glow.shadowColor = NSColor(calibratedRed: 1, green: 0.32, blue: 0.04, alpha: 0.5); glow.shadowBlurRadius = 48; glow.shadowOffset = .zero
glow.set()
let mark = NSBezierPath()
mark.move(to: NSPoint(x: 284, y: 250))
mark.line(to: NSPoint(x: 284, y: 774))
mark.line(to: NSPoint(x: 505, y: 774))
mark.curve(to: NSPoint(x: 748, y: 548), controlPoint1: NSPoint(x: 654, y: 774), controlPoint2: NSPoint(x: 748, y: 685))
mark.curve(to: NSPoint(x: 515, y: 333), controlPoint1: NSPoint(x: 748, y: 412), controlPoint2: NSPoint(x: 655, y: 333))
mark.line(to: NSPoint(x: 446, y: 333))
mark.line(to: NSPoint(x: 446, y: 492))
mark.line(to: NSPoint(x: 515, y: 492))
mark.curve(to: NSPoint(x: 578, y: 551), controlPoint1: NSPoint(x: 554, y: 492), controlPoint2: NSPoint(x: 578, y: 515))
mark.curve(to: NSPoint(x: 510, y: 614), controlPoint1: NSPoint(x: 578, y: 589), controlPoint2: NSPoint(x: 553, y: 614))
mark.line(to: NSPoint(x: 446, y: 614))
mark.line(to: NSPoint(x: 446, y: 250))
mark.close()
NSColor(calibratedRed: 1, green: 0.37, blue: 0.055, alpha: 1).setFill(); mark.fill()

NSGraphicsContext.restoreGraphicsState()
let out = URL(fileURLWithPath: CommandLine.arguments[1])
try rep.representation(using: .png, properties: [:])!.write(to: out)
