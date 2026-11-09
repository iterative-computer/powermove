import type { StudioScene } from './extract';

export const STUDIO_SESSION_BYTES = 2 * 1024 ** 3;
export const STUDIO_SCENE_BYTES = 512 * 1024 ** 2;
export const STUDIO_CANVAS_BYTES = 128 * 1024 ** 2;

/** Includes temporal/Final guides, readback, photon grid and driver headroom.
 * Native and portable output limits agree; reject before creating buffers. */
export function studioTargetBytes(width: number, height: number, maxBinding = Infinity): number {
  if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1 || width > 4096 || height > 4096 || width * height > 8_847_360)
    throw new Error('Studio output dimensions exceed the memory budget; reduce the output size');
  if (width * height * 32 > maxBinding)
    throw new Error('Studio output exceeds this GPU buffer budget; reduce the output size');
  const bytes = width * height * 320 + 16 * 1024 ** 2;
  if (bytes > STUDIO_SESSION_BYTES) throw new Error('Studio output exceeds the 2 GiB session budget; reduce the output size');
  return bytes;
}

export function studioSceneBytes(scene: StudioScene, layers: number, maxBinding = Infinity): number {
  const buffers = [scene.positions.byteLength, scene.tris.byteLength, scene.tlasNodes.byteLength + scene.blasNodes.byteLength, scene.materials.byteLength, scene.sdata.byteLength];
  const geometry = buffers.reduce((n, bytes) => n + bytes, 0);
  if (geometry > STUDIO_SCENE_BYTES || buffers.some(bytes => bytes > maxBinding) || scene.instanceCount > 1_000_000)
    throw new Error('Studio scene exceeds the geometry budget; reduce mesh detail or scatter count');
  const size = scene.mapSize ?? 1024;
  if (!Number.isInteger(size) || size < 1 || size > 4096 || !Number.isInteger(layers) || layers < 0 || layers > 256)
    throw new Error('Invalid Studio texture dimensions or layer count');
  const textures = size * size * Math.max(1, layers) * 4;
  if (textures > 256 * 1024 ** 2) throw new Error('Studio textures exceed the 256 MiB budget; reduce image resolution');
  return geometry + textures;
}
