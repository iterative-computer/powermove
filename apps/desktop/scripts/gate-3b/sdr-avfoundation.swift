// The SDR color bar's AVFoundation reference at full precision (plan section
// 2: "two references (FFmpeg with zimg, and AVFoundation)"), for the Phase 3b
// gate (docs/engine/phase3b-gate.md). `pm-export`'s bar test reads
// AVFoundation's decode as 8-bit BGRA, which cannot judge the 10-bit line
// (ΔE2000 maximum 0.5). This program reads frame 0 through AVAssetReader as
// 64-bit half float RGBA (the reader converts with the stream's matrix and
// range, as the BGRA path does; no color matching) and measures the chart's
// patch centers (24 × 24 means) against the analytic sRGB codes, CIELAB D65,
// CIEDE2000. Driven by sdr-avfoundation.mjs. Opens no window.
//
//   swift sdr-avfoundation.swift <movie> <width> <height> <cols> <rows> <codes.json>

import AVFoundation
import CoreVideo
import Foundation

func fail(_ message: String) -> Never {
  FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
  exit(1)
}

let args = CommandLine.arguments
if args.count != 7 { fail("usage: sdr-avfoundation.swift <movie> <width> <height> <cols> <rows> <codes.json>") }
let W = Int(args[2])!, H = Int(args[3])!, COLS = Int(args[4])!, ROWS = Int(args[5])!
let codes = try! JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: args[6]))) as! [[Int]]

// sRGB → CIELAB (D65), as pm-export's tests/bars_delivery.rs computes it.
func srgbToLab(_ rgb: [Double]) -> [Double] {
  let lin = rgb.map { $0 <= 0.04045 ? $0 / 12.92 : pow(($0 + 0.055) / 1.055, 2.4) }
  let x = 0.4124564 * lin[0] + 0.3575761 * lin[1] + 0.1804375 * lin[2]
  let y = 0.2126729 * lin[0] + 0.7151522 * lin[1] + 0.0721750 * lin[2]
  let z = 0.0193339 * lin[0] + 0.1191920 * lin[1] + 0.9503041 * lin[2]
  func f(_ t: Double) -> Double { t > pow(6.0 / 29.0, 3) ? cbrt(t) : t / (3 * pow(6.0 / 29.0, 2)) + 4.0 / 29.0 }
  let (fx, fy, fz) = (f(x / 0.95047), f(y / 1.0), f(z / 1.08883))
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

// Sharma's vectors: refuse to measure with a wrong formula.
for (a, b, want) in [
  ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
  ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
  ([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387], 1.2644),
  ([2.0776, 0.0795, -1.1350], [0.9033, -0.0636, -0.5514], 0.9082),
] {
  if abs(deltaE2000(a, b) - want) >= 1e-4 { fail("CIEDE2000 self-check failed") }
}

/// Frame 0 of the movie through AVAssetReader in `format`.
func readFrame(_ format: OSType) -> CVPixelBuffer {
  let asset = AVURLAsset(url: URL(fileURLWithPath: args[1]))
  let sem = DispatchSemaphore(value: 0)
  var track: AVAssetTrack?
  Task {
    track = try? await asset.loadTracks(withMediaType: .video).first
    sem.signal()
  }
  sem.wait()
  guard let track else { fail("no video track") }
  guard let reader = try? AVAssetReader(asset: asset) else { fail("no reader") }
  let output = AVAssetReaderTrackOutput(track: track, outputSettings: [kCVPixelBufferPixelFormatTypeKey as String: format])
  reader.add(output)
  if !reader.startReading() { fail("reader: \(String(describing: reader.error))") }
  guard let sample = output.copyNextSampleBuffer(), let pb = CMSampleBufferGetImageBuffer(sample) else { fail("no frame") }
  reader.cancelReading()
  return pb
}

func halfToFloat(_ h: UInt16) -> Double { Double(Float(Float16(bitPattern: h))) }

/// Mean RGB of the central 24 × 24 of each patch, row-major.
func patches(_ pb: CVPixelBuffer) -> [[Double]] {
  CVPixelBufferLockBaseAddress(pb, .readOnly)
  defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
  if CVPixelBufferGetWidth(pb) != W || CVPixelBufferGetHeight(pb) != H { fail("frame size") }
  let base = CVPixelBufferGetBaseAddress(pb)!, row = CVPixelBufferGetBytesPerRow(pb)
  let half = CVPixelBufferGetPixelFormatType(pb) == kCVPixelFormatType_64RGBAHalf
  let (pw, ph) = (W / COLS, H / ROWS)
  return (0..<(COLS * ROWS)).map { i in
    let (cx, cy) = ((i % COLS) * pw + pw / 2 - 12, (i / COLS) * ph + ph / 2 - 12)
    var acc = [0.0, 0.0, 0.0]
    for y in cy..<(cy + 24) {
      for x in cx..<(cx + 24) {
        if half {
          let p = (base + y * row).assumingMemoryBound(to: UInt16.self) + x * 4
          for k in 0..<3 { acc[k] += halfToFloat(p[k]) }
        } else {
          let p = (base + y * row).assumingMemoryBound(to: UInt8.self) + x * 4
          acc[0] += Double(p[2]) / 255; acc[1] += Double(p[1]) / 255; acc[2] += Double(p[0]) / 255
        }
      }
    }
    return acc.map { $0 / 576 }
  }
}

var result: [String: Any] = [:]
for (name, format) in [("half", kCVPixelFormatType_64RGBAHalf), ("bgra8", kCVPixelFormatType_32BGRA)] {
  let got = patches(readFrame(format))
  let de = zip(codes, got).map { (c, g) in deltaE2000(srgbToLab(c.map { Double($0) / 255 }), srgbToLab(g.map { min(max($0, 0), 1) })) }
  let worst = de.enumerated().max { $0.element < $1.element }!
  result[name] = ["max": de.max()!, "mean": de.reduce(0, +) / Double(de.count), "worstPatch": codes[worst.offset]]
}
let json = try! JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
print(String(data: json, encoding: .utf8)!)
