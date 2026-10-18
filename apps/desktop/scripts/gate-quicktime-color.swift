// Section 2 color invariant (3), Standard color, for the Phase 2 gate
// (docs/engine/phase2-gate.md): an export using the "Rec.709 (QuickTime and
// Safari)" output, played through AVPlayerItemVideoOutput into the display
// profile, matches the viewer's ColorSync conversion within ΔE2000 1.0 on a
// test chart.
//
// Driven by gate-quicktime-color.mjs, which renders the chart through
// `pm-cli render` (the job engine's export pipeline). This program measures one
// movie:
//
//   swift scripts/gate-quicktime-color.swift <movie> <width> <height> <cols> <rows> <codes.json>
//
// <codes.json> is the chart's 8-bit sRGB codes, patch by patch in row-major
// order. Viewer path: the codes, as sRGB, converted by ColorSync to the
// display profile (what Chromium's sRGB canvas gets from Core Animation).
// Video path: AVPlayer + AVPlayerItemVideoOutput's frame at 0.5 s (16-bit
// half float and 8-bit BGRA outputs), the patch-center means converted from
// the color space AVFoundation attaches to the frame to the display profile.
// Both display values go to extended linear sRGB, CIELAB (D65), CIEDE2000.
// Done for the main display's profile and for Display P3. Prints JSON.
// Opens no window.

import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation

func fail(_ message: String) -> Never {
  FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
  exit(1)
}

// MARK: Color math (as engine/spikes/s2-precision/src/color.rs)

func lin709ToXYZ(_ c: [Double]) -> [Double] {
  [
    0.4123907992659595 * c[0] + 0.357584339383878 * c[1] + 0.1804807884018343 * c[2],
    0.21263900587151036 * c[0] + 0.715168678767756 * c[1] + 0.07219231536073371 * c[2],
    0.01933081871559185 * c[0] + 0.11919477979462599 * c[1] + 0.9505321522496606 * c[2],
  ]
}

func xyzToLab(_ xyz: [Double], _ white: [Double]) -> [Double] {
  func f(_ t: Double) -> Double {
    let d = 6.0 / 29.0
    return t > d * d * d ? cbrt(t) : t / (3 * d * d) + 4.0 / 29.0
  }
  let fx = f(xyz[0] / white[0]), fy = f(xyz[1] / white[1]), fz = f(xyz[2] / white[2])
  return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)]
}

/// CIEDE2000 (Sharma, Wu, Dalal 2005), kL = kC = kH = 1.
func deltaE2000(_ lab1: [Double], _ lab2: [Double]) -> Double {
  let (l1, a1, b1) = (lab1[0], lab1[1], lab1[2])
  let (l2, a2, b2) = (lab2[0], lab2[1], lab2[2])
  let c1 = (a1 * a1 + b1 * b1).squareRoot(), c2 = (a2 * a2 + b2 * b2).squareRoot()
  let cbar7 = pow((c1 + c2) / 2, 7)
  let g = 0.5 * (1 - (cbar7 / (cbar7 + pow(25, 7))).squareRoot())
  let a1p = (1 + g) * a1, a2p = (1 + g) * a2
  let c1p = (a1p * a1p + b1 * b1).squareRoot(), c2p = (a2p * a2p + b2 * b2).squareRoot()
  func hp(_ b: Double, _ ap: Double) -> Double {
    if b == 0 && ap == 0 { return 0 }
    let h = atan2(b, ap) * 180 / .pi
    return h < 0 ? h + 360 : h
  }
  let h1p = hp(b1, a1p), h2p = hp(b2, a2p)
  let dlp = l2 - l1, dcp = c2p - c1p
  var dhp = 0.0
  if c1p * c2p != 0 {
    let d = h2p - h1p
    dhp = abs(d) <= 180 ? d : (d > 180 ? d - 360 : d + 360)
  }
  let dhpBig = 2 * (c1p * c2p).squareRoot() * sin(dhp * .pi / 360)
  let lbarp = (l1 + l2) / 2, cbarp = (c1p + c2p) / 2
  var hbarp: Double
  if c1p * c2p == 0 { hbarp = h1p + h2p }
  else if abs(h1p - h2p) <= 180 { hbarp = (h1p + h2p) / 2 }
  else if h1p + h2p < 360 { hbarp = (h1p + h2p + 360) / 2 }
  else { hbarp = (h1p + h2p - 360) / 2 }
  let t = 1 - 0.17 * cos((hbarp - 30) * .pi / 180) + 0.24 * cos(2 * hbarp * .pi / 180)
    + 0.32 * cos((3 * hbarp + 6) * .pi / 180) - 0.20 * cos((4 * hbarp - 63) * .pi / 180)
  let dtheta = 30 * exp(-pow((hbarp - 275) / 25, 2))
  let cbarp7 = pow(cbarp, 7)
  let rc = 2 * (cbarp7 / (cbarp7 + pow(25, 7))).squareRoot()
  let lb = pow(lbarp - 50, 2)
  let sl = 1 + 0.015 * lb / (20 + lb).squareRoot()
  let sc = 1 + 0.045 * cbarp
  let sh = 1 + 0.015 * cbarp * t
  let rt = -sin(2 * dtheta * .pi / 180) * rc
  return (pow(dlp / sl, 2) + pow(dcp / sc, 2) + pow(dhpBig / sh, 2) + rt * (dcp / sc) * (dhpBig / sh)).squareRoot()
}

