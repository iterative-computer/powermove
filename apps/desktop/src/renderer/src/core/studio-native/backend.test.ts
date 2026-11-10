// @vitest-environment happy-dom
import { afterEach,describe, expect, it,vi } from 'vitest';
import * as THREE from 'three';
import { NativeStudioBackend } from './backend';
import { STUDIO_MATERIAL_FLOATS, STUDIO_TRIANGLE_FLOATS } from '../studio/layout';
import { extractScene } from '../studio/extract';
import type { StudioView } from '../studio/gpu';
import type { StudioNativeCommand, StudioNativeBridge } from '../../../../shared/studio-native';

const view: StudioView = { camera: { position: [0, 0, 5], forward: [0, 0, -1], right: [1, 0, 0], up: [0, 1, 0], tanY: .5, aspect: 1, lensRadius: 0, focus: 5 }, exposure: 1, backdrop: [1, 1, 1], backdropOpaque: true, shadowCatcher: false, shadows: true, floorY: 0, environment: { zenith: [.2, .2, .2], horizon: [.1, .1, .1], ground: [.1, .1, .1] } };
afterEach(()=>{vi.restoreAllMocks();vi.unstubAllGlobals();});

describe('native Studio adapter', () => {
  it('composes managed VideoFrames directly and releases every surface once, including buffered and late frames', async () => {
    let receive!: NonNullable<StudioNativeBridge['onTexture']> extends (callback: infer T) => unknown ? T : never;
    const unsubscribe = vi.fn(), frames: Array<{ close: ReturnType<typeof vi.fn> }> = [];
    let frameId = 0;
    const deliver = (session: string, id: number) => {
      const frame = { close: vi.fn() }; frames.push(frame);
      void receive({ session, frame: id, width: 64, height: 64 }, { getVideoFrame: () => frame });
      return frame;
    };
    const bridge: StudioNativeBridge = {
      onTexture: callback => { receive = callback; return unsubscribe; },
      request: async (session, command) => {
        if (command.op === 'render') {
          deliver(session, ++frameId);
          return { result: { transport: 'sharedTexture', surface: frameId, width: 64, height: 64, samples: 1 }, binary: new Uint8Array(), generation: 1, roundTripMs: 0 };
        }
        return { result: command.op === 'hello' ? { layout: { materialBytes: STUDIO_MATERIAL_FLOATS * 4, triangleBytes: STUDIO_TRIANGLE_FLOATS * 4 } } : null, binary: new Uint8Array(), generation: 1, roundTripMs: 0 };
      }, release: async () => {}, status: async () => ({ available: true, running: true, generation: 1, failure: null })
    };
    const graph = new THREE.Scene(), mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
    mesh.userData.sceneId = 'box'; graph.add(mesh);
    const backend = new NativeStudioBackend(bridge); await backend.initialize();
    backend.setPreparedScene(extractScene(graph, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 }), [], () => { throw new Error('binary path'); });
    backend.setView(view);
    const canvasContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext');
    const request = { tier: 'live' as const, size: { width: 64, height: 64 }, samples: 1 };
    const first = await backend.render(request), second = await backend.render(request);
    expect(first.source).toBe(frames[0]); expect(second.source).toBe(frames[1]);
    expect(canvasContext).not.toHaveBeenCalled();
    expect(frames[0]!.close).not.toHaveBeenCalled();
    first.release!(); first.release!();
    expect(frames[0]!.close).toHaveBeenCalledOnce();
    for (let i = 0; i < 4; i++) deliver(backend.session, ++frameId);
    expect(frames[2]!.close).toHaveBeenCalledOnce();
    backend.dispose(); second.release!(); backend.dispose();
    expect(unsubscribe).toHaveBeenCalledOnce();
    expect(frames.every(frame => frame.close.mock.calls.length === 1)).toBe(true);
    expect(deliver(backend.session, ++frameId).close).toHaveBeenCalledOnce();
    mesh.geometry.dispose(); (mesh.material as THREE.Material).dispose();
  });

  it('drops Refine interrupted during an upload before submitting GPU work',async()=>{
    let complete!:()=>void;const uploading=new Promise<void>(resolve=>{complete=resolve;}),calls:string[]=[];
    const bridge:StudioNativeBridge={request:async(_session,command)=>{calls.push(command.op);if(command.op==='prepared')await uploading;return {result:command.op==='hello'?{capabilities:['cancelRefine'],layout:{materialBytes:STUDIO_MATERIAL_FLOATS*4,triangleBytes:STUDIO_TRIANGLE_FLOATS*4}}:null,binary:new Uint8Array(),generation:1,roundTripMs:0};},release:async()=>{},status:async()=>({available:true,running:true,generation:1,failure:null})};
    const graph=new THREE.Scene(),geometry=new THREE.BoxGeometry(),material=new THREE.MeshStandardMaterial(),mesh=new THREE.Mesh(geometry,material);mesh.userData.sceneId='box';graph.add(mesh);
    const backend=new NativeStudioBackend(bridge);await backend.initialize();backend.setPreparedScene(extractScene(graph,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0}),[],()=>{throw new Error('binary path');});backend.setView(view);
    const pending=expect(backend.render({tier:'refine',samples:32,size:{width:64,height:64}})).rejects.toMatchObject({name:'StudioFrameCancelled'});
    await vi.waitFor(()=>expect(calls).toContain('prepared'));backend.cancelRefine();complete();await pending;
    expect(calls).toContain('cancelRefine');expect(calls).not.toContain('render');backend.dispose();geometry.dispose();material.dispose();
  });
  it('streams large scenes in bounded packets and sends repeated part identities as binary indices',async()=>{
    const calls:{command:StudioNativeCommand;binary:Uint8Array}[]=[];
    const bridge:StudioNativeBridge={request:async(_session,command,binary)=>{calls.push({command,binary:binary?.slice()??new Uint8Array()});return {result:command.op==='hello'?{capabilities:['preparedUpload','instanceParts'],layout:{materialBytes:STUDIO_MATERIAL_FLOATS*4,triangleBytes:STUDIO_TRIANGLE_FLOATS*4}}:null,binary:new Uint8Array(),generation:1,roundTripMs:0};},release:async()=>{},status:async()=>({available:true,running:true,generation:1,failure:null})};
    const graph=new THREE.Scene(),mesh=new THREE.Mesh(new THREE.SphereGeometry(1,128,64),new THREE.MeshStandardMaterial());mesh.userData.sceneId='sphere';graph.add(mesh);
    const scene=extractScene(graph,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0});
    const backend=new NativeStudioBackend(bridge);await backend.initialize();backend.setPreparedScene(scene,[],()=>{throw new Error('JSON mesh copy');});backend.setView(view);await backend.bounds();
    const begin=calls.find(c=>c.command.op==='preparedBegin')!,chunks=calls.filter(c=>c.command.op==='preparedChunk');
    expect(begin).toBeTruthy();expect(chunks.length).toBeGreaterThan(1);expect(chunks.every(c=>c.binary.length<=1024*1024)).toBe(true);
    let offset=0;for(const chunk of chunks){expect(chunk.command.offset).toBe(offset);offset+=chunk.binary.length;}expect(offset).toBe(begin.command.bytes);
    expect((begin.command.upload as any).instanceParts.names).toHaveLength(1);expect(begin.command.upload).not.toHaveProperty('instancePolicies');
    expect(calls.findIndex(c=>c.command.op==='preparedCommit')).toBeGreaterThan(calls.map(c=>c.command.op).lastIndexOf('preparedChunk'));
    const all=new Uint8Array(offset);for(const c of chunks)all.set(c.binary,c.command.offset as number);
    const range=(begin.command.upload as any).positions;expect(all.slice(range.offset,range.offset+range.length)).toEqual(new Uint8Array(scene.positions.buffer));
    backend.dispose();mesh.geometry.dispose();mesh.material.dispose();
  });
  it('sends edits made while geometry and textures are uploading on the next frame', async () => {
    let finish!: () => void;
    const held = new Promise<void>(resolve => { finish = resolve; });
    const calls: StudioNativeCommand[] = [];
    let holdTextures = true;
    const bridge: StudioNativeBridge = {
      request: async (_session, command) => {
        calls.push(command);
        if (command.op === 'textures' && holdTextures) { holdTextures = false; await held; }
        return { result: command.op === 'hello' ? { capabilities: ['preparedView'], layout: { materialBytes:STUDIO_MATERIAL_FLOATS*4, triangleBytes:STUDIO_TRIANGLE_FLOATS*4 } } : null, binary: new Uint8Array(), generation: 1, roundTripMs: 0 };
      }, release: async () => {}, status: async () => ({ available: true, running: true, generation: 1, failure: null })
    };
    const graph = new THREE.Scene(), mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
    mesh.userData.sceneId = 'box'; graph.add(mesh);
    const extract = () => Object.assign(extractScene(graph, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 }),{mapSize:512});
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
  it('releases the resident texture array when source maps are removed',async()=>{
    const calls:{command:StudioNativeCommand;bytes:number}[]=[];
    const bridge:StudioNativeBridge={request:async(_session,command,binary)=>{
      calls.push({command,bytes:binary?.byteLength ?? 0});
      return {result:command.op==='hello'?{layout:{materialBytes:STUDIO_MATERIAL_FLOATS*4,triangleBytes:STUDIO_TRIANGLE_FLOATS*4}}:null,binary:new Uint8Array(),generation:1,roundTripMs:0};
    },release:async()=>{},status:async()=>({available:true,running:true,generation:1,failure:null})};
    const graph=new THREE.Scene(),material=new THREE.MeshStandardMaterial(),mesh=new THREE.Mesh(new THREE.BoxGeometry(),material);mesh.userData.sceneId='box';graph.add(mesh);
    const prepared=extractScene(graph,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0});prepared.mapSize=2;
    const backend=new NativeStudioBackend(bridge);await backend.initialize();backend.setView(view);
    backend.setPreparedScene(prepared,[new Uint8Array(16)],()=>{throw new Error('binary path');});await backend.bounds();
    backend.setPreparedScene(prepared,[],()=>{throw new Error('binary path');});await backend.bounds();
    backend.setPreparedScene(prepared,[],()=>{throw new Error('binary path');});await backend.bounds();
    expect(calls.filter(c=>c.command.op==='textures')).toEqual([
      {command:{op:'textures',width:2,height:2,layers:1},bytes:16},
      {command:{op:'textures',width:1,height:1,layers:1},bytes:4},
    ]);
    backend.dispose();mesh.geometry.dispose();material.dispose();
  });
  it('does not reopen a released session when an in-flight upload finishes after disposal',async()=>{
    let complete!:()=>void;
    const held=new Promise<void>(resolve=>{complete=resolve;});
    const calls:string[]=[];
    const release=vi.fn(async()=>{calls.push('release');});
    const bridge:StudioNativeBridge={request:async(_session,command)=>{
      calls.push(command.op);
      if(command.op==='prepared')await held;
      return {result:command.op==='hello'?{layout:{materialBytes:STUDIO_MATERIAL_FLOATS*4,triangleBytes:STUDIO_TRIANGLE_FLOATS*4}}:null,binary:new Uint8Array(),generation:1,roundTripMs:0};
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
    const renders:any[]=[],close=vi.fn(),drawImage=vi.fn();vi.stubGlobal('VideoFrame',class{close(){close();}});
    vi.spyOn(HTMLCanvasElement.prototype,'getContext').mockReturnValue({drawImage} as any);
    const bridge:StudioNativeBridge={request:async(_session,command)=>{
      if(command.op==='render'){renders.push(command);return {result:{format:'rgba8',width:64,height:64,samples:16},binary:new Uint8Array(64*64*4),generation:1,roundTripMs:0};}
      return {result:command.op==='hello'?{layout:{materialBytes:STUDIO_MATERIAL_FLOATS*4,triangleBytes:STUDIO_TRIANGLE_FLOATS*4}}:null,binary:new Uint8Array(),generation:1,roundTripMs:0};
    },release:async()=>{},status:async()=>({available:true,running:true,generation:1,failure:null})};
    const graph=new THREE.Scene(),mesh=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshStandardMaterial());mesh.userData.sceneId='box';graph.add(mesh);
    const backend=new NativeStudioBackend(bridge);await backend.initialize();backend.setPreparedScene(extractScene(graph,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0}),[],()=>{throw new Error('binary path');});backend.setView(view);
    const request={tier:'final' as const,size:{width:64,height:64},samples:16,accumulate:false};
    (await backend.render({...request,layer:'box'})).release?.();(await backend.render(request)).release?.();
    backend.setView({...view,camera:{...view.camera,position:[1,0,5]}});(await backend.render(request)).release?.();
    expect(renders.map(r=>r.reset)).toEqual([true,false,true]);
    (await backend.render({...request,layer:'world',background:true})).release?.();expect(renders.at(-1)).toMatchObject({layer:1,background:true,reset:false});
    drawImage.mockImplementationOnce(()=>{throw new Error('Canvas unavailable');});
    await expect(backend.render(request)).rejects.toThrow('Canvas unavailable');
    expect(close).toHaveBeenCalledTimes(renders.length);backend.dispose();
  });
  it('uploads binary geometry once and sends a small patch while the camera moves', async () => {
    const calls: { command: StudioNativeCommand; bytes: number }[] = [];
    const bridge: StudioNativeBridge = { request: async (_session, command, binary) => { calls.push({ command, bytes: binary?.byteLength ?? 0 }); return { result: command.op === 'hello' ? { protocol: 1, capabilities:['preparedView'], layout:{materialBytes:STUDIO_MATERIAL_FLOATS*4,triangleBytes:STUDIO_TRIANGLE_FLOATS*4} } : command.op==='bounds'?{min:[-1,-1,-1],max:[1,1,1]}:null, binary: new Uint8Array(), generation: 1, roundTripMs: .1 }; }, release: async () => {}, status: async () => ({ available: true, running: true, generation: 1, failure: null }) };
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
