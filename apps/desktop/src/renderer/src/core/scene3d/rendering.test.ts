import * as THREE from 'three';
import { describe, expect, it } from 'vitest';

import { sourceMaterialSnapshot } from './rendering';

const textures = new Map<THREE.Texture, string>();
const textureAsset = async (texture: THREE.Texture | undefined) => (texture ? textures.get(texture) : undefined);

describe('Blender snapshot of source materials', () => {
  it('keeps every field of a standard glTF material', async () => {
    const map = new THREE.Texture(); textures.set(map, 'tex-color');
    const material = new THREE.MeshStandardMaterial({
      color: '#ff8000', roughness: 0.25, metalness: 0.75, emissive: '#102030', emissiveIntensity: 2, opacity: 0.5, map,
    });
    expect(await sourceMaterialSnapshot(material, textureAsset)).toEqual({
      p: {
        color: { v: '#ff8000' }, roughness: { v: 0.25 }, metalness: { v: 0.75 },
        emissive: { v: '#102030' }, emissiveIntensity: { v: 2 }, opacity: { v: 0.5 },
      },
      maps: { color: 'tex-color' },
    });
  });

  it('turns an unlit glTF material into emission instead of throwing', async () => {
    const plain = new THREE.MeshBasicMaterial({ color: '#33cc66', opacity: 0.8 });
    expect(await sourceMaterialSnapshot(plain, textureAsset)).toEqual({
      p: {
        color: { v: '#000000' }, roughness: { v: 1 }, metalness: { v: 0 },
        emissive: { v: '#33cc66' }, emissiveIntensity: { v: 1 }, opacity: { v: 0.8 },
      },
      maps: {},
    });
    const map = new THREE.Texture(); textures.set(map, 'tex-unlit');
    const textured = new THREE.MeshBasicMaterial({ color: '#ffffff', map });
    expect(await sourceMaterialSnapshot(textured, textureAsset)).toMatchObject({
      p: { color: { v: '#000000' }, emissive: { v: '#FFFFFF' } }, maps: { emissive: 'tex-unlit' },
    });
  });

  it('fills the worker defaults for materials without PBR fields', async () => {
    const phong = new THREE.MeshPhongMaterial({ color: '#808080', emissive: '#000000' });
    expect((await sourceMaterialSnapshot(phong, textureAsset)).p).toEqual({
      color: { v: '#808080' }, roughness: { v: 1 }, metalness: { v: 0 },
      emissive: { v: '#000000' }, emissiveIntensity: { v: 1 }, opacity: { v: 1 },
    });
  });
});
