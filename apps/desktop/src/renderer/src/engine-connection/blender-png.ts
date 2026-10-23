/* Exact converted image inputs for Blender. The old renderer's assetBytes
 * (core/scene3d/rendering.ts) draws PM.assets.get(id)?.el, or a newly
 * decoded ImageBitmap, at its intrinsic size into Canvas2D and calls
 * toBlob('image/png'). The original content hash guards against a window
 * answering an export for a different revision of the asset. */
import type { CompatFrameRequest } from '../../../shared/engine-protocol';
import { sha256Hex } from './svg-rasters';

export interface BlenderPngAsset {
  kind: string;
  name: string;
}

export interface BlenderPngHost {
  meta(id: string): BlenderPngAsset | null;
  blob(meta: BlenderPngAsset): Promise<Blob | null>;
  image(id: string): { el?: CanvasImageSource | null } | null;
  canvas(): HTMLCanvasElement;
  decode(blob: Blob): Promise<ImageBitmap>;
}

export type BlenderPngAnswer = {
  key: string;
  width: number;
  height: number;
  format: 'png';
  origin: [number, number];
  scale: number;
  data: Uint8Array;
};

function hex(bytes: Uint8Array): string {
  return Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
}

const DEFAULT_CACHE_BYTES = 256 * 1024 * 1024;
type CacheEntry = { promise: Promise<BlenderPngAnswer>; bytes: number | null };

/** Share in-flight work and retain successful PNGs by original SHA-256. */
export class BlenderPngs {
  private readonly cache = new Map<string, CacheEntry>();
  private retainedBytes = 0;

  constructor(private readonly host: BlenderPngHost, private readonly cacheBytes = DEFAULT_CACHE_BYTES) {}

  async produce(request: CompatFrameRequest): Promise<BlenderPngAnswer> {
    const asked = request.blender;
    if (request.source !== 'blenderPng' || !asked) throw new Error('The Blender PNG ask names no source image.');
    const meta = this.host.meta(asked.asset);
    if (!meta || meta.kind !== 'image') throw new Error(`Missing 3D image: ${asked.asset}`);
    const blob = await this.host.blob(meta);
    if (!blob) throw new Error(`Missing media for ${meta.name}`);
    const content = hex(asked.content);
    const bytes = new Uint8Array(await blob.arrayBuffer());
    if (await sha256Hex(bytes) !== content) throw new Error(`The image ${meta.name} changed while Blender was preparing its frame.`);
    const cached = this.cache.get(content);
    if (cached) {
      this.cache.delete(content);
      this.cache.set(content, cached);
      return cached.promise;
    }
    const converted = this.convert(asked.asset, blob, content);
    const entry: CacheEntry = { promise: converted, bytes: null };
    entry.promise = converted.then(answer => {
      if (this.cache.get(content) === entry) {
        const size = answer.data.byteLength;
        if (size > this.cacheBytes) this.cache.delete(content);
        else {
          entry.bytes = size;
          this.retainedBytes += size;
          this.evict();
        }
      }
      return answer;
    }, error => {
      if (this.cache.get(content) === entry) this.cache.delete(content);
      throw error;
    });
    this.cache.set(content, entry);
    return entry.promise;
  }

  private evict(): void {
    while (this.retainedBytes > this.cacheBytes) {
      let removed = false;
      for (const [key, entry] of this.cache) {
        if (entry.bytes === null) continue;
        this.cache.delete(key);
        this.retainedBytes -= entry.bytes;
        removed = true;
        break;
      }
      if (!removed) break;
    }
  }

  private async convert(id: string, blob: Blob, content: string): Promise<BlenderPngAnswer> {
    const current = this.host.image(id);
    const temporary = !current?.el;
    const image = current?.el || await this.host.decode(blob);
    try {
      const dims = image as CanvasImageSource & { width?: number; height?: number; naturalWidth?: number; naturalHeight?: number };
      const canvas = this.host.canvas();
      canvas.width = dims.width || dims.naturalWidth || 0;
      canvas.height = dims.height || dims.naturalHeight || 0;
      const context = canvas.getContext('2d');
      if (!context || !canvas.width || !canvas.height) throw new Error('Could not convert texture');
      context.drawImage(image, 0, 0);
      const png = await new Promise<Blob>((resolve, reject) => canvas.toBlob(
        value => value ? resolve(value) : reject(new Error('Could not convert texture')), 'image/png'));
      const data = new Uint8Array(await png.arrayBuffer());
      return { key: content, width: canvas.width, height: canvas.height, format: 'png', origin: [0, 0], scale: 1, data };
    } finally {
      if (temporary) (image as ImageBitmap).close?.();
    }
  }
}
