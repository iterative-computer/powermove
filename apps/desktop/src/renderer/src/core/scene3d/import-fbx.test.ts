import { afterEach, describe, expect, it, vi } from 'vitest';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { fbxToGlb } from './import-fbx';
afterEach(resetBridgeForTests);

describe('native FBX import', () => {
  it('sends only bytes to a disposable import session and retains the portable GLB', async () => {
    const binary = new Uint8Array(20);new DataView(binary.buffer).setUint32(0,0x46546c67,true);
    const request = vi.fn(async()=>({binary,result:{warnings:[]}}));
    installBridgeForTests({studioNative:{request}} as any);
    const file = await fbxToGlb(new File(['FBX fixture'],'Animated.fbx'));
    expect(file.name).toBe('Animated.glb');expect(file.type).toBe('model/gltf-binary');
    expect([...new Uint8Array(await file.arrayBuffer())]).toEqual([...binary]);
    expect(request).toHaveBeenCalledWith(expect.stringMatching(/^fbx:/),{op:'importFbx'},new TextEncoder().encode('FBX fixture'));
  });
  it('rejects empty input, unavailable native importers and malformed replies', async()=>{
    await expect(fbxToGlb(new File([],'empty.fbx'))).rejects.toThrow('smaller than');
    installBridgeForTests(undefined);
    await expect(fbxToGlb(new File(['x'],'model.fbx'))).rejects.toThrow('desktop Studio');
    installBridgeForTests({studioNative:{request:async()=>({binary:new Uint8Array(3),result:{}})}} as any);
    await expect(fbxToGlb(new File(['x'],'model.fbx'))).rejects.toThrow('invalid GLB');
  });
});
