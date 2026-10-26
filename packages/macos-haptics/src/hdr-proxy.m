// powermove-hdr-proxy <source> <output.mp4>
//
// Tone maps HLG or PQ video to a Rec.709 HEVC playback proxy. The Metal kernel
// reproduces the FFmpeg proxy filter chain in media-proxy.ts to within one
// code value: zimg's linearization at 203 cd/m², a Möbius shoulder up to a
// 1000 cd/m² peak, and BT.1886 encoding. FFmpeg runs that math on the CPU at
// below real time for 4K; here the media engine decodes and encodes and the
// GPU converts, several times faster. Any failure exits non-zero and the
// caller falls back to FFmpeg.
#import <AVFoundation/AVFoundation.h>
#import <CoreVideo/CoreVideo.h>
#import <Metal/Metal.h>
#import <VideoToolbox/VideoToolbox.h>

static NSString *const kKernelSource = @R"METAL(
#include <metal_stdlib>
using namespace metal;

constant float CAP = 1.5f; // zimg's input limit for HDR transfers

static float3 toLinear(float3 e, uint transfer) {
  e = clamp(e, 0.0f, CAP);
  if (transfer == 0) {
    // HLG inverse OETF, then zimg's per-channel system gamma for 1000 cd/m².
    const float a = 0.17883277f, b = 0.28466892f, c = 0.55991073f;
    float3 scene = select((exp((e - c) / a) + b) / 12.0f, e * e / 3.0f, e <= 0.5f);
    return pow(scene, 1.2f) * (1000.0f / 203.0f);
  }
  const float m1 = 2610.0f / 16384.0f, m2 = 2523.0f / 4096.0f * 128.0f;
  const float c1 = 3424.0f / 4096.0f, c2 = 2413.0f / 4096.0f * 32.0f, c3 = 2392.0f / 4096.0f * 32.0f;
  float3 p = pow(e, 1.0f / m2);
  return pow(max(p - c1, 0.0f) / (c2 - c3 * p), 1.0f / m1) * (10000.0f / 203.0f);
}

static float mobius(float x) {
  const float j = 0.9f, peak = 4.926f;
  if (x <= j) return x;
  const float a = -j * j * (peak - 1.0f) / (j * j - 2.0f * j + peak);
  const float b = (j * j - 2.0f * j * peak + peak) / max(peak - 1.0f, 1e-6f);
  return (b * b + 2.0f * b * j + j * j) / (b - a) * (x + a) / (x + b);
}

// One HDR luma sample and its interpolated chroma to Rec.709 R'G'B'.
static float3 convert(float y, float2 c, uint transfer) {
  y = (y * (65535.0f / 64.0f) - 64.0f) / 876.0f;
  c = (c * (65535.0f / 64.0f) - 512.0f) / 896.0f;
  float r = y + 1.4746f * c.y, b = y + 1.8814f * c.x;
  float3 rgb = toLinear(float3(r, (y - 0.2627f * r - 0.0593f * b) / 0.6780f, b), transfer);
  rgb = float3x3(float3(1.6604910021f, -0.1245504745f, -0.0181507634f),
                 float3(-0.5876411388f, 1.1328998971f, -0.1005788980f),
                 float3(-0.0728498633f, -0.0083494226f, 1.1187296614f)) * rgb;
  float peak = max(max(max(rgb.r, rgb.g), rgb.b), 1e-6f);
  rgb *= mobius(peak) / peak;
  return pow(max(rgb, 0.0f), 1.0f / 2.4f);
}

static float lumaOf(float3 rgb) { return dot(rgb, float3(0.2126f, 0.7152f, 0.0722f)); }

