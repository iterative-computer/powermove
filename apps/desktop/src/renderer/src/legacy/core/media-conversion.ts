import { cloudSourcePaths } from './cloud-media';
import { MAX_SEQUENCE_FRAMES, animationTiming } from '../../../../shared/animated-image';
import { imageMimeType, isAnimatedImageExtension, mediaExtension } from '../../../../shared/media-formats';

/* Formats Chromium cannot put on screen by itself are converted in the main
   process and streamed back as an ordinary File, so everything downstream —
   decoding, persistence, playback, export — stays on one code path. */

const UPLOAD_CHUNK = 4 * 1024 * 1024;

/** Stream a finished conversion out of the main process and back into a File. */
export async function readProxyFile(result: any, name: string, lastModified: number): Promise<File> {
  const media = (window as any).powermove?.media;
  const parts: ArrayBuffer[] = [];
  try {
    for (let offset = 0; offset < result.size; offset += UPLOAD_CHUNK) {
      const chunk = await media.readPlaybackProxy(result.token, offset, Math.min(UPLOAD_CHUNK, result.size - offset));
      if (!chunk.byteLength) throw new Error('The converted file ended unexpectedly');
      const owned = new Uint8Array(chunk.byteLength);
      owned.set(chunk);
      parts.push(owned.buffer);
    }
    return new (window as any).File(parts, name, { type: result.type, lastModified: Number(lastModified) || Date.now() });
  } finally {
    await media.releasePlaybackProxy(result.token).catch(() => undefined);
  }
}

function decoderType(file: any): string {
  const declared = String(file?.type || '').toLowerCase();
  if (declared.startsWith('image/')) return declared;
  return imageMimeType(mediaExtension(file?.name));
}

/** Containers that can hold more than one frame. Only the decoder can say
    whether a particular file actually does. */
export function mayAnimate(file: any): boolean {
  if (typeof window === 'undefined' || !('ImageDecoder' in window)) return false;
  if (isAnimatedImageExtension(mediaExtension(file?.name))) return true;
  return /^image\/(?:gif|webp|avif|apng|png)$/.test(String(file?.type || '').toLowerCase());
}

/** Both APNG and animated WebP announce themselves in a header chunk that the
    format requires ahead of the pixel data, so a still never pays for a decoder
    or for reading itself into memory a second time. A marker pushed past this
    window only costs the animation its frames, the behaviour before any of
    this existed. */
const ANIMATION_HEADER_BYTES = 256 * 1024;
const HEADER_MARKERS: Record<string, string> = { 'image/png': 'acTL', 'image/webp': 'ANIM' };

async function declaresAnimation(file: any, type: string): Promise<boolean> {
  const marker = HEADER_MARKERS[type];
  if (!marker) return true;
  const head = new Uint8Array(await file.slice(0, ANIMATION_HEADER_BYTES).arrayBuffer());
  const wanted = [...marker].map(character => character.charCodeAt(0));
  outer: for (let at = 0; at + wanted.length <= head.length; at++) {
    for (let index = 0; index < wanted.length; index++) if (head[at + index] !== wanted[index]) continue outer;
    return true;
  }
  return false;
}

async function openDecoder(file: any) {
  const type = decoderType(file);
  const ImageDecoder = (window as any).ImageDecoder;
  if (!type || !ImageDecoder || !await ImageDecoder.isTypeSupported(type)) return null;
  if (!await declaresAnimation(file, type)) return null;
  const decoder = new ImageDecoder({ data: await file.arrayBuffer(), type });
  try {
    await decoder.tracks.ready;
    await decoder.completed;
    const track = decoder.tracks.selectedTrack;
    if (track && Number(track.frameCount) > 1) return { decoder, track };
  } catch (error) { /* A still, or bytes this build cannot parse. */ }
  try { decoder.close(); } catch (error) { }
  return null;
}

