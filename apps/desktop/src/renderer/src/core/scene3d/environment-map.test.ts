// @vitest-environment happy-dom
import { describe, it, expect, vi } from 'vitest';
import * as THREE from 'three';
import { loadEnvironmentImage } from './environment-map';
import { hdrPanorama, exrPanorama } from './__fixtures__/environment';
import { parseScene, sceneAssetIds } from './schema';
import { SceneRuntime } from './runtime';

describe('HDR and EXR environments', () => {
  it.each([['hdr', hdrPanorama], ['exr', exrPanorama]] as const)('decodes %s as linear half-float with values above white', async (format, fixture) => {
    const image = await loadEnvironmentImage(new Blob([fixture()]), `studio.${format}`);
    expect(image.format).toBe(format); expect([image.w, image.h]).toEqual([128, 64]);
    expect(image.environmentTexture.colorSpace).toBe(THREE.LinearSRGBColorSpace);
    expect(image.environmentTexture.mapping).toBe(THREE.EquirectangularReflectionMapping);
    const data = image.environmentTexture.image.data as Uint16Array;
    expect(THREE.DataUtils.fromHalfFloat(data[0]!)).toBeCloseTo(4, 1);
    const runtime = new SceneRuntime(), scene = parseScene({environment:{map:'env',p:{intensity:2,rotation:90}}});
    const asset = {environmentTexture:image.environmentTexture};
    runtime.sync(scene, () => asset);
    expect(runtime.scene.environment).toBe(image.environmentTexture);
    expect(runtime.scene.environmentIntensity).toBe(2); expect(runtime.scene.environmentRotation.y).toBeCloseTo(Math.PI / 2);
    const dispose = vi.spyOn(image.environmentTexture, 'dispose');
    for (let i=0; i<20; i++) runtime.sync(scene, () => asset);
    expect(runtime.scene.environment).toBe(image.environmentTexture); expect(dispose).not.toHaveBeenCalled();
    expect(sceneAssetIds(scene)).toEqual(['env']);
    runtime.sync(parseScene({})); expect(runtime.scene.environment).toBeNull();
    runtime.dispose(); expect(dispose).not.toHaveBeenCalled(); image.environmentTexture.dispose();
  });
  it('bounds GPU panorama dimensions while retaining linear lighting values', async () => {
    const image=await loadEnvironmentImage(new Blob([hdrPanorama(8192,2)]),'wide.hdr');
    expect([image.w,image.h]).toEqual([8192,2]);
    expect([image.environmentTexture.image.width,image.environmentTexture.image.height]).toEqual([4096,1]);
    expect(THREE.DataUtils.fromHalfFloat((image.environmentTexture.image.data as Uint16Array)[0]!)).toBeCloseTo(4,1);
    image.environmentTexture.dispose();
  });
  it('reports malformed maps and oversized input', async () => {
    await expect(loadEnvironmentImage(new Blob(['broken']), 'bad.exr')).rejects.toThrow('Could not read');
    await expect(loadEnvironmentImage({size:257*1024*1024} as Blob, 'huge.hdr')).rejects.toThrow('256 MB');
  });
});