kernel void tonemap(texture2d<float, access::read> srcY [[texture(0)]],
                    texture2d<float, access::sample> srcC [[texture(1)]],
                    texture2d<float, access::write> dstY [[texture(2)]],
                    texture2d<float, access::write> dstC [[texture(3)]],
                    constant uint &transfer [[buffer(0)]],
                    uint2 gid [[thread_position_in_grid]]) {
  if (gid.x >= dstC.get_width() || gid.y >= dstC.get_height()) return;
  constexpr sampler bilinear(coord::pixel, filter::linear, address::clamp_to_edge);
  const int w = int(dstY.get_width()), h = int(dstY.get_height());
  // 4:2:0 chroma sits on even columns, between rows. Output chroma uses the
  // matching bilinear weights: 1/4, 1/2, 1/4 across, 1/2, 1/2 down.
  float2 chroma = 0.0f;
  for (int dy = 0; dy < 2; dy++) {
    for (int dx = -1; dx < 2; dx++) {
      int2 p = clamp(int2(gid) * 2 + int2(dx, dy), int2(0), int2(w - 1, h - 1));
      float2 c = srcC.sample(bilinear, float2(p.x * 0.5f + 0.5f, p.y * 0.5f + 0.25f)).rg;
      float3 rgb = convert(srcY.read(uint2(p)).r, c, transfer);
      float luma = lumaOf(rgb);
      chroma += float2((rgb.b - luma) / 1.8556f, (rgb.r - luma) / 1.5748f) * (dx == 0 ? 0.25f : 0.125f);
      if (dx >= 0 && int(gid.x) * 2 + dx < w && int(gid.y) * 2 + dy < h)
        dstY.write(float4((16.0f + 219.0f * luma) / 255.0f), uint2(p));
    }
  }
  dstC.write(float4((128.0f + 224.0f * chroma) / 255.0f, 0.0f, 0.0f), gid);
}
)METAL";

static int fail(NSString *message) {
  fprintf(stderr, "%s\n", message.UTF8String);
  return 1;
}

static CVMetalTextureRef planeTexture(CVMetalTextureCacheRef cache, CVPixelBufferRef buffer, size_t plane, MTLPixelFormat format) {
  CVMetalTextureRef texture = NULL;
  CVMetalTextureCacheCreateTextureFromImage(kCFAllocatorDefault, cache, buffer, NULL, format,
    CVPixelBufferGetWidthOfPlane(buffer, plane), CVPixelBufferGetHeightOfPlane(buffer, plane), plane, &texture);
  return texture;
}

static void tagRec709(CVPixelBufferRef buffer) {
  CVBufferSetAttachment(buffer, kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_ITU_R_709_2, kCVAttachmentMode_ShouldPropagate);
  CVBufferSetAttachment(buffer, kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_ITU_R_709_2, kCVAttachmentMode_ShouldPropagate);
  CVBufferSetAttachment(buffer, kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_709_2, kCVAttachmentMode_ShouldPropagate);
}

