// @vitest-environment happy-dom
import { afterEach,describe, expect, it,vi } from 'vitest';
import * as THREE from 'three';
import { NativeStudioBackend } from './backend';
import { extractScene } from '../studio/extract';
import type { StudioView } from '../studio/gpu';
import type { StudioNativeCommand, StudioNativeBridge } from '../../../../shared/studio-native';

const view: StudioView = { camera: { position: [0, 0, 5], forward: [0, 0, -1], right: [1, 0, 0], up: [0, 1, 0], tanY: .5, aspect: 1, lensRadius: 0, focus: 5 }, exposure: 1, backdrop: [1, 1, 1], backdropOpaque: true, shadowCatcher: false, shadows: true, floorY: 0, environment: { zenith: [.2, .2, .2], horizon: [.1, .1, .1], ground: [.1, .1, .1] } };
afterEach(()=>{vi.restoreAllMocks();vi.unstubAllGlobals();});

describe('native Studio adapter', () => {
  it('sends edits made while geometry and textures are uploading on the next frame', async () => {
    let finish!: () => void;
    const held = new Promise<void>(resolve => { finish = resolve; });
    const calls: StudioNativeCommand[] = [];
    let holdTextures = true;
    const bridge: StudioNativeBridge = {
      request: async (_session, command) => {
        calls.push(command);
        if (command.op === 'textures' && holdTextures) { holdTextures = false; await held; }
        return { result: command.op === 'hello' ? { capabilities: ['preparedView'], layout: { materialBytes: 128, triangleBytes: 96 } } : null, binary: new Uint8Array(), generation: 1, roundTripMs: 0 };
      }, release: async () => {}, status: async () => ({ available: true, running: true, generation: 1, failure: null })
    };
    const graph = new THREE.Scene(), mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
    mesh.userData.sceneId = 'box'; graph.add(mesh);
    const extract = () => extractScene(graph, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 });
    const backend = new NativeStudioBackend(bridge); await backend.initialize(); backend.setView(view);
    const describe = () => { throw new Error('binary path'); };
    backend.setPreparedScene(extract(), [new Uint8Array(512 * 512 * 4)], describe);
    const pending = backend.pick(.5, .5);
    await vi.waitFor(() => expect(calls.some(c => c.op === 'textures')).toBe(true));
    mesh.position.x = 2; graph.updateMatrixWorld(true);
    backend.setPreparedScene(extract(), [new Uint8Array(512 * 512 * 4).fill(255)], describe);
    finish(); await pending; await backend.pick(.5, .5);
    expect(calls.filter(c => c.op === 'textures')).toHaveLength(2);
    expect(calls.filter(c => c.op === 'preparedPatch')).toHaveLength(1);
    backend.dispose(); mesh.geometry.dispose(); mesh.material.dispose();
  });
  it('does not reopen a released session when an in-flight upload finishes after disposal',async()=>{
    let complete!:()=>void;
    const held=new Promise<void>(resolve=>{complete=resolve;});
    const calls:string[]=[];
    const release=vi.fn(async()=>{calls.push('release');});
    const bridge:StudioNativeBridge={request:async(_session,command)=>{
      calls.push(command.op);
      if(command.op==='prepared')await held;
      return {result:command.op==='hello'?{layout:{materialBytes:128,triangleBytes:96}}:null,binary:new Uint8Array(),generation:1,roundTripMs:0};
    },release,status:async()=>({available:true,running:true,generation:1,failure:null})};
    const graph=new THREE.Scene(),mesh=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshStandardMaterial());mesh.userData.sceneId='box';graph.add(mesh);
    const backend=new NativeStudioBackend(bridge);await backend.initialize();
    backend.setPreparedScene(extractScene(graph,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0}),[],()=>{throw new Error('binary path');});backend.setView(view);
    const pending=backend.pick(.5,.5);
    await vi.waitFor(()=>expect(calls).toContain('prepared'));
    backend.dispose();complete();
    await expect(pending).rejects.toThrow('closed');
    expect(calls.at(-1)).toBe('release');expect(release).toHaveBeenCalledOnce();
    mesh.geometry.dispose();(mesh.material as THREE.Material).dispose();
  });
  it('reuses a finished trace across layer images and resets it when the camera changes',async()=>{
    const renders:any[]=[];vi.stubGlobal('VideoFrame',class{close(){}});
    vi.spyOn(HTMLCanvasElement.prototype,'getContext').mockReturnValue({drawImage:vi.fn()} as any);
    const bridge:StudioNativeBridge={request:async(_session,command)=>{
      if(command.op==='render'){renders.push(command);return {result:{format:'rgba8',width:64,height:64,samples:16},binary:new Uint8Array(64*64*4),generation:1,roundTripMs:0};}
      return {result:command.op==='hello'?{layout:{materialBytes:128,triangleBytes:96}}:null,binary:new Uint8Array(),generation:1,roundTripMs:0};
    },release:async()=>{},status:async()=>({available:true,running:true,generation:1,failure:null})};
    const graph=new THREE.Scene(),mesh=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshStandardMaterial());mesh.userData.sceneId='box';graph.add(mesh);
    const backend=new NativeStudioBackend(bridge);await backend.initialize();backend.setPreparedScene(extractScene(graph,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0}),[],()=>{throw new Error('binary path');});backend.setView(view);
    const request={tier:'final' as const,size:{width:64,height:64},samples:16,accumulate:false};
    (await backend.render({...request,layer:'box'})).release?.();(await backend.render(request)).release?.();
    backend.setView({...view,camera:{...view.camera,position:[1,0,5]}});(await backend.render(request)).release?.();
    expect(renders.map(r=>r.reset)).toEqual([true,false,true]);
    (await backend.render({...request,layer:'world',background:true})).release?.();expect(renders.at(-1)).toMatchObject({layer:1,background:true,reset:false});backend.dispose();
  });
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
