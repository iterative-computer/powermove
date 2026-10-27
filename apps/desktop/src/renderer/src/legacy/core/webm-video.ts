/* The current exporter's WebCodecs VideoEncoder settings for WebM: VP9
   profile 0 level 3.1 8-bit, else VP8; the
   quality tier's bit rate; `latencyMode: 'quality'`; microsecond timestamps
   rounded from the frame index; a forced keyframe every rounded two seconds;
   at most a dozen frames queued before the producer waits. */

export const WEBM_VIDEO_CODECS = [
  { codec: 'vp09.00.31.08', id: 'V_VP9' },
  { codec: 'vp8', id: 'V_VP8' },
] as const;

export type WebmVideoCodec = (typeof WEBM_VIDEO_CODECS)[number];

export const WEBM_ENCODE_QUEUE_LIMIT = 12;

export function webmVideoConfig(codec: string, width: number, height: number, bitrate: number, fps: number) {
  return { codec, width, height, bitrate, framerate: fps, latencyMode: 'quality' as const };
}

export function webmFrameTiming(index: number, fps: number): { timestamp: number; duration: number } {
  return { timestamp: Math.round(index / fps * 1e6), duration: Math.round(1e6 / fps) };
}

export function webmKeyFrame(index: number, fps: number): boolean {
  return index % Math.round(fps * 2) === 0;
}
