import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import * as halfReadback from './half-readback';
import {
  compositingLayers, compositingLevel, continuousRasterScale, effectParamValue, groupContainsSolo,
  hasRenderableEffects, install, paramUniformName, svgRasterDimensions, trackPresentedVideoFrames,
} from './compositor';

function compositorRegistry(): PMRegistry {
  const PM: PMRegistry = {
    proj: { w: 1920, h: 1080, layers: [] },
    TYPE_META: {},
    active: (item: any, time: any) => time >= item.from && time < item.from + item.dur,
    worldMatrix: () => [1, 0, 0, 1, 0, 0],
    raster: () => ({
      w: 500,
      h: 300,
      anchorX: 250,
      anchorY: 150,
      selection: { x0: -180, y0: -42, x1: 180, y1: 42, w: 360, h: 84 },
    }),
  };
  install(PM);
  return PM;
}

/** Exercise the real source-key/upload path without a GPU. Programs and the
* working target are already resident; only source texture uploads are faked. */
function preparedVideoRegistry() {
  const PM = compositorRegistry(), canvas = { width: 4, height: 2 }, video = { videoWidth: 4, videoHeight: 2, readyState: 2, currentTime: 0, addEventListener: vi.fn() };
  const assets = new Map([['asset', { id: 'asset', el: video, w: 4, h: 2, dur: 1 }], ['alias', { id: 'alias', el: video, w: 4, h: 2, dur: 1 }]]);
  const layer = (id: string, asset = 'asset') => ({ id, type: 'video', from: 0, dur: 1, d: { asset, w: 2, h: 2, speed: 1, trim: 0 }, fx: [], masks: [] });
  PM.proj = { w: 2, h: 2, dur: 1, fps: 30, colorDepth: 8, layers: [layer('first'), layer('second', 'alias')] };
  PM.assets = { get: (id: string) => assets.get(id) };
  PM.Export = { busy: true };
  PM.preparedVideoVersion = 1;
  PM.preparedVideoFrames = new Map([['first@0', canvas], ['second@0', canvas]]);
  PM.beginEval = vi.fn();
  PM.scope = { push: vi.fn(), pop: vi.fn() };
  PM.ev = () => 100;
  PM.clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
  PM.mul = (_a: any, b: any) => b;
  const gl: any = {};
  for (const [name, index] of ['FRAMEBUFFER', 'TEXTURE_2D', 'RGBA', 'UNSIGNED_BYTE', 'TEXTURE0', 'TEXTURE_MIN_FILTER', 'TEXTURE_MAG_FILTER',
    'TEXTURE_WRAP_S', 'TEXTURE_WRAP_T', 'LINEAR', 'CLAMP_TO_EDGE', 'UNPACK_PREMULTIPLY_ALPHA_WEBGL', 'UNPACK_FLIP_Y_WEBGL',
    'COLOR_BUFFER_BIT', 'DEPTH_BUFFER_BIT', 'ARRAY_BUFFER', 'FLOAT', 'TRIANGLE_STRIP', 'ONE', 'ONE_MINUS_SRC_ALPHA', 'BLEND'].map((name, index) => [name, index] as const))
    gl[name] = index + 1;
  for (const name of ['bindFramebuffer', 'viewport', 'activeTexture', 'bindTexture', 'texParameteri', 'pixelStorei', 'clearColor', 'clearDepth', 'clear',
    'useProgram', 'bindBuffer', 'enableVertexAttribArray', 'vertexAttribPointer', 'uniformMatrix3fv', 'uniform1i', 'uniform1f', 'uniform2f', 'uniform4f',
    'uniform2fv', 'uniform3fv', 'uniform4fv', 'drawArrays', 'readPixels', 'deleteTexture', 'enable', 'disable', 'blendFunc'])
    gl[name] = vi.fn();
  gl.createTexture = vi.fn(() => ({}));
  gl.texImage2D = vi.fn();
  gl.getUniformLocation = vi.fn((_program: any, name: string) => name);
  Object.assign(PM.GL, { gl, canvas: { width: 2, height: 2 }, supportedColorDepths: [8], colorDepth: 8,
    pool: [{ fb: {}, tex: {}, w: 2, h: 2, colorDepth: 8, busy: false }] });
  PM.GL.progs.set('draw', { pr: {}, a: 0, u: new Map() });
  return { PM, gl, canvas, video, assets, layer, capture: (options: any = {}) => PM.GL.renderToPixels(0, 2, 2, { transparent: true, mblur: false, ...options }) };
}
describe('prepared video texture sharing', () => {
  it('keeps viewer uploads separate while an export owns prepared frames', () => {
    const h = preparedVideoRegistry();
    h.PM.GL.progs.set('present', { pr: {}, a: 0, u: new Map() });
    vi.stubGlobal('window', { performance });
    try {
      h.PM.GL.render(0, { transparent: true, sourceClipping: false, mblur: false });
      expect(h.gl.texImage2D).toHaveBeenCalledTimes(2);
      expect(h.PM.GL.texes.size).toBe(2);
      h.PM.GL.pool.push({ fb: {}, tex: {}, w: 2, h: 2, colorDepth: 8, busy: false });
      h.capture();
      expect(h.gl.texImage2D).toHaveBeenCalledTimes(3);
    } finally {
      vi.unstubAllGlobals();
    }
  });
  it('uploads one identical prepared canvas across decoder aliases, retains the first asset key, and accounts it once', () => {
    const h = preparedVideoRegistry();
    h.capture();
    expect(h.gl.texImage2D).toHaveBeenCalledOnce();
    expect(h.gl.texImage2D.mock.calls[0]![5]).toBe(h.canvas);
    expect([...h.PM.GL.texes.keys()]).toEqual(['a:alias:second@0']);
    expect(h.PM.GL.memoryStats().textures).toMatchObject({ entries: 1, bytes: 32 });
    const token = h.PM.GL.texes.get('a:alias:second@0').v;
    expect(typeof token).toBe('object');
    h.capture();
    expect(h.gl.texImage2D).toHaveBeenCalledOnce();
  });
  it('does not merge different time snapshots or sizes, even when they use the same source asset', () => {
    const h = preparedVideoRegistry(), other = { width: 2, height: 2 };
    h.PM.proj.layers[1].d.asset = 'asset';
    h.PM.preparedVideoFrames.set('second@0', other);
    h.capture();
    expect(h.gl.texImage2D.mock.calls.map((args: any[]) => args[5])).toEqual([other, h.canvas]);
    expect(h.PM.GL.memoryStats().textures).toMatchObject({ entries: 2, bytes: 48 });
  });
  it.each(['version', 'map', 'canvas'] as const)('uploads changed prepared pixels when %s changes despite an identical layer/time key', change => {
    const h = preparedVideoRegistry();
    h.capture();
    const first = h.PM.GL.texes.get('a:alias:second@0').v;
    if (change === 'version')
      h.PM.preparedVideoVersion++;
    if (change === 'map')
      h.PM.preparedVideoFrames = new Map(h.PM.preparedVideoFrames);
    if (change === 'canvas') {
      const replacement = { width: 4, height: 2 };
      h.PM.preparedVideoFrames.set('first@0', replacement);
      h.PM.preparedVideoFrames.set('second@0', replacement);
    }
    h.capture();
    expect(h.gl.texImage2D).toHaveBeenCalledTimes(2);
    expect(h.PM.GL.texes.get('a:alias:second@0').v).not.toBe(first);
  });
  it('retries an unsuccessful upload for the next alias and reuses pixels only after a successful upload', () => {
    const h = preparedVideoRegistry();
    h.gl.texImage2D.mockImplementationOnce(() => { throw new Error('Upload failed'); });
    h.capture();
    expect(h.gl.texImage2D).toHaveBeenCalledTimes(2);
    expect(h.PM.GL.texes.size).toBe(1);
    h.capture();
    expect(h.gl.texImage2D).toHaveBeenCalledTimes(2);
  });
  it('reuploads after asset eviction without retaining duplicate texture entries', () => {
    const h = preparedVideoRegistry();
    h.capture();
    const texture = h.PM.GL.texes.get('a:alias:second@0').tex;
    h.PM.GL.dropTextures('a:alias:');
    expect(h.gl.deleteTexture).toHaveBeenCalledWith(texture);
    expect(h.PM.GL.memoryStats().textures).toMatchObject({ entries: 0, bytes: 0 });
    h.capture();
    expect(h.gl.texImage2D).toHaveBeenCalledTimes(2);
    expect(h.PM.GL.memoryStats().textures).toMatchObject({ entries: 1, bytes: 32 });
  });
  it.each(['normal', 'isolated'] as const)('keeps %s capture source ownership independent', mode => {
    const h = preparedVideoRegistry();
    if (mode === 'normal')
      h.PM.Export.busy = false;
    h.capture(mode === 'isolated' ? { isolatedVideoFrames: true } : {});
    expect(h.gl.texImage2D).toHaveBeenCalledTimes(2);
    expect(h.PM.GL.texes.size).toBe(mode === 'isolated' ? 0 : 2);
    h.PM.Export.busy = true;
    h.capture();
    expect(h.gl.texImage2D).toHaveBeenCalledTimes(3);
  });
  it('keeps live-video fallback on its existing decoder texture and numeric versions', () => {
    const h = preparedVideoRegistry();
    h.PM.preparedVideoFrames = null;
    h.capture();
    expect(h.gl.texImage2D).toHaveBeenCalledOnce();
    const [key, entry] = [...h.PM.GL.texes.entries()][0]!;
    expect(key.startsWith('video:')).toBe(true);
    expect(typeof entry.v).toBe('number');
  });
  it('refreshes prepared texture identity after GL.init even when a context lookup fails', () => {
    const h = preparedVideoRegistry();
    h.capture();
    const first = h.PM.GL.texes.get('a:alias:second@0').v;
    h.PM.GL.init({ getContext: () => null }, { quiet: true });
    h.capture();
    expect(h.gl.texImage2D).toHaveBeenCalledTimes(2);
    expect(h.PM.GL.texes.get('a:alias:second@0').v).not.toBe(first);
  });
});