// Sharma's vectors: the measurement refuses to run on a wrong formula.
for (a, b, want) in [
  ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
  ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
  ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
  ([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387], 1.2644),
  ([90.8027, -2.0831, 1.4410], [91.1528, -1.6435, 0.0447], 1.4441),
  ([2.0776, 0.0795, -1.1350], [0.9033, -0.0636, -0.5514], 0.9082),
] {
  let got = deltaE2000(a, b)
  if abs(got - want) >= 1e-4 { fail("CIEDE2000 self-check failed: \(got) vs \(want)") }
}

func labFromLinearSRGB(_ c: [Float]) -> [Double] {
  xyzToLab(lin709ToXYZ(c.map(Double.init)), lin709ToXYZ([1, 1, 1]))
}

/// ColorSync conversion of RGB triplets (α = 1) between color spaces, one
/// `CGColor` per patch (`CGColor.converted`, the default intent).
func convert(_ src: [[Float]], from: CGColorSpace, to: CGColorSpace) -> [[Float]] {
  src.map { c in
    let comps: [CGFloat] = [CGFloat(c[0]), CGFloat(c[1]), CGFloat(c[2]), 1]
    guard let color = CGColor(colorSpace: from, components: comps),
          let out = color.converted(to: to, intent: .defaultIntent, options: nil),
          let o = out.components, o.count >= 3 else { fail("ColorSync conversion failed") }
    return [Float(o[0]), Float(o[1]), Float(o[2])]
  }
}

// MARK: Arguments

let args = CommandLine.arguments
guard args.count == 7, let W = Int(args[2]), let H = Int(args[3]), let COLS = Int(args[4]), let ROWS = Int(args[5]) else {
  fail("usage: gate-quicktime-color.swift <movie> <width> <height> <cols> <rows> <codes.json>")
}
let movie = URL(fileURLWithPath: args[1])
guard let codes = try? JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: args[6]))) as? [[Int]], codes.count == COLS * ROWS else {
  fail("codes.json must list \(COLS * ROWS) [r, g, b] codes")
}

// MARK: AVPlayer frame

func spin(_ seconds: Double) { RunLoop.current.run(until: Date(timeIntervalSinceNow: seconds)) }

func playerFrame(_ format: OSType, at seconds: Double) -> (CVPixelBuffer, Double) {
  let item = AVPlayerItem(url: movie)
  let output = AVPlayerItemVideoOutput(pixelBufferAttributes: [
    kCVPixelBufferPixelFormatTypeKey as String: format,
    kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any](),
  ])
  item.add(output)
  let player = AVPlayer(playerItem: item)
  let start = Date()
  while item.status != .readyToPlay {
    if item.status == .failed { fail("AVPlayerItem failed: \(String(describing: item.error))") }
    if Date().timeIntervalSince(start) > 20 { fail("AVPlayerItem never became ready") }
    spin(0.01)
  }
  let t = CMTime(seconds: seconds, preferredTimescale: 600)
  player.seek(to: t, toleranceBefore: .zero, toleranceAfter: .zero)
  let pulled = Date()
  var last: (CVPixelBuffer, Double)?
  while Date().timeIntervalSince(pulled) < 20 {
    if output.hasNewPixelBuffer(forItemTime: t) {
      var shown = CMTime.invalid
      if let pb = output.copyPixelBuffer(forItemTime: t, itemTimeForDisplay: &shown) {
        last = (pb, shown.seconds)
        if abs(shown.seconds - seconds) < 0.03 { return (pb, shown.seconds) }
      }
    }
    spin(0.01)
  }
  guard let l = last else { fail("AVPlayerItemVideoOutput produced no frame") }
  return l
}

func halfToFloat(_ h: UInt16) -> Float { Float(Float16(bitPattern: h)) }

