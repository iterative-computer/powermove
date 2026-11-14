// Writes the HAP test files: uncompressed HAP (section types 0xAB, 0xAE,
// 0xAF) whose blocks have a known colour per block column, muxed by
// AVAssetWriter in pass-through. No FFmpeg build encodes HAP without Snappy.
//   swiftc -O make.swift -o /tmp/make-hap && /tmp/make-hap hap1|hap5|hapy out.mov
import AVFoundation
import CoreMedia

let flavour = CommandLine.arguments[1]
let out = URL(fileURLWithPath: CommandLine.arguments[2])
try? FileManager.default.removeItem(at: out)
let (w, h, frames) = (64, 32, 2)

func rgb565(_ r: Int, _ g: Int, _ b: Int) -> UInt16 { UInt16((r >> 3) << 11 | (g >> 2) << 5 | (b >> 3)) }
func le16(_ v: UInt16) -> [UInt8] { [UInt8(v & 0xFF), UInt8(v >> 8)] }

/// One 4x4 block: a flat colour (DXT1), with a flat alpha before it (DXT5).
/// HAP Q's blocks are scaled YCoCg: the colour carries Co, Cg and the scale,
/// the alpha carries Y.
func block(_ bx: Int, _ by: Int, _ n: Int) -> [UInt8] {
    let colour = flavour == "hapy"
        ? rgb565(96 + bx * 8, 160 - by * 16, 8)
        : rgb565(bx * 16, 255 - bx * 16, n * 60 + by * 30)
    let solid = le16(colour) + le16(colour) + [0, 0, 0, 0]
    switch flavour {
    case "hap1": return solid
    case "hap5": return [UInt8(bx * 16 + 15), UInt8(bx * 16 + 15), 0, 0, 0, 0, 0, 0] + solid
    default: return [UInt8(40 + bx * 12 + n * 20), UInt8(40 + bx * 12 + n * 20), 0, 0, 0, 0, 0, 0] + solid
    }
}

func frame(_ n: Int) -> Data {
    var texture = Data()
    for by in 0..<(h / 4) {
        for bx in 0..<(w / 4) { texture.append(contentsOf: block(bx, by, n)) }
    }
    let type: UInt8 = ["hap1": 0xAB, "hap5": 0xAE, "hapy": 0xAF][flavour]!
    var section = Data([UInt8(texture.count & 0xFF), UInt8((texture.count >> 8) & 0xFF), UInt8(texture.count >> 16), type])
    section.append(texture)
    return section
}

let fourcc: FourCharCode = ["hap1": 0x4861_7031, "hap5": 0x4861_7035, "hapy": 0x4861_7059][flavour]!
var format: CMVideoFormatDescription?
CMVideoFormatDescriptionCreate(allocator: nil, codecType: fourcc, width: Int32(w), height: Int32(h), extensions: nil, formatDescriptionOut: &format)
let writer = try AVAssetWriter(outputURL: out, fileType: .mov)
let input = AVAssetWriterInput(mediaType: .video, outputSettings: nil, sourceFormatHint: format)
writer.add(input)
writer.startWriting()
writer.startSession(atSourceTime: .zero)
for n in 0..<frames {
    let bytes = frame(n)
    var data: CMBlockBuffer?
    CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: bytes.count, blockAllocator: nil,
                                       customBlockSource: nil, offsetToData: 0, dataLength: bytes.count, flags: 0, blockBufferOut: &data)
    bytes.withUnsafeBytes { _ = CMBlockBufferReplaceDataBytes(with: $0.baseAddress!, blockBuffer: data!, offsetIntoDestination: 0, dataLength: bytes.count) }
    var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 24), presentationTimeStamp: CMTime(value: CMTimeValue(n), timescale: 24), decodeTimeStamp: .invalid)
    var size = bytes.count
    var sample: CMSampleBuffer?
    CMSampleBufferCreateReady(allocator: nil, dataBuffer: data, formatDescription: format, sampleCount: 1, sampleTimingEntryCount: 1,
                              sampleTimingArray: &timing, sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sample)
    while !input.isReadyForMoreMediaData { usleep(1000) }
    precondition(input.append(sample!), "append: \(String(describing: writer.error))")
}
input.markAsFinished()
let done = DispatchSemaphore(value: 0)
writer.finishWriting { done.signal() }
done.wait()
precondition(writer.status == .completed, "\(String(describing: writer.error))")
