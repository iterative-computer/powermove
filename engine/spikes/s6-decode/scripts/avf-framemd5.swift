// AVFoundation reference for spike S6: decodes the first video track with
// AVAssetReader (edit lists applied the way QuickTime presents them) into the
// requested CoreVideo pixel format and prints one line per frame:
//
//   <pts value> <pts timescale> <md5 of the planes packed without row padding>
//
// The hash layout matches the spike's own framemd5 for the same pixel format,
// so the two can be compared frame by frame, aligned by presentation time.
//
// Usage: avf-framemd5 FILE FOURCC   (e.g. 420v, x420, x422, y416, v0a8)

import AVFoundation
import CoreVideo
import CryptoKit
import Foundation

func fourcc(_ s: String) -> OSType {
    s.utf8.reduce(0) { ($0 << 8) | OSType($1) }
}

let args = CommandLine.arguments
guard args.count == 3, args[2].utf8.count == 4 else {
    FileHandle.standardError.write("usage: avf-framemd5 FILE FOURCC\n".data(using: .utf8)!)
    exit(2)
}
let asset = AVURLAsset(url: URL(fileURLWithPath: args[1]))
let format = fourcc(args[2])

let sem = DispatchSemaphore(value: 0)
var status: Int32 = 0
Task {
    defer { sem.signal() }
    do {
        guard let track = try await asset.loadTracks(withMediaType: .video).first else {
            FileHandle.standardError.write("no video track\n".data(using: .utf8)!)
            status = 1
            return
        }
        let reader = try AVAssetReader(asset: asset)
        let out = AVAssetReaderTrackOutput(
            track: track,
            outputSettings: [kCVPixelBufferPixelFormatTypeKey as String: format])
        out.alwaysCopiesSampleData = false
        reader.add(out)
        guard reader.startReading() else {
            FileHandle.standardError.write("startReading: \(String(describing: reader.error))\n".data(using: .utf8)!)
            status = 1
            return
        }
        var lines = ""
        while let sb = out.copyNextSampleBuffer() {
            guard let pb = CMSampleBufferGetImageBuffer(sb) else { continue }
            let pts = CMSampleBufferGetPresentationTimeStamp(sb)
            let got = CVPixelBufferGetPixelFormatType(pb)
            if got != format {
                FileHandle.standardError.write("got pixel format \(got), wanted \(format)\n".data(using: .utf8)!)
                status = 1
                return
            }
            CVPixelBufferLockBaseAddress(pb, .readOnly)
            var md5 = Insecure.MD5()
            if CVPixelBufferIsPlanar(pb) {
                for p in 0..<CVPixelBufferGetPlaneCount(pb) {
                    let base = CVPixelBufferGetBaseAddressOfPlane(pb, p)!
                    let stride = CVPixelBufferGetBytesPerRowOfPlane(pb, p)
                    let h = CVPixelBufferGetHeightOfPlane(pb, p)
                    let w = CVPixelBufferGetWidthOfPlane(pb, p)
                    // Bytes per sample of each plane for the formats the spike uses.
                    let bpp: Int
                    switch (format, p) {
                    case (fourcc("420v"), 0), (fourcc("420f"), 0), (fourcc("v0a8"), 0), (fourcc("v0a8"), 2): bpp = 1
                    case (fourcc("420v"), 1), (fourcc("420f"), 1), (fourcc("v0a8"), 1): bpp = 2
                    case (_, 0): bpp = 2
                    default: bpp = 4
                    }
                    for y in 0..<h {
                        md5.update(bufferPointer: UnsafeRawBufferPointer(start: base + y * stride, count: w * bpp))
                    }
                }
            } else {
                let base = CVPixelBufferGetBaseAddress(pb)!
                let stride = CVPixelBufferGetBytesPerRow(pb)
                let w = CVPixelBufferGetWidth(pb)
                let h = CVPixelBufferGetHeight(pb)
                let bpp = format == fourcc("y416") ? 8 : 4
                for y in 0..<h {
                    md5.update(bufferPointer: UnsafeRawBufferPointer(start: base + y * stride, count: w * bpp))
                }
            }
            CVPixelBufferUnlockBaseAddress(pb, .readOnly)
            let hex = md5.finalize().map { String(format: "%02x", $0) }.joined()
            lines += "\(pts.value) \(pts.timescale) \(hex)\n"
        }
        if reader.status == .failed {
            FileHandle.standardError.write("reader failed: \(String(describing: reader.error))\n".data(using: .utf8)!)
            status = 1
            return
        }
        FileHandle.standardOutput.write(lines.data(using: .utf8)!)
    } catch {
        FileHandle.standardError.write("\(error)\n".data(using: .utf8)!)
        status = 1
    }
}
sem.wait()
exit(status)
