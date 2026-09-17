/* Single source of truth for the media containers Powermove imports. Main and
   renderer both read these lists so a format is never accepted by the file
   picker but rejected by the importer, or vice versa. */

export function mediaExtension(name: unknown): string {
  const match = /\.([a-z0-9]+)$/i.exec(String(name || ''));
  return match ? match[1]!.toLowerCase() : '';
}

/** Raster stills Chromium decodes on its own. */
export const NATIVE_IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp', 'ico', 'svg'] as const;

/** Containers that may hold more than one frame. Chromium decodes every one of
    these through ImageDecoder, so animation detection never needs FFmpeg. */
export const ANIMATED_IMAGE_EXTENSIONS = ['gif', 'png', 'webp', 'avif'] as const;

/** Stills Chromium cannot decode. The main process converts them to PNG. */
export const CONVERTED_IMAGE_EXTENSIONS = ['tif', 'tiff', 'heic', 'heif'] as const;

/** Audio containers Chromium decodes. Providers often drop the MIME type, so
    the extension has to stand on its own. */
export const AUDIO_EXTENSIONS = ['wav', 'mp3', 'm4a', 'aac', 'ogg', 'oga', 'opus', 'weba', 'flac', 'aif', 'aiff'] as const;

/** Video containers Chromium plays without a conversion pass. */
export const NATIVE_VIDEO_EXTENSIONS = ['mp4', 'mov', 'm4v', 'webm'] as const;

/** Video containers only FFmpeg can open. These always import through a proxy. */
export const CONVERTED_VIDEO_EXTENSIONS = [
  'mkv', 'avi', 'wmv', 'asf', 'flv', 'mpg', 'mpeg', 'm2v', 'mts', 'm2ts', 'ts', '3gp', '3g2', 'mxf', 'ogv', 'dv', 'vob',
] as const;

const IMAGE = new Set<string>([...NATIVE_IMAGE_EXTENSIONS, ...CONVERTED_IMAGE_EXTENSIONS]);
const ANIMATED = new Set<string>(ANIMATED_IMAGE_EXTENSIONS);
const CONVERTED_IMAGE = new Set<string>(CONVERTED_IMAGE_EXTENSIONS);
const NATIVE_VIDEO = new Set<string>(NATIVE_VIDEO_EXTENSIONS);
const VIDEO = new Set<string>([...NATIVE_VIDEO_EXTENSIONS, ...CONVERTED_VIDEO_EXTENSIONS]);

export function isImageExtension(extension: string): boolean { return IMAGE.has(extension); }
export function isVideoExtension(extension: string): boolean { return VIDEO.has(extension); }
/** True when the file may carry an animation worth decoding frame by frame. */
export function isAnimatedImageExtension(extension: string): boolean { return ANIMATED.has(extension); }
/** True when the still needs a main-process conversion before Chromium sees it. */
export function needsImageConversion(extension: string): boolean { return CONVERTED_IMAGE.has(extension); }
/** True when the container has no chance of opening in a <video> element. */
export function needsVideoProxy(extension: string): boolean { return VIDEO.has(extension) && !NATIVE_VIDEO.has(extension); }
/** MOV/MP4 can hold codecs Chromium refuses, so they may still fall back to a proxy. */
export function mayNeedVideoProxy(extension: string): boolean { return VIDEO.has(extension) && extension !== 'webm'; }

const MIME: Record<string, string> = {
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp',
  avif: 'image/avif', bmp: 'image/bmp', ico: 'image/x-icon', svg: 'image/svg+xml',
  tif: 'image/tiff', tiff: 'image/tiff', heic: 'image/heic', heif: 'image/heif',
};
/** Drag-and-drop and paste providers routinely omit a File's type. */
export function imageMimeType(extension: string): string { return MIME[extension] || ''; }

/** Chromium's `image/*` and `video/*` pickers only offer what it decodes itself,
    so every converted container has to be named explicitly. */
export const MEDIA_ACCEPT = [
  'image/*', 'video/*', 'audio/*',
  ...[...NATIVE_IMAGE_EXTENSIONS, ...CONVERTED_IMAGE_EXTENSIONS,
    ...NATIVE_VIDEO_EXTENSIONS, ...CONVERTED_VIDEO_EXTENSIONS,
    ...AUDIO_EXTENSIONS].map(extension => `.${extension}`),
  '.obj', '.pmv',
].join(',');
