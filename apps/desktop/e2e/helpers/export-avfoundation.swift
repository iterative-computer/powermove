import AVFoundation
import AudioToolbox
import CoreMedia
import Foundation

func fail(_ message: String) -> Never {
  fputs("AVFoundation import: \(message)\n", stderr)
  exit(1)
}

guard CommandLine.arguments.count == 2 else { fail("expected one movie path") }
let asset = AVURLAsset(url: URL(fileURLWithPath: CommandLine.arguments[1]))
guard let video = asset.tracks(withMediaType: .video).first else { fail("no video track") }
guard let reader = try? AVAssetReader(asset: asset) else { fail("reader creation failed") }
let output = AVAssetReaderTrackOutput(track: video, outputSettings: [
  kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
])
reader.add(output)
guard reader.startReading() else { fail("reader start failed: \(String(describing: reader.error))") }
var frames = 0
var firstPTS: Double? = nil
while let sample = output.copyNextSampleBuffer() {
  if firstPTS == nil { firstPTS = CMSampleBufferGetPresentationTimeStamp(sample).seconds }
  guard CMSampleBufferGetImageBuffer(sample) != nil else { fail("frame \(frames) did not decode") }
  frames += 1
}
guard reader.status == .completed else { fail("reader stopped: \(String(describing: reader.error))") }
guard frames > 0, let firstPTS else { fail("no decoded frames") }

var audioSamples = 0
if let audio = asset.tracks(withMediaType: .audio).first {
  guard let audioReader = try? AVAssetReader(asset: asset) else { fail("audio reader creation failed") }
  let audioOutput = AVAssetReaderTrackOutput(track: audio, outputSettings: [
    AVFormatIDKey: kAudioFormatLinearPCM,
    AVLinearPCMIsFloatKey: true,
    AVLinearPCMBitDepthKey: 32,
    AVLinearPCMIsNonInterleaved: false,
  ])
  audioReader.add(audioOutput)
  guard audioReader.startReading() else { fail("audio reader start failed: \(String(describing: audioReader.error))") }
  while let sample = audioOutput.copyNextSampleBuffer() {
    audioSamples += CMSampleBufferGetNumSamples(sample)
  }
  guard audioReader.status == .completed else { fail("audio reader stopped: \(String(describing: audioReader.error))") }
  guard audioSamples > 0 else { fail("audio track did not decode") }
}

let timecodeTrack = asset.tracks(withMediaType: .timecode).first

let report: [String: Any] = [
  "readable": asset.isReadable,
  "playable": asset.isPlayable,
  "frames": frames,
  "audioSamples": audioSamples,
  "firstPTS": firstPTS,
  "duration": asset.duration.seconds,
  "videoDuration": video.timeRange.duration.seconds,
  "timecodeTrackStart": timecodeTrack?.timeRange.start.seconds as Any? ?? NSNull(),
  "timecodeTrackDuration": timecodeTrack?.timeRange.duration.seconds as Any? ?? NSNull(),
]
let data = try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])
print(String(data: data, encoding: .utf8)!)