/// Mean RGB of the central 32 × 32 of each patch.
func samplePatches(_ pb: CVPixelBuffer) -> [[Float]] {
  CVPixelBufferLockBaseAddress(pb, .readOnly)
  defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
  let w = CVPixelBufferGetWidth(pb), h = CVPixelBufferGetHeight(pb)
  if w != W || h != H { fail("frame is \(w) × \(h), expected \(W) × \(H)") }
  let base = CVPixelBufferGetBaseAddress(pb)!
  let row = CVPixelBufferGetBytesPerRow(pb)
  let half = CVPixelBufferGetPixelFormatType(pb) == kCVPixelFormatType_64RGBAHalf
  let (pw, ph) = (W / COLS, H / ROWS)
  var out: [[Float]] = []
  for i in 0..<(COLS * ROWS) {
    let cx = (i % COLS) * pw + pw / 2, cy = (i / COLS) * ph + ph / 2
    var acc = [Double](repeating: 0, count: 3)
    var n = 0.0
    for y in (cy - 16)..<(cy + 16) {
      for x in (cx - 16)..<(cx + 16) {
        if half {
          let p = (base + y * row).assumingMemoryBound(to: UInt16.self) + x * 4
          for k in 0..<3 { acc[k] += Double(halfToFloat(p[k])) }
        } else {
          let p = (base + y * row).assumingMemoryBound(to: UInt8.self) + x * 4
          acc[0] += Double(p[2]) / 255; acc[1] += Double(p[1]) / 255; acc[2] += Double(p[0]) / 255
        }
        n += 1
      }
    }
    out.append(acc.map { Float($0 / n) })
  }
  return out
}

func frameColorSpace(_ pb: CVPixelBuffer) -> CGColorSpace? {
  if let v = CVBufferCopyAttachment(pb, kCVImageBufferCGColorSpaceKey, nil) { return (v as! CGColorSpace) }
  guard let att = CVBufferCopyAttachments(pb, .shouldPropagate) else { return nil }
  return CVImageBufferCreateColorSpaceFromAttachments(att)?.takeRetainedValue()
}

func describe(_ v: Any?) -> String { v.map { "\($0)" } ?? "none" }

// MARK: Measure

let srgb = CGColorSpace(name: CGColorSpace.sRGB)!
let lin = CGColorSpace(name: CGColorSpace.extendedLinearSRGB)!
let displays: [(String, CGColorSpace)] = [
  ("main display profile", CGDisplayCopyColorSpace(CGMainDisplayID())),
  ("Display P3", CGColorSpace(name: CGColorSpace.displayP3)!),
]
let viewerCodes: [[Float]] = codes.map { $0.map { Float($0) / 255 } }

var cases: [[String: Any]] = []
for (formatName, format) in [("RGhA (64RGBAHalf)", kCVPixelFormatType_64RGBAHalf), ("BGRA", kCVPixelFormatType_32BGRA)] {
  let (frame, shown) = playerFrame(format, at: 0.5)
  guard let vcs = frameColorSpace(frame) else { fail("the frame has no color space") }
  let rgb = samplePatches(frame)
  let attachments = CVBufferCopyAttachments(frame, .shouldPropagate) as? [String: Any] ?? [:]
  for (displayName, dcs) in displays {
    let viewer = convert(convert(viewerCodes, from: srgb, to: dcs), from: dcs, to: lin)
    let video = convert(convert(rgb, from: vcs, to: dcs), from: dcs, to: lin)
    var patches: [[String: Any]] = []
    var worst = (0.0, -1)
    var sum = 0.0
    for i in 0..<codes.count {
      let de = deltaE2000(labFromLinearSRGB(viewer[i]), labFromLinearSRGB(video[i]))
      sum += de
      if de > worst.0 { worst = (de, i) }
      patches.append(["srgb": codes[i], "video": rgb[i], "deltaE": de])
    }
    cases.append([
      "output": formatName, "display": displayName, "frameTime": shown,
      "frameColorSpace": describe(vcs.name),
      "primaries": describe(attachments[kCVImageBufferColorPrimariesKey as String]),
      "transfer": describe(attachments[kCVImageBufferTransferFunctionKey as String]),
      "matrix": describe(attachments[kCVImageBufferYCbCrMatrixKey as String]),
      "maxDeltaE": worst.0, "meanDeltaE": sum / Double(codes.count),
      "worstPatch": worst.1 >= 0 ? codes[worst.1] : [], "passMax1": worst.0 <= 1.0,
      "patches": patches,
    ])
  }
}

let report: [String: Any] = [
  "movie": movie.path,
  "displayProfile": describe(CGDisplayCopyColorSpace(CGMainDisplayID()).name),
  "cases": cases,
]
let data = try! JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
FileHandle.standardOutput.write(data)
FileHandle.standardOutput.write("\n".data(using: .utf8)!)
