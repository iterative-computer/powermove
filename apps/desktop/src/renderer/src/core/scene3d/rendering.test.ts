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

describe('cancelRender with the engine on', () => {
  // A host bridge whose Blender worker behaves like main's (blender-ipc.ts):
  // cancel disposes the window's worker, which rejects the running job.
  const worker = () => {
    const calls = { cancel: 0, run: 0 };
    let fail: ((error: Error) => void) | null = null;
    const blender = {
      status: async () => ({ available: true }),
      run: () => { calls.run++; return new Promise<never>((_, reject) => { fail = reject; }); },
      cancel: async () => { calls.cancel++; fail?.(new Error('Blender job cancelled')); fail = null; },
    };
    return { calls, blender };
  };
  const enginePM = (preview: { cancel(): Promise<void> }) => ({ EngineConnection: { preview }, EngineDoc: { enabled: true }, bus: { emit() {} } });

  it('stops the window Blender worker that runs Regenerate and .blend import', async () => {
    const { installBridgeForTests } = await import('../../kernel/bridge');
    const { cancelBlender } = await import('./rendering');
    const { editModel } = await import('./modeling');
    const { calls, blender } = worker();
    installBridgeForTests({ blender } as any);
    try {
      let previewCancels = 0;
      const PM: any = enginePM({ cancel: async () => { previewCancels++; } });
      PM.proj = { assets: {} }; PM.sel = { layers: [] }; PM._scene3dRevision = 0;
      const job = editModel(PM, { operation: 'create_model', recipe: { kind: 'rocket', parameters: {}, modifiers: [] } });
      await Promise.resolve();
      expect(calls.run).toBe(1);
      await cancelBlender(PM);
      expect(calls.cancel).toBe(1);
      expect(previewCancels).toBe(1);
      expect(await job).toEqual({ ok: false, message: 'Blender job cancelled' });
    } finally {
      installBridgeForTests(undefined);
    }
  });

  it('resolves as today when no native viewer session can take the cancel', async () => {
    const { installBridgeForTests } = await import('../../kernel/bridge');
    const { cancelBlender } = await import('./rendering');
    const { calls, blender } = worker();
    installBridgeForTests({ blender } as any);
    try {
      for (const name of ['EnginePreviewUnavailable', 'EnginePreviewCancelled']) {
        const PM = enginePM({ cancel: async () => { throw Object.assign(new Error('no session'), { name }); } });
        await expect(cancelBlender(PM)).resolves.toBeUndefined();
      }
      expect(calls.cancel).toBe(2);
      await expect(cancelBlender({ EngineDoc: { enabled: true }, bus: { emit() {} } })).resolves.toBeUndefined();
      expect(calls.cancel).toBe(3);
      const broken = enginePM({ cancel: async () => { throw new Error('protocol fault'); } });
      await expect(cancelBlender(broken)).rejects.toThrow('protocol fault');
    } finally {
      installBridgeForTests(undefined);
    }
  });
});
