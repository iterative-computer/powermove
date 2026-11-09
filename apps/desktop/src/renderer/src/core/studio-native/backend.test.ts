import { describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { NativeStudioBackend } from './backend';
import { extractScene } from '../studio/extract';
import type { StudioView } from '../studio/gpu';
import type { StudioNativeCommand, StudioNativeBridge } from '../../../../shared/studio-native';

const view: StudioView = { camera: { position: [0, 0, 5], forward: [0, 0, -1], right: [1, 0, 0], up: [0, 1, 0], tanY: .5, aspect: 1, lensRadius: 0, focus: 5 }, exposure: 1, backdrop: [1, 1, 1], backdropOpaque: true, shadowCatcher: false, shadows: true, floorY: 0, environment: { zenith: [.2, .2, .2], horizon: [.1, .1, .1], ground: [.1, .1, .1] } };

describe('native Studio adapter', () => {
  it('uploads binary geometry once and sends a small patch while the camera moves', async () => {
    const calls: { command: StudioNativeCommand; bytes: number }[] = [];
    const bridge: StudioNativeBridge = { request: async (_session, command, binary) => { calls.push({ command, bytes: binary?.byteLength ?? 0 }); return { result: command.op === 'hello' ? { protocol: 1, capabilities:['preparedView'], layout:{materialBytes:128,triangleBytes:96} } : command.op==='bounds'?{min:[-1,-1,-1],max:[1,1,1]}:null, binary: new Uint8Array(), generation: 1, roundTripMs: .1 }; }, release: async () => {}, status: async () => ({ available: true, running: true, generation: 1, failure: null }) };
    const scene = new THREE.Scene(), geometry = new THREE.SphereGeometry(1, 32, 16), material = new THREE.MeshStandardMaterial();
    const object = new THREE.Mesh(geometry, material); object.userData.sceneId = 'sphere'; scene.add(object); scene.updateMatrixWorld(true);
    const prepared = extractScene(scene, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 });
    const backend = new NativeStudioBackend(bridge); await backend.initialize();
    backend.setPreparedScene(prepared, [], () => { throw new Error('Fast path must not copy meshes to JSON'); }); backend.setView(view);
    await backend.pick(.5, .5);
    backend.setView({ ...view, camera: { ...view.camera, position: [.1, 0, 5] } });
    await backend.pick(.5, .5);
    const viewPatch=calls.find(c=>c.command.op==='preparedView')!;expect(viewPatch.bytes).toBe(0);expect(viewPatch.command).not.toHaveProperty('sdata');
    object.position.x=2;const moved=extractScene(scene,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0});backend.setPreparedScene(moved,[],()=>{throw new Error('binary path');});await backend.pick(.5,.5);
    const full = calls.find(c => c.command.op === 'prepared')!, patch = calls.find(c => c.command.op === 'preparedPatch')!;
    expect(full.bytes).toBeGreaterThan(100_000); expect(patch.bytes).toBeLessThan(full.bytes / 20);
    expect(patch.command).not.toHaveProperty('positions'); expect(patch.command).not.toHaveProperty('tris');
    expect(await backend.bounds()).toEqual({ min: { x: -1, y: -1, z: -1 }, max: { x: 1, y: 1, z: 1 } });
    backend.dispose(); geometry.dispose(); material.dispose();
  });
});