/** True when the file holds an animation rather than a single still. */
export async function isAnimatedImage(file: any): Promise<boolean> {
  if (!mayAnimate(file)) return false;
  let opened: any = null;
  try { opened = await openDecoder(file); } catch (error) { return false; }
  if (!opened) return false;
  try { opened.decoder.close(); } catch (error) { }
  return true;
}

export interface AnimationConversion { file: File; fps: number; frames: number; w: number; h: number }

/** Decode every frame with Chromium, then encode them through the same proxy
    converter numbered image sequences use. Each frame repeats for as long as
    its own delay lasts, so variable-delay animations keep their timing. */
export async function convertAnimatedImage(file: any, { onStage, onProgress }: any = {}): Promise<AnimationConversion> {
  const media = (window as any).powermove?.media;
  if (!media?.beginAnimation) throw new Error('Animated image import is unavailable');
  const opened = await openDecoder(file);
  if (!opened) throw new Error('Could not read the frames in this animated image');
  const { decoder, track } = opened;
  const count = Number(track.frameCount) || 0;
  if (count > MAX_SEQUENCE_FRAMES) {
    try { decoder.close(); } catch (error) { }
    throw new Error(`This animation has ${count} frames · Powermove imports up to ${MAX_SEQUENCE_FRAMES}`);
  }
  onStage?.('Reading animation frames');
  const durations: number[] = [];
  const encoded: Blob[] = [];
  let w = 0, h = 0, canvas: any = null, context: any = null;
  try {
    for (let index = 0; index < count; index++) {
      const { image } = await decoder.decode({ frameIndex: index });
      try {
        if (!canvas) {
          w = Number(image.displayWidth) || 0; h = Number(image.displayHeight) || 0;
          if (!(w > 0 && h > 0)) throw new Error('This animated image has no readable size');
          canvas = new (window as any).OffscreenCanvas(w, h);
          context = canvas.getContext('2d', { alpha: true });
          if (!context) throw new Error('Could not open a canvas to read this animation');
        }
        durations.push(Number(image.duration) || 0);
        // Decoded frames arrive composited, so each one paints a full canvas.
        context.clearRect(0, 0, w, h);
        context.drawImage(image, 0, 0, w, h);
      } finally { image.close(); }
      encoded.push(await canvas.convertToBlob({ type: 'image/png' }));
    }
  } finally {
    try { decoder.close(); } catch (error) { }
  }
  const timing = animationTiming(durations);
  onStage?.('Converting animation');
  const token = await media.beginAnimation(timing.fps, timing.repeats);
  let result: any;
  try {
    for (let index = 0; index < encoded.length; index++) {
      const bytes = new Uint8Array(await encoded[index]!.arrayBuffer());
      for (let offset = 0; offset < bytes.length; offset += UPLOAD_CHUNK) {
        await media.writeAnimationFrame(token, index, offset,
          bytes.subarray(offset, Math.min(offset + UPLOAD_CHUNK, bytes.length)));
      }
    }
    result = await media.finishAnimation(token, (completed: number) => {
      onStage?.(`Converting animation · ${Math.min(99, Math.round((completed / timing.frames) * 100))}%`);
      onProgress?.(completed, timing.frames);
    });
  } catch (error) {
    await media.releasePlaybackProxy(token).catch(() => undefined);
    throw error;
  }
  if (!result.ok) throw new Error(result.error);
  onStage?.('Loading animation');
  return {
    file: await readProxyFile(result, `${String(file.name || 'animation')}.webm`, file.lastModified),
    fps: timing.fps, frames: timing.frames, w, h,
  };
}

/** TIFF and HEIF stills become a PNG before the renderer ever sees them. */
export async function convertStillImage(file: any, { onStage }: any = {}): Promise<File> {
  const media = (window as any).powermove?.media;
  if (!media?.createStillImage) throw new Error('Image conversion is unavailable');
  onStage?.('Converting image');
  const result = await media.createStillImage(file, cloudSourcePaths.get(file));
  if (!result.ok) throw new Error(result.error);
  return readProxyFile(result, `${String(file.name || 'image')}.png`, file.lastModified);
}