int main(int argc, const char *argv[]) {
  @autoreleasepool {
    if (argc != 3) return fail(@"usage: powermove-hdr-proxy <source> <output.mp4>");
    NSURL *sourceURL = [NSURL fileURLWithPath:@(argv[1])], *outputURL = [NSURL fileURLWithPath:@(argv[2])];
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:sourceURL options:@{ AVURLAssetPreferPreciseDurationAndTimingKey: @YES }];
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    AVAssetTrack *video = [asset tracksWithMediaType:AVMediaTypeVideo].firstObject;
    AVAssetTrack *audio = [asset tracksWithMediaType:AVMediaTypeAudio].firstObject;
    NSArray *formats = video.formatDescriptions;
    CGAffineTransform transform = video.preferredTransform;
    float frameRate = video.nominalFrameRate;
#pragma clang diagnostic pop
    if (!video || formats.count == 0) return fail(@"The file has no video track");
    CMFormatDescriptionRef format = (__bridge CMFormatDescriptionRef)formats.firstObject;
    NSString *transferName = (__bridge NSString *)CMFormatDescriptionGetExtension(format, kCMFormatDescriptionExtension_TransferFunction);
    NSString *matrixName = (__bridge NSString *)CMFormatDescriptionGetExtension(format, kCMFormatDescriptionExtension_YCbCrMatrix);
    NSString *primariesName = (__bridge NSString *)CMFormatDescriptionGetExtension(format, kCMFormatDescriptionExtension_ColorPrimaries);
    uint32_t transfer;
    if ([transferName isEqual:(__bridge NSString *)kCVImageBufferTransferFunction_ITU_R_2100_HLG]) transfer = 0;
    else if ([transferName isEqual:(__bridge NSString *)kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ]) transfer = 1;
    else return fail(@"The video is not tagged HLG or PQ");
    if (![matrixName isEqual:(__bridge NSString *)kCVImageBufferYCbCrMatrix_ITU_R_2020]
        || ![primariesName isEqual:(__bridge NSString *)kCVImageBufferColorPrimaries_ITU_R_2020]) {
      return fail(@"The video is not tagged BT.2020");
    }

    NSError *error = nil;
    AVAssetReader *reader = [AVAssetReader assetReaderWithAsset:asset error:&error];
    if (!reader) return fail(error.localizedDescription);
    AVAssetReaderTrackOutput *videoOutput = [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:video outputSettings:@{
      (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange),
      (id)kCVPixelBufferMetalCompatibilityKey: @YES,
      (id)kCVPixelBufferIOSurfacePropertiesKey: @{},
    }];
    videoOutput.alwaysCopiesSampleData = NO;
    [reader addOutput:videoOutput];
    AVAssetReaderAudioMixOutput *audioOutput = nil;
    AudioChannelLayout stereo = { .mChannelLayoutTag = kAudioChannelLayoutTag_Stereo };
    NSData *stereoLayout = [NSData dataWithBytes:&stereo length:sizeof(stereo)];
    if (audio) {
      audioOutput = [AVAssetReaderAudioMixOutput assetReaderAudioMixOutputWithAudioTracks:@[audio] audioSettings:@{
        AVFormatIDKey: @(kAudioFormatLinearPCM), AVSampleRateKey: @48000, AVNumberOfChannelsKey: @2,
        AVChannelLayoutKey: stereoLayout, AVLinearPCMBitDepthKey: @32, AVLinearPCMIsFloatKey: @YES,
        AVLinearPCMIsNonInterleaved: @NO, AVLinearPCMIsBigEndianKey: @NO,
      }];
      [reader addOutput:audioOutput];
    }
    if (![reader startReading]) return fail(reader.error.localizedDescription ?: @"Could not read the video");

    CMSampleBufferRef first = [videoOutput copyNextSampleBuffer];
    CVPixelBufferRef firstFrame = first ? CMSampleBufferGetImageBuffer(first) : NULL;
    if (!firstFrame) return fail(reader.error.localizedDescription ?: @"The video has no frames");
    const size_t width = CVPixelBufferGetWidth(firstFrame), height = CVPixelBufferGetHeight(firstFrame);

    id<MTLDevice> device = MTLCreateSystemDefaultDevice();
    if (!device) return fail(@"No Metal device");
    id<MTLLibrary> library = [device newLibraryWithSource:kKernelSource options:nil error:&error];
    id<MTLFunction> function = [library newFunctionWithName:@"tonemap"];
    id<MTLComputePipelineState> pipeline = function ? [device newComputePipelineStateWithFunction:function error:&error] : nil;
    if (!pipeline) return fail(error.localizedDescription ?: @"Could not compile the tone map kernel");
    id<MTLCommandQueue> queue = [device newCommandQueue];
    CVMetalTextureCacheRef cache = NULL;
    CVMetalTextureCacheCreate(kCFAllocatorDefault, NULL, device, NULL, &cache);

    [NSFileManager.defaultManager removeItemAtURL:outputURL error:nil];
    AVAssetWriter *writer = [AVAssetWriter assetWriterWithURL:outputURL fileType:AVFileTypeMPEG4 error:&error];
    if (!writer) return fail(error.localizedDescription);
    NSDictionary *rec709 = @{
      AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
      AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
      AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
    };
    NSMutableDictionary *compression = [@{
      // Closer to the exact conversion than the VP9 proxy at a third of its size.
      AVVideoQualityKey: @0.65,
      // A keyframe every 15 frames bounds the decode for a random timeline seek.
      AVVideoMaxKeyFrameIntervalKey: @15,
      AVVideoAllowFrameReorderingKey: @NO,
      (id)kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality: @YES,
    } mutableCopy];
    if (frameRate > 0) compression[AVVideoExpectedSourceFrameRateKey] = @(frameRate);
    AVAssetWriterInput *videoInput = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:@{
      AVVideoCodecKey: AVVideoCodecTypeHEVC, AVVideoWidthKey: @(width), AVVideoHeightKey: @(height),
      AVVideoColorPropertiesKey: rec709, AVVideoCompressionPropertiesKey: compression,
    }];
    videoInput.expectsMediaDataInRealTime = NO;
    videoInput.transform = transform;
    AVAssetWriterInputPixelBufferAdaptor *adaptor = [AVAssetWriterInputPixelBufferAdaptor
      assetWriterInputPixelBufferAdaptorWithAssetWriterInput:videoInput sourcePixelBufferAttributes:@{
        (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange),
        (id)kCVPixelBufferWidthKey: @(width), (id)kCVPixelBufferHeightKey: @(height),
        (id)kCVPixelBufferMetalCompatibilityKey: @YES, (id)kCVPixelBufferIOSurfacePropertiesKey: @{},
      }];
    [writer addInput:videoInput];
    AVAssetWriterInput *audioInput = nil;
    if (audioOutput) {
      audioInput = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeAudio outputSettings:@{
        AVFormatIDKey: @(kAudioFormatMPEG4AAC), AVSampleRateKey: @48000, AVNumberOfChannelsKey: @2,
        AVChannelLayoutKey: stereoLayout, AVEncoderBitRateKey: @192000,
      }];
      audioInput.expectsMediaDataInRealTime = NO;
      [writer addInput:audioInput];
    }
    if (![writer startWriting]) return fail(writer.error.localizedDescription ?: @"Could not create the output video");
    [writer startSessionAtSourceTime:kCMTimeZero];

    __block CMSampleBufferRef pending = first;
    __block NSString *problem = nil;
    dispatch_group_t group = dispatch_group_create();
    dispatch_group_enter(group);
    dispatch_queue_t videoQueue = dispatch_queue_create("powermove.hdr.video", DISPATCH_QUEUE_SERIAL);
    [videoInput requestMediaDataWhenReadyOnQueue:videoQueue usingBlock:^{
      while (videoInput.isReadyForMoreMediaData) {
        CMSampleBufferRef sample = pending ?: [videoOutput copyNextSampleBuffer];
        pending = NULL;
        CVPixelBufferRef source = sample ? CMSampleBufferGetImageBuffer(sample) : NULL;
        if (!source) {
          if (sample) { CFRelease(sample); continue; }
          if (reader.status == AVAssetReaderStatusFailed) problem = reader.error.localizedDescription;
          [videoInput markAsFinished];
          dispatch_group_leave(group);
          return;
        }
        @autoreleasepool {
          CVPixelBufferRef target = NULL;
          CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, adaptor.pixelBufferPool, &target);
          CVMetalTextureRef textures[4] = {
            planeTexture(cache, source, 0, MTLPixelFormatR16Unorm), planeTexture(cache, source, 1, MTLPixelFormatRG16Unorm),
            target ? planeTexture(cache, target, 0, MTLPixelFormatR8Unorm) : NULL,
            target ? planeTexture(cache, target, 1, MTLPixelFormatRG8Unorm) : NULL,
          };
          BOOL ready = target && textures[0] && textures[1] && textures[2] && textures[3];
          if (ready) {
            id<MTLCommandBuffer> commands = [queue commandBuffer];
            id<MTLComputeCommandEncoder> encoder = [commands computeCommandEncoder];
            [encoder setComputePipelineState:pipeline];
            for (NSUInteger i = 0; i < 4; i++) [encoder setTexture:CVMetalTextureGetTexture(textures[i]) atIndex:i];
            [encoder setBytes:&transfer length:sizeof(transfer) atIndex:0];
            MTLSize grid = MTLSizeMake((width + 1) / 2, (height + 1) / 2, 1);
            [encoder dispatchThreads:grid threadsPerThreadgroup:MTLSizeMake(16, 16, 1)];
            [encoder endEncoding];
            [commands commit];
            [commands waitUntilCompleted];
            ready = commands.status == MTLCommandBufferStatusCompleted;
          }
          for (int i = 0; i < 4; i++) if (textures[i]) CFRelease(textures[i]);
          if (ready) {
            tagRec709(target);
            ready = [adaptor appendPixelBuffer:target withPresentationTime:CMSampleBufferGetPresentationTimeStamp(sample)];
          }
          if (target) CVPixelBufferRelease(target);
          CFRelease(sample);
          if (!ready) {
            problem = writer.error.localizedDescription ?: @"Could not convert a frame";
            [reader cancelReading];
            [videoInput markAsFinished];
            dispatch_group_leave(group);
            return;
          }
        }
      }
    }];
    if (audioInput) {
      dispatch_group_enter(group);
      dispatch_queue_t audioQueue = dispatch_queue_create("powermove.hdr.audio", DISPATCH_QUEUE_SERIAL);
      [audioInput requestMediaDataWhenReadyOnQueue:audioQueue usingBlock:^{
        while (audioInput.isReadyForMoreMediaData) {
          CMSampleBufferRef sample = [audioOutput copyNextSampleBuffer];
          if (!sample) { [audioInput markAsFinished]; dispatch_group_leave(group); return; }
          BOOL appended = [audioInput appendSampleBuffer:sample];
          CFRelease(sample);
          if (!appended) { [audioInput markAsFinished]; dispatch_group_leave(group); return; }
        }
      }];
    }
    dispatch_group_wait(group, DISPATCH_TIME_FOREVER);
    if (problem) { [writer cancelWriting]; return fail(problem); }
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    [writer finishWritingWithCompletionHandler:^{ dispatch_semaphore_signal(done); }];
    dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
    if (cache) CFRelease(cache);
    if (writer.status != AVAssetWriterStatusCompleted) return fail(writer.error.localizedDescription ?: @"Could not finish the output video");
    return 0;
  }
}