describe('legacy compositor install', () => {
  it.each([8, 16, 32])('preserves %i-bit CPU fallback for null private reader placeholders', depth => {
    const PM = compositorRegistry();
    const gl = { FRAMEBUFFER: 1, RGBA: 2, FLOAT: 3, UNSIGNED_BYTE: 4, bindFramebuffer: vi.fn(), viewport: vi.fn(),
      readPixels: vi.fn((_x: number, _y: number, _w: number, _h: number, _format: number, type: number, pixels: Float32Array | Uint8Array) => pixels.fill(type === 3 ? .5 : 2)) };
    const scene = { fb: {}, tex: {}, w: 2, h: 1, colorDepth: depth, busy: true };
    PM.beginEval = vi.fn(); PM.scope = { push: vi.fn(), pop: vi.fn() };
    Object.assign(PM.GL, { gl, canvas: { width: 2, height: 1 }, renderProject: vi.fn(() => scene) });
    for (const readers of [[null, undefined], [undefined, null], [null, null]]) {
      const pixels = PM.GL.renderToPixels(0, 2, 1, { transparent: true }, undefined, ...readers);
      expect(pixels).toEqual(new Uint8Array(8).fill(depth === 8 ? 2 : 128));
    }
    expect(gl.readPixels).toHaveBeenCalledTimes(3);
    expect(PM.scope.pop).toHaveBeenCalledTimes(3);
    expect(scene.busy).toBe(false);
  });

  it.each([16, 32] as const)('queues only matching actual %i-bit alpha targets without changing quality or options', requested => {
    const PM = compositorRegistry(), queue = vi.fn(), ticket = { readInto: vi.fn(), dispose: vi.fn() };
    const factory = vi.spyOn(halfReadback, 'createAlphaReadback').mockImplementation((_gl, _w, _h, _generation, _depth, render, _valid, matches) => ({
      byteLength: 16,
      begin(time, options = {}) {
        if (!matches()) return null;
        const calls = queue.mock.calls.length;
        render(time, options, { queue });
        return queue.mock.calls.length > calls ? ticket : null;
      },
      dispose() {},
    }));
    try {
      const gl = { FRAMEBUFFER: 1, bindFramebuffer: vi.fn(), viewport: vi.fn(), readPixels: vi.fn() };
      const scene = { fb: {}, tex: {}, w: 2, h: 1, colorDepth: requested as number, busy: true };
      const options = { transparent: true, mblur: false, mbSamples: 20, shutter: -.5 };
      PM.proj.colorDepth = requested; PM.quality = .25;
      PM.beginEval = vi.fn(); PM.scope = { push: vi.fn(), pop: vi.fn() };
      Object.assign(PM.GL, { gl, canvas: { width: 2, height: 1 }, supportedColorDepths: [8, 16, 32], renderProject: vi.fn(() => scene) });
      const session = PM.GL.createAlphaReadback(2, 1);
      expect(factory.mock.calls[0]![4]).toBe(requested);
      expect(session.begin(.25, options)).toBe(ticket);
      expect(queue).toHaveBeenCalledExactlyOnceWith(scene.tex);
      expect(PM.GL.renderProject).toHaveBeenCalledWith(PM.proj, .25, 2, 1, options);
      expect(PM.quality).toBe(.25);
      // Even a depth change inside rendering must not allocate/read a speculative
      // FLOAT or RGBA8 frame; the caller will join and serially capture this time.
      for (const actual of [8, requested === 16 ? 32 : 16]) {
        scene.colorDepth = actual;
        expect(session.begin(.5, options)).toBeNull();
        expect(queue).toHaveBeenCalledOnce();
      }
      expect(gl.readPixels).not.toHaveBeenCalled();
      expect(PM.scope.pop).toHaveBeenCalledTimes(3);
      expect(scene.busy).toBe(false);
      expect(gl.bindFramebuffer).toHaveBeenLastCalledWith(gl.FRAMEBUFFER, null);
    } finally { factory.mockRestore(); }
  });

  it.each([8, 16, 32, undefined] as const)('preflights requested alpha depth %s before creating GPU resources', requested => {
    const PM = compositorRegistry(), factory = vi.spyOn(halfReadback, 'createAlphaReadback').mockReturnValue(null);
    try {
      PM.proj.colorDepth = requested;
      Object.assign(PM.GL, { gl: {}, supportedColorDepths: [8, 16] });
      expect(PM.GL.createAlphaReadback(2, 1)).toBeNull();
      expect(factory).toHaveBeenCalledTimes(requested === 8 || requested === 32 ? 0 : 1);
      if (requested !== 8 && requested !== 32) expect(factory.mock.calls[0]![4]).toBe(16);
    } finally { factory.mockRestore(); }
  });

  it('rechecks requested/available depth and init generation for each alpha session', () => {
    const PM = compositorRegistry(), gl = {}, factory = vi.spyOn(halfReadback, 'createAlphaReadback').mockReturnValue(null);
    try {
      PM.proj.colorDepth = 32;
      Object.assign(PM.GL, { gl, supportedColorDepths: [8, 16, 32] });
      PM.GL.createAlphaReadback(2, 1);
      const first = factory.mock.calls[0]!, valid = first[6], matches = first[7];
      expect(valid()).toBe(true); expect(matches()).toBe(true);
      PM.proj.colorDepth = 16; expect(matches()).toBe(false);
      PM.proj.colorDepth = 8; expect(matches()).toBe(false);
      PM.proj.colorDepth = 32; PM.GL.supportedColorDepths = [8, 16]; expect(matches()).toBe(false);
      PM.GL.supportedColorDepths = [8, 16, 32]; expect(matches()).toBe(true);
      PM.GL.init({ getContext: () => null }, { quiet: true }); PM.GL.gl = gl;
      expect(valid()).toBe(false);
      PM.GL.createAlphaReadback(2, 1);
      expect(factory.mock.calls[1]![3]).not.toBe(first[3]);
      expect(factory.mock.calls[1]![6]()).toBe(true);
    } finally { factory.mockRestore(); }
  });

  it('releases the accumulator and restores scoped state if a deferred alpha queue throws', () => {
    const PM = compositorRegistry(), queue = vi.fn(() => { throw new Error('Queue failed'); });
    const factory = vi.spyOn(halfReadback, 'createAlphaReadback').mockImplementation((_gl, _w, _h, _generation, _depth, render) => ({
      byteLength: 16, begin(time, options = {}) { render(time, options, { queue }); return null; }, dispose() {},
    }));
    try {
      const gl = { FRAMEBUFFER: 1, bindFramebuffer: vi.fn(), viewport: vi.fn(), readPixels: vi.fn() };
      const scene = { fb: {}, tex: {}, w: 2, h: 1, colorDepth: 16, busy: true };
      PM.beginEval = vi.fn(); PM.scope = { push: vi.fn(), pop: vi.fn() };
      Object.assign(PM.GL, { gl, canvas: { width: 2, height: 1 }, renderProject: vi.fn(() => scene) });
      expect(() => PM.GL.createAlphaReadback(2, 1).begin(0)).toThrow('Queue failed');
      expect(scene.busy).toBe(false); expect(PM.scope.pop).toHaveBeenCalledOnce();
      expect(gl.readPixels).not.toHaveBeenCalled();
      expect(gl.bindFramebuffer).toHaveBeenLastCalledWith(gl.FRAMEBUFFER, null);
    } finally { factory.mockRestore(); }
  });

  it.each([8, 16, 32])('uses the private half reader only for an actual 16-bit target: %s-bit', depth => {
    const PM = compositorRegistry(), converted = new Uint8Array(8).fill(9), reader = vi.fn(() => converted);
    const factory = vi.spyOn(halfReadback, 'createHalfReadback').mockImplementation((_gl, _w, _h, _generation, render) => ({
      capture: (time, options = {}) => render(time, options, reader), dispose() {},
    }));
    try {
      const gl = { FRAMEBUFFER: 1, RGBA: 2, FLOAT: 3, UNSIGNED_BYTE: 4, bindFramebuffer: vi.fn(), viewport: vi.fn(),
        readPixels: vi.fn((_x: number, _y: number, _w: number, _h: number, _format: number, type: number, pixels: Float32Array | Uint8Array) => pixels.fill(type === 3 ? .5 : 2)) };
      const scene = { fb: {}, tex: {}, w: 2, h: 1, colorDepth: depth, busy: true };
      PM.beginEval = vi.fn(); PM.scope = { push: vi.fn(), pop: vi.fn() };
      Object.assign(PM.GL, { gl, canvas: { width: 2, height: 1 }, colorDepth: depth, renderProject: vi.fn(() => scene) });
      const result = PM.GL.createHalfReadback(2, 1).capture(0, { transparent: true });
      expect(result).toEqual(depth === 16 ? converted : new Uint8Array(8).fill(depth === 8 ? 2 : 128));
      expect(reader).toHaveBeenCalledTimes(depth === 16 ? 1 : 0);
      expect(gl.readPixels).toHaveBeenCalledTimes(depth === 16 ? 0 : 1);
      expect(PM.scope.pop).toHaveBeenCalledOnce(); expect(scene.busy).toBe(false);
      expect(gl.bindFramebuffer).toHaveBeenLastCalledWith(gl.FRAMEBUFFER, null);
      if (depth === 16) {
        reader.mockReturnValueOnce(null as any);
        expect(PM.GL.createHalfReadback(2, 1).capture(1)).toEqual(new Uint8Array(8).fill(128));
        expect(PM.GL.renderProject).toHaveBeenCalledTimes(2); // One render per capture, including fallback.
        expect(gl.readPixels).toHaveBeenCalledOnce();
        expect(PM.GL.renderToPixels(2, 2, 1)).toEqual(new Uint8Array(8).fill(128));
        expect(reader).toHaveBeenCalledTimes(2); // Public synchronous capture keeps its established path.
      }
    } finally { factory.mockRestore(); }
  });

  it('invalidates half-readback sessions on every GL.init attempt even with the same context object', () => {
    const PM = compositorRegistry(), gl = {};
    const factory = vi.spyOn(halfReadback, 'createHalfReadback').mockReturnValue(null);
    try {
      PM.GL.gl = gl; PM.GL.createHalfReadback(2, 1);
      const first = factory.mock.calls[0]!, valid = first[5];
      expect(valid()).toBe(true);
      PM.GL.init({ getContext: () => null }, { quiet: true });
      PM.GL.gl = gl; PM.GL.createHalfReadback(2, 1);
      expect(valid()).toBe(false);
      expect(factory.mock.calls[1]![3]).not.toBe(first[3]);
      expect(factory.mock.calls[1]![5]()).toBe(true);
    } finally { factory.mockRestore(); }
  });

  it.each([8,16,32])('uses the private Float32 reader only for an actual 32-bit target and preserves fallback: %s-bit', depth => {
    const PM=compositorRegistry(),converted=new Uint8Array(8).fill(9),reader=vi.fn(()=>converted);
    const factory=vi.spyOn(halfReadback,'createFloatReadback').mockImplementation((_gl,_w,_h,_generation,render)=>({
      capture:(time,options={})=>render(time,options,reader),dispose(){},
    }));
    try{
      const gl={FRAMEBUFFER:1,RGBA:2,FLOAT:3,UNSIGNED_BYTE:4,bindFramebuffer:vi.fn(),viewport:vi.fn(),
        readPixels:vi.fn((_x:number,_y:number,_w:number,_h:number,_format:number,type:number,pixels:Float32Array|Uint8Array)=>pixels.fill(type===3?.5:2))};
      const scene={fb:{},tex:{},w:2,h:1,colorDepth:depth,busy:true};
      PM.beginEval=vi.fn();PM.scope={push:vi.fn(),pop:vi.fn()};
      Object.assign(PM.GL,{gl,canvas:{width:2,height:1},colorDepth:depth,renderProject:vi.fn(()=>scene)});
      expect(PM.GL.createFloatReadback(2,1).capture(0,{transparent:true})).toEqual(depth===32?converted:new Uint8Array(8).fill(depth===8?2:128));
      expect(reader).toHaveBeenCalledTimes(depth===32?1:0);expect(gl.readPixels).toHaveBeenCalledTimes(depth===32?0:1);
      expect(PM.scope.pop).toHaveBeenCalledOnce();expect(scene.busy).toBe(false);
      expect(gl.bindFramebuffer).toHaveBeenLastCalledWith(gl.FRAMEBUFFER,null);
      if(depth===32){
        reader.mockReturnValueOnce(null as any);
        expect(PM.GL.createFloatReadback(2,1).capture(1)).toEqual(new Uint8Array(8).fill(128));
        expect(PM.GL.renderProject).toHaveBeenCalledTimes(2);expect(gl.readPixels).toHaveBeenCalledOnce();
        expect(PM.GL.renderToPixels(2,2,1)).toEqual(new Uint8Array(8).fill(128));
        expect(reader).toHaveBeenCalledTimes(2); // Public synchronous capture still uses FLOAT readback.
      }
    }finally{factory.mockRestore();}
  });

  it('invalidates Float32 readback on init even when the context object is reused, and cleans capture failures', () => {
    const PM=compositorRegistry(),reader=vi.fn(()=>{throw new Error('Copy failed');});
    const factory=vi.spyOn(halfReadback,'createFloatReadback').mockImplementation((_gl,_w,_h,_generation,render)=>({capture:(time,options={})=>render(time,options,reader),dispose(){}}));
    try{
      const gl={FRAMEBUFFER:1,bindFramebuffer:vi.fn(),viewport:vi.fn(),readPixels:vi.fn()};
      const scene={fb:{},tex:{},w:2,h:1,colorDepth:32,busy:true};
      PM.beginEval=vi.fn();PM.scope={push:vi.fn(),pop:vi.fn()};
      Object.assign(PM.GL,{gl,canvas:{width:2,height:1},colorDepth:32,renderProject:vi.fn(()=>scene)});
      const session=PM.GL.createFloatReadback(2,1),first=factory.mock.calls[0]!,valid=first[5];
      expect(()=>session.capture(0)).toThrow('Copy failed');expect(PM.scope.pop).toHaveBeenCalledOnce();expect(scene.busy).toBe(false);
      expect(gl.readPixels).not.toHaveBeenCalled();expect(gl.bindFramebuffer).toHaveBeenLastCalledWith(gl.FRAMEBUFFER,null);
      expect(valid()).toBe(true);PM.GL.init({getContext:()=>null},{quiet:true});PM.GL.gl=gl;
      PM.GL.createFloatReadback(2,1);expect(valid()).toBe(false);expect(factory.mock.calls[1]![3]).not.toBe(first[3]);expect(factory.mock.calls[1]![5]()).toBe(true);
    }finally{factory.mockRestore();}
  });

  it.each(['framebuffer', 'renderbuffer', 'storage'] as const)('aborts asynchronous capture on opaque target allocation failure: %s', failure => {
    const PM = compositorRegistry();
    const framebuffer = {}, renderbuffer = {}, buffer = {}, scene = { fb: {}, tex: {}, w: 2, h: 1, colorDepth: 8, busy: true };
    const gl = {
      PIXEL_PACK_BUFFER: 1, PIXEL_PACK_BUFFER_BINDING: 2, STREAM_READ: 3, BUFFER_SIZE: 4,
      FRAMEBUFFER: 5, RENDERBUFFER: 6, FRAMEBUFFER_COMPLETE: 7, RGBA8: 8, COLOR_ATTACHMENT0: 9,
      isContextLost: () => false, getParameter: () => null, createBuffer: () => buffer,
      bindBuffer: vi.fn(), bufferData: vi.fn(), getBufferParameter: () => 8, deleteBuffer: vi.fn(),
      createFramebuffer: () => failure === 'framebuffer' ? null : framebuffer,
      createRenderbuffer: () => failure === 'renderbuffer' ? null : renderbuffer,
      bindFramebuffer: vi.fn(), viewport: vi.fn(), bindRenderbuffer: vi.fn(),
      renderbufferStorage: vi.fn(), framebufferRenderbuffer: vi.fn(), checkFramebufferStatus: () => 0,
      deleteFramebuffer: vi.fn(), deleteRenderbuffer: vi.fn(),
      fenceSync: vi.fn(), clientWaitSync: vi.fn(), getBufferSubData: vi.fn(), readPixels: vi.fn(),
    };
    PM.beginEval = vi.fn(); PM.scope = { push: vi.fn(), pop: vi.fn() };
    Object.assign(PM.GL, { gl, canvas: { width: 2, height: 1 }, pool: [scene], colorDepth: 8, renderProject: () => scene });
    const session = PM.GL.createOpaqueReadback(2, 1);
    expect(() => session.begin(0, { mblur: false })).toThrow('GPU');
    expect(gl.readPixels).not.toHaveBeenCalled(); expect(gl.fenceSync).not.toHaveBeenCalled();
    expect(PM.scope.pop).toHaveBeenCalledOnce(); expect(scene.busy).toBe(false);
    expect(gl.bindFramebuffer).toHaveBeenLastCalledWith(gl.FRAMEBUFFER, null);
    expect(gl.bindRenderbuffer).toHaveBeenLastCalledWith(gl.RENDERBUFFER, null);
    expect(gl.deleteFramebuffer).toHaveBeenCalledWith(failure === 'framebuffer' ? null : framebuffer);
    expect(gl.deleteRenderbuffer).toHaveBeenCalledWith(failure === 'renderbuffer' ? null : renderbuffer);
    session.dispose(); expect(gl.deleteBuffer).toHaveBeenCalledWith(buffer);
  });

  it('gives transparent null objects a compact canvas hit target', () => {
    const PM = compositorRegistry();
    expect(PM.GL.bounds({ type: 'null', d: { w: 100, h: 100 } }, 0)).toEqual({
      x0: -0, y0: -0, x1: 100, y1: 100, w: 100, h: 100, ax: 0, ay: 0
    });
  });

  it('measures primitive selection bounds without allocating a source bitmap', () => {
    const PM = compositorRegistry();
    PM.raster = vi.fn(() => { throw new Error('Bounds must not rasterize a primitive'); });
    const bounds = PM.GL.bounds({ type: 'shape', d: { w: 3000, h: 2000, stroke: 10 } }, 0);
    expect(bounds).toEqual({ x0: -1505, y0: -1005, x1: 1505, y1: 1005, w: 3010, h: 2010, ax: 1509, ay: 1009 });
    expect(PM.raster).not.toHaveBeenCalled();
  });

  it('continuously rasterizes editable vector sources at their displayed scale', () => {
    expect(continuousRasterScale([4, 0, 0, 4, 0, 0], 1)).toBe(4);
    expect(continuousRasterScale([0, 3, -3, 0, 0, 0], 1)).toBe(3);
    expect(continuousRasterScale([4, 0, 0, 4, 0, 0], .5)).toBe(2);
    expect(continuousRasterScale([.25, 0, 0, .25, 0, 0], 1)).toBe(.25);
    expect(continuousRasterScale([32, 0, 0, 32, 0, 0], 1)).toBe(32);
    expect(continuousRasterScale([64, 0, 0, 64, 0, 0], 1)).toBe(32);
  });

  it('sizes SVG backing textures from their displayed box without changing source aspect ratio', () => {
    expect(svgRasterDimensions(100, 50, 400, 200, [2, 0, 0, 2, 0, 0])).toEqual({
      width: 800,
      height: 400,
      scale: 8,
    });
    expect(svgRasterDimensions(100, 50, 400, 400, [1, 0, 0, 1, 0, 0])).toEqual({
      width: 800,
      height: 400,
      scale: 8,
    });
    const capped = svgRasterDimensions(100, 50, 20_000, 10_000, [1, 0, 0, 1, 0, 0]);
    expect(capped).toEqual({ width: 8192, height: 4096, scale: 81.92 });
  });

  it('invalidates a paused seek as soon as the decoded frame is available', () => {
    const video = new EventTarget() as any;
    video.requestVideoFrameCallback = vi.fn();
    const invalidated = vi.fn();
    const state = trackPresentedVideoFrames(video, invalidated);
    video.dispatchEvent(new Event('seeked'));
    expect(state.version).toBe(1);
    expect(invalidated).toHaveBeenCalledOnce();
  });

  it('versions textures from each frame the browser presents', () => {
    const callbacks: Array<() => void> = [];
    const invalidated = vi.fn();
    const video = {
      requestVideoFrameCallback(callback: () => void) {
        callbacks.push(callback);
        return callbacks.length;
      },
    };

    const state = trackPresentedVideoFrames(video, invalidated);
    expect(state).toMatchObject({ version: 0, supported: true });
    callbacks.shift()!();
    expect(state.version).toBe(1);
    expect(invalidated).toHaveBeenCalledOnce();
    expect(callbacks).toHaveLength(1);
  });

  it('uses tight text selection bounds and keeps the background fill shader path', () => {
    const PM = compositorRegistry();
    const text = { type: 'text', d: {} };
    const bounds = PM.GL.bounds(text, 0);

    expect(bounds.h).toBeLessThan(300 * .5);
    expect(bounds.w).toBeLessThan(500);
    expect(bounds.w).toBeGreaterThan(350);
    expect(Math.abs(bounds.x0 + bounds.x1)).toBeLessThan(1);
    expect(bounds.ax).toBe(250);
    expect(bounds.ay).toBe(150);
    expect(String(PM.GL.renderProject)).toContain('PM.FRAG_BACKGROUND_FILL');
  });

  it('routes adjustment layers through the accumulated lower-layer image', () => {
    const PM = compositorRegistry();
    const source = String(PM.GL.renderProject);

    expect(source).toMatch(/L\.type === ["']adjustment["']/);
    expect(source).toContain('compositeAdjustment(L, T, acc, W, H, alpha, hasMasks, blend)');
  });

  it('builds nested compositing levels and retains a group containing the solo layer', () => {
    const outer = { id: 'outer', type: 'group', group: null };
    const child = { id: 'child', type: 'shape', group: 'outer' };
    const inner = { id: 'inner', type: 'group', group: 'outer' };
    const solo = { id: 'solo', type: 'text', group: 'inner', solo: true };
    const top = { id: 'top', type: 'shape', group: null };
    const layers = [outer, child, inner, solo, top];
    const ancestors = (layer: any) => layer === solo ? [inner, outer]
      : layer === child || layer === inner ? [outer] : [];

    expect(compositingLevel(layers, null)).toEqual([outer, top]);
    expect(compositingLevel(layers, 'outer')).toEqual([child, inner]);
    expect(compositingLevel(layers, 'inner')).toEqual([solo]);
    expect(groupContainsSolo(outer, layers, ancestors)).toBe(true);
    expect(groupContainsSolo(inner, layers, ancestors)).toBe(true);
    expect(groupContainsSolo(top, layers, ancestors)).toBe(false);
  });

  it('renders group contents into their own effect, mask, blend, matte, motion-blur and transition pass', () => {
    const PM = compositorRegistry();
    const source = String(PM.GL.renderProject);

    expect(source).toMatch(/L\.type !== ["']group["']/);
    expect(source).toContain('groupParent: L.id');
    expect(source).toMatch(/runEffects\(L, T, res, W, H, acc, opt.inputPrefix\)/);
    expect(source).toMatch(/applyMasks\(L, T, res, W, H\)/);
    expect(source).toMatch(/applyTrackMatte\(L, T, res, W, H/);
    expect(source).toMatch(/compositeAlpha = L\.type === ["']group["'] \? alpha : 1/);
  });
});

describe('effect and transition parameter uniforms', () => {
  /* Kernel-generated shaders declare `u_<param>`; legacy raw shaders declare
     the positional `u_p<i>` / `u_c<i>` names. Bind by name first, fall back. */
  it('prefers the named uniform when the program declares it', () => {
    expect(paramUniformName({ k: 'amount' }, 0, true)).toBe('u_amount');
    expect(paramUniformName({ k: 'shadow', type: 'color' }, 4, true)).toBe('u_shadow');
  });

  it('falls back to the positional name for legacy raw shaders', () => {
    expect(paramUniformName({ k: 'amount' }, 0, false)).toBe('u_p0');
    expect(paramUniformName({ k: 'angle' }, 1, false)).toBe('u_p1');
    expect(paramUniformName({ k: 'shadow', type: 'color' }, 4, false)).toBe('u_c4');
    expect(paramUniformName({ k: 'on', type: 'toggle' }, 2, false)).toBe('u_p2');
  });
});

describe('compositor effect fast path', () => {
  it('ignores enabled missing-effect placeholders when choosing the fast path', () => {
    expect(hasRenderableEffects([{ on: true, missing: true }])).toBe(false);
    expect(hasRenderableEffects([
      { on: true, missing: true },
      { on: false, missing: false }
    ])).toBe(false);
    expect(hasRenderableEffects([
      { on: true, missing: true },
      { on: true, missing: false }
    ])).toBe(true);
  });

  it('starts a fresh evaluator window for pixel export and defaults absent effect channels', () => {
    const PM = compositorRegistry();
    PM.evP = () => 42;
    expect(String(PM.GL.renderToPixels)).toContain('PM.beginEval(T)');
    expect(effectParamValue(PM, {}, { p: {} }, { k: 'amount', def: 12 }, 0)).toBe(12);
    expect(effectParamValue(PM, {}, { p: { amount: { v: 42 } } }, { k: 'amount', def: 12 }, 0)).toBe(42);
  });
});

describe('linear compositing traversal', () => {
  it('matches recursive stack order with nested boundaries, missing groups and edits', () => {
    const layers: any[] = [
      { id: 'top', type: 'shape' },
      { id: 'outer', type: 'group' },
      { id: 'a', type: 'shape', group: 'outer' },
      { id: 'inner', type: 'group', group: 'outer' },
      { id: 'b', type: 'shape', group: 'inner' },
      { id: 'c', type: 'shape', group: 'outer' },
      { id: 'orphan', type: 'shape', group: 'missing' },
      { id: 'bottom', type: 'shape', group: '' },
    ];
    const reference = (parent: string | null, boundaries: Set<string>): any[] =>
      compositingLevel(layers, parent).flatMap(layer => layer.type === 'group' && !boundaries.has(layer.id)
        ? reference(layer.id, boundaries) : [layer]);
    for (const edit of [() => {}, () => { layers[4].group = 'outer'; }, () => { layers.reverse(); }]) {
      edit();
      for (const parent of [null, 'outer', 'inner']) {
        for (const isolated of [[], ['inner'], ['outer'], ['outer', 'inner']]) {
          const boundaries = new Set(isolated);
          expect(compositingLayers(layers, parent, layer => boundaries.has(layer.id))).toEqual(reference(parent, boundaries));
        }
      }
    }
  });

  it('reads membership a linear number of times in a heavily grouped scene', () => {
    let reads = 0;
    const layers = Array.from({ length: 2000 }, (_, i) => [
      { id: `group-${i}`, type: 'group', get group() { reads++; return null; } },
      { id: `child-${i}`, type: 'shape', get group() { reads++; return `group-${i}`; } },
    ]).flat();
    const result = compositingLayers(layers, null, () => false);
    expect(result.map(layer => layer.id)).toEqual(Array.from({ length: 2000 }, (_, i) => `child-${i}`));
    expect(reads).toBeLessThanOrEqual(layers.length * 2);
  });
});
