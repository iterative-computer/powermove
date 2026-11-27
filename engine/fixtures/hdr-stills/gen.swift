// Synthetic HDR stills (engine/fixtures/README.md, "HDR stills"): a 64×36
// picture, left half 18 % grey in both renditions, right half 0.8 (linear,
// SDR base) and 3.0 (linear, HDR), written by Core Image and ImageIO.
// usage: xcrun swift gen.swift <out dir>
import CoreImage
import Foundation
import ImageIO
import UniformTypeIdentifiers

let w = 64, h = 36
let out = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)

func image(_ right: Float) -> CIImage {
    var px = [Float](repeating: 1, count: w * h * 4)
    for y in 0..<h {
        for x in 0..<w {
            let v: Float = x < w / 2 ? 0.18 : right
            let i = (y * w + x) * 4
            px[i] = v; px[i + 1] = v; px[i + 2] = v; px[i + 3] = 1
        }
    }
    let data = px.withUnsafeBufferPointer { Data(buffer: $0) }
    return CIImage(bitmapData: data, bytesPerRow: w * 16, size: CGSize(width: w, height: h),
                   format: .RGBAf, colorSpace: CGColorSpace(name: CGColorSpace.extendedLinearSRGB)!)
}

let ctx = CIContext()
let sdr = image(0.8)
let hdr = image(3.0)
let srgb = CGColorSpace(name: CGColorSpace.sRGB)!
func write(_ name: String, _ data: Data?) {
    guard let data else { print("FAILED \(name)"); return }
    try! data.write(to: out.appendingPathComponent(name))
    print("wrote \(name) \(data.count) bytes")
}
write("still-gainmap.heic", ctx.heifRepresentation(of: sdr, format: .RGBA8, colorSpace: srgb,
                                                    options: [.hdrImage: hdr]))
write("still-gainmap.jpg", ctx.jpegRepresentation(of: sdr, colorSpace: srgb,
                                                   options: [.hdrImage: hdr]))
write("still-hdr-pq.heic", try? ctx.heif10Representation(of: hdr,
    colorSpace: CGColorSpace(name: CGColorSpace.itur_2100_PQ)!, options: [:]))
write("still-hdr-hlg.heic", try? ctx.heif10Representation(of: hdr,
    colorSpace: CGColorSpace(name: CGColorSpace.itur_2100_HLG)!, options: [:]))

// AVIF with the HEIC's ISO gain map, through ImageIO's writer.
let heic = out.appendingPathComponent("still-gainmap.heic") as CFURL
if let src = CGImageSourceCreateWithURL(heic, nil) {
    let avif = out.appendingPathComponent("still-gainmap.avif") as CFURL
    if let dst = CGImageDestinationCreateWithURL(avif, "public.avif" as CFString, 1, nil) {
        CGImageDestinationAddImageFromSource(dst, src, 0, nil)
        for type in [kCGImageAuxiliaryDataTypeISOGainMap, kCGImageAuxiliaryDataTypeHDRGainMap] {
            if let aux = CGImageSourceCopyAuxiliaryDataInfoAtIndex(src, 0, type) {
                CGImageDestinationAddAuxiliaryDataInfo(dst, type, aux)
                print("avif: added \(type)")
            }
        }
        print("avif finalize \(CGImageDestinationFinalize(dst))")
    } else {
        print("FAILED avif destination")
    }
}
