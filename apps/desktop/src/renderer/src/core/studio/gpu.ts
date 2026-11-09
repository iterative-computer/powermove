import traceSource from '../../../../../../../engine/crates/pm-studio/src/shaders/trace.wgsl?raw';
import postSource from '../../../../../../../engine/crates/pm-studio/src/shaders/post.wgsl?raw';
import presentSource from '../../../../../../../engine/crates/pm-studio/src/shaders/present.wgsl?raw';
import aovSource from '../../../../../../../engine/crates/pm-studio/src/shaders/aov.wgsl?raw';
import denoiseSource from '../../../../../../../engine/crates/pm-studio/src/shaders/denoise.wgsl?raw';
import { MAP_SIZE, type StudioScene } from './extract';

/*
 * The app's WebGPU host for the Studio tracer. Same WGSL as the Rust engine
 * (engine/crates/pm-studio), so the editor, exports and the native engine
 * agree on every pixel's look. One trace per (scene, camera, size) serves
 * every 3D layer; `present` copies one layer's pixels into its own canvas,
 * which the WebGL compositor uploads like any other source.
 */

export type StudioCamera = {
  position: [number, number, number];
  forward: [number, number, number];
  right: [number, number, number];
  up: [number, number, number];
  tanY: number;
  aspect: number;
  lensRadius: number;
  focus: number;
};

export type StudioView = {
  camera: StudioCamera;
  exposure: number;
  /** Display-referred backdrop and whether the void shows it. */
  backdrop: [number, number, number];
  backdropOpaque: boolean;
  shadowCatcher: boolean;
  /** The environment's shadows switch (off: lights are never blocked). */
  shadows: boolean;
  /** Height of the shadow-catching floor (world y). */
  floorY: number;
  environment: { zenith: [number, number, number]; horizon: [number, number, number]; ground: [number, number, number] };
};

/** post.wgsl's Post: three vec4s, then two cameras of four vec4s. */
const POST_BYTES = 48 + 128;
/** denoise.wgsl's Den: dims, k. */
const DENOISE_BYTES = 32;
/** Final's denoiser: à-trous passes (steps 1…16) and how many standard deviations its edges allow. */
const DENOISE_PASSES = 5;
const DENOISE_SIGMA = 4;
/** trace.wgsl's Params: 13 vec4. */
const PARAMS_BYTES = 208;
/** Photon grid slots (8 words each); must match trace.wgsl's GRID_CELLS. */
const GRID_SLOTS = 1 << 19;
/** Photons traced per sample batch when a scene has caustics. */
const PHOTONS_PER_PASS = 1 << 17;
/** Global switches (tests compare a render with and without caustics, or Final's denoiser). */
export const studioOptions = { caustics: true, denoise: true };

const GPU: any = typeof navigator !== 'undefined' ? (navigator as any).gpu : undefined;
const U = (globalThis as any).GPUBufferUsage, T = (globalThis as any).GPUTextureUsage, S = (globalThis as any).GPUShaderStage;

let devicePromise: Promise<any> | null = null;
let device: any = null;
let failure: string | null = null;

/** Starts WebGPU once; Studio renders after it resolves (the old renderer covers the gap). */
export function initStudioGpu(): Promise<any> {
  if (!GPU) { failure = 'WebGPU is not available'; return Promise.resolve(null); }
  devicePromise ??= (async () => {
    try {
      const adapter = await GPU.requestAdapter({ powerPreference: 'high-performance' });
      if (!adapter) throw new Error('No WebGPU adapter');
      const need = 10;
      if (adapter.limits.maxStorageBuffersPerShaderStage < need) throw new Error('Too few storage buffers per stage');
      device = await adapter.requestDevice({
        requiredLimits: {
          maxStorageBuffersPerShaderStage: need,
          maxStorageBufferBindingSize: adapter.limits.maxStorageBufferBindingSize,
          maxBufferSize: adapter.limits.maxBufferSize,
        },
      });
      device.lost.then((info: any) => { failure = `WebGPU device lost: ${info.message}`; device = null; devicePromise = null; });
      return device;
    } catch (error) {
      failure = String(error);
      return null;
    }
  })();
  return devicePromise;
}
export const studioGpuReady = () => !!device;
/** The largest buffer one binding can hold (bytes): bounds how many triangles Studio can trace. */
export const studioMaxBindingBytes = (): number => device?.limits?.maxStorageBufferBindingSize ?? 0;
export const studioGpuFailure = () => failure;

type Pipelines = { trace: any; photons: any; resolve: any; atrous: any; compose: any; pack: any; temporal: any; present: any; traceLayout: any; postLayout: any; presentLayout: any; sampler: any };
let pipelines: Pipelines | null = null;

function storage(binding: number, readOnly: boolean, visibility = S.COMPUTE) {
  return { binding, visibility, buffer: { type: readOnly ? 'read-only-storage' : 'storage' } };
}
function uniform(binding: number, visibility = S.COMPUTE) {
  return { binding, visibility, buffer: { type: 'uniform' } };
}

function build(): Pipelines {
  if (pipelines) return pipelines;
  const traceLayout = device.createBindGroupLayout({
    label: 'studio trace',
    entries: [
      uniform(0), storage(1, true), storage(2, true), storage(3, true), storage(4, true),
      { binding: 5, visibility: S.COMPUTE, texture: { sampleType: 'float', viewDimension: '2d-array' } },
      { binding: 6, visibility: S.COMPUTE, sampler: { type: 'filtering' } },
      storage(7, false), storage(8, false), storage(9, false), storage(10, false), storage(11, true), storage(12, false),
    ],
  });
  const postLayout = device.createBindGroupLayout({
    label: 'studio post',
    entries: [uniform(0), storage(1, true), storage(2, true), storage(3, true), storage(4, true), storage(5, false), storage(6, false), storage(7, true), storage(8, true), storage(9, false)],
  });
  const presentLayout = device.createBindGroupLayout({
    label: 'studio present',
    entries: [uniform(0, S.FRAGMENT), storage(1, true, S.FRAGMENT), storage(2, true, S.FRAGMENT)],
  });
  const trace = device.createShaderModule({ label: 'trace.wgsl', code: traceSource });
  const post = device.createShaderModule({ label: 'post.wgsl', code: postSource });
  const present = device.createShaderModule({ label: 'present.wgsl', code: presentSource });
  const compute = (module: any, layout: any, entryPoint: string) => device.createComputePipeline({
    label: entryPoint, layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }), compute: { module, entryPoint },
  });
  pipelines = {
    trace: compute(trace, traceLayout, 'main'),
    photons: compute(trace, traceLayout, 'photons'),
    resolve: compute(post, postLayout, 'resolve'),
    atrous: compute(post, postLayout, 'atrous'),
    compose: compute(post, postLayout, 'compose'),
    temporal: compute(post, postLayout, 'temporal'),
    pack: compute(post, postLayout, 'pack'),
    present: device.createRenderPipeline({
      label: 'present',
      layout: device.createPipelineLayout({ bindGroupLayouts: [presentLayout] }),
      vertex: { module: present, entryPoint: 'vs' },
      fragment: { module: present, entryPoint: 'fs', targets: [{ format: GPU.getPreferredCanvasFormat() }] },
      primitive: { topology: 'triangle-list' },
    }),
    traceLayout, postLayout, presentLayout,
    sampler: device.createSampler({ magFilter: 'linear', minFilter: 'linear', addressModeU: 'repeat', addressModeV: 'repeat' }),
  };
  return pipelines;
}

type DenoisePipelines = { aovs: any; demod: any; step: any; finish: any; aovLayout: any; layout: any };
let denoisePipelines: DenoisePipelines | null = null;

/** Final's denoiser (built the first time a Final or still frame needs it). */
function buildDenoise(): DenoisePipelines {
  if (denoisePipelines) return denoisePipelines;
  // The denoiser's guides read only the scene (a pipeline may bind 10 storage buffers; the trace uses all of them).
  const aovLayout = device.createBindGroupLayout({
    label: 'studio aov',
    entries: [
      uniform(0), storage(1, true), storage(2, true), storage(3, true), storage(4, true),
      { binding: 5, visibility: S.COMPUTE, texture: { sampleType: 'float', viewDimension: '2d-array' } },
      { binding: 6, visibility: S.COMPUTE, sampler: { type: 'filtering' } },
      storage(11, true), storage(13, false),
    ],
  });
  const layout = device.createBindGroupLayout({
    label: 'studio denoise',
    entries: [uniform(0), storage(2, true), storage(3, true), storage(4, false), storage(5, true), storage(6, false), storage(7, true), storage(8, true), storage(9, true)],
  });
  const aov = device.createShaderModule({ label: 'aov.wgsl', code: `${traceSource}\n@group(0) @binding(13) var<storage, read_write> aov: array<vec4f>;\n${aovSource}` });
  const denoise = device.createShaderModule({ label: 'denoise.wgsl', code: denoiseSource });
  const compute = (module: any, l: any, entryPoint: string) => device.createComputePipeline({
    label: entryPoint, layout: device.createPipelineLayout({ bindGroupLayouts: [l] }), compute: { module, entryPoint },
  });
  denoisePipelines = {
    aovs: compute(aov, aovLayout, 'aovs'), demod: compute(denoise, layout, 'demod'),
    step: compute(denoise, layout, 'atrous_step'), finish: compute(denoise, layout, 'finish'), aovLayout, layout,
  };
  return denoisePipelines;
}

/** One render target size: the accumulation, denoise and per-layer canvases. */
export class StudioTarget {
  readonly width: number;
  readonly height: number;
  samples = 0;
  private frame = 0;
  private params: any;
  private postParams: any[];
  private presentParams = new Map<string, any>();
  private accum: any; private bgsum: any; private gbuf: any; private ebuf: any;
  /** Caustic photons: allocated the first time a scene can focus light. */
  private grid: any; private gridSize = 0;
  private caustics = false;
  private photonsEmitted = 0;
  private bounds: [number, number, number, number] = [0, 0, 0, 1];
  private display: any[];
  private packed: any;
  private sceneBuffers: any[] = [];
  private mapTexture: any = null;
  private traceGroup: any = null;
  private lightCount = 0;
  private base = new ArrayBuffer(PARAMS_BYTES);
  private canvases = new Map<string, { canvas: HTMLCanvasElement; context: any }>();
  view: StudioView | null = null;
  live = false;

  constructor(width: number, height: number) {
    this.width = width; this.height = height;
    const p = build();
    void p;
    const px = width * height;
    const buf = (label: string, size: number, usage = U.STORAGE | U.COPY_DST | U.COPY_SRC) => device.createBuffer({ label, size: Math.max(16, size), usage });
    this.params = buf('studio params', PARAMS_BYTES, U.UNIFORM | U.COPY_DST);
    this.postParams = Array.from({ length: 10 }, () => buf('studio post params', POST_BYTES, U.UNIFORM | U.COPY_DST));
    // Live's history, swapped each frame: one read, one written.
    this.history = [buf('history a', px * 32), buf('history b', px * 32)];
    this.accum = buf('accum', px * 16); this.bgsum = buf('bgsum', px * 16); this.gbuf = buf('gbuf', px * 16);
    this.ebuf = buf('ebuf', px * 16); this.grid = buf('photon grid', 32); this.gridSize = 32;
    this.display = [buf('display a', px * 16), buf('display b', px * 16)];
    this.packed = buf('packed', px * 4);
  }

  destroy(): void {
    for (const b of [this.params, ...this.postParams, ...this.presentParams.values(), this.accum, this.bgsum, this.gbuf, this.ebuf, this.grid, ...this.display, this.packed, ...this.history, ...this.sceneBuffers, ...this.denoiseBuffers()]) b?.destroy?.();
    this.mapTexture?.destroy?.();
  }

  setScene(scene: StudioScene, maps: (HTMLCanvasElement | OffscreenCanvas | Uint8Array)[]): void {
    const bytesOf = (data: ArrayBufferView | ArrayBuffer) => data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    const init = (label: string, data: ArrayBufferView | ArrayBuffer) => {
      const bytes = bytesOf(data);
      const b = device.createBuffer({ label, size: Math.max(64, Math.ceil(bytes.byteLength / 4) * 4), usage: U.STORAGE | U.COPY_DST });
      device.queue.writeBuffer(b, 0, bytes);
      return b;
    };
    const sameGeometry = this.geometryKey === scene.geometryKey && this.sceneBuffers.length === 5;
    if (sameGeometry) {
      // Only things moved: rewrite the top level in place; materials and scene data are small.
      device.queue.writeBuffer(this.sceneBuffers[1], 0, bytesOf(scene.tlasNodes));
      this.sceneBuffers[2].destroy(); this.sceneBuffers[2] = init('materials', scene.materials);
      this.sceneBuffers[3].destroy(); this.sceneBuffers[3] = init('scene data', scene.sdata);
    } else {
      for (const b of this.sceneBuffers) b.destroy();
      const nodes = new Uint8Array(scene.tlasNodes.byteLength + scene.blasNodes.byteLength);
      nodes.set(new Uint8Array(scene.tlasNodes), 0);
      nodes.set(new Uint8Array(scene.blasNodes), scene.tlasNodes.byteLength);
      this.sceneBuffers = [
        init('tris', scene.tris), init('nodes', nodes), init('materials', scene.materials),
        init('scene data', scene.sdata), init('positions', scene.triangles ? scene.positions : new Float32Array(4)),
      ];
      this.geometryKey = scene.geometryKey;
    }
    this.lightCount = scene.lightCount;
    this.instanceCount = scene.instanceCount;
    // Caustics only where something can focus light: glass or polished metal.
    const m = scene.materials;
    this.caustics = false;
    for (let i = 0; i + 24 <= m.length; i += 24) {
      if (m[i + 7]! * (1 - m[i + 3]!) > 0.5 || (m[i + 3]! > 0.5 && m[i + 4]! < 0.25)) { this.caustics = true; break; }
    }
    if (!scene.bounds.isEmpty()) {
      const { min: lo, max: hi } = scene.bounds;
      this.bounds = [(lo.x + hi.x) / 2, (lo.y + hi.y) / 2, (lo.z + hi.z) / 2, Math.max(1e-3, Math.hypot(hi.x - lo.x, hi.y - lo.y, hi.z - lo.z) / 2)];
    }
    const layers = Math.max(1, maps.length);
    if (!this.mapTexture || this.mapLayers !== layers) {
      this.mapTexture?.destroy?.();
      this.mapTexture = device.createTexture({
        label: 'studio maps', size: [MAP_SIZE, MAP_SIZE, layers], format: 'rgba8unorm',
        usage: T.TEXTURE_BINDING | T.COPY_DST | T.RENDER_ATTACHMENT,
      });
      this.mapLayers = layers;
    }
    // Maps change with the scene (a live screen's frame, a new texture): always write them.
    maps.forEach((source, layer) => this.writeMap(layer, source));
    this.traceGroup = null;
    this.samples = 0;
  }
  private geometryKey = '';
  private instanceCount = 0;
  private mapLayers = 0;

  /** Replace one texture layer (a live screen's new frame). */
  writeMap(layer: number, source: HTMLCanvasElement | OffscreenCanvas | Uint8Array): void {
    if (!this.mapTexture) return;
    if (source instanceof Uint8Array) {
      device.queue.writeTexture({ texture: this.mapTexture, origin: [0, 0, layer] }, source, { bytesPerRow: MAP_SIZE * 4, rowsPerImage: MAP_SIZE }, [MAP_SIZE, MAP_SIZE, 1]);
    } else {
      device.queue.copyExternalImageToTexture({ source }, { texture: this.mapTexture, origin: [0, 0, layer] }, [MAP_SIZE, MAP_SIZE]);
    }
  }

  setView(view: StudioView): void {
    this.view = view;
    const c = view.camera;
    const f = new Float32Array(this.base), u = new Uint32Array(this.base);
    f.set([...c.position, c.lensRadius], 0);
    f.set([...c.forward, c.focus], 4);
    f.set([...c.right, c.tanY * c.aspect], 8);
    f.set([...c.up, c.tanY], 12);
    f.set([...view.backdrop, view.shadowCatcher ? 1 : 0], 16);
    f.set([...view.environment.zenith, view.exposure], 20);
    f.set([...view.environment.horizon, 0.82], 24);
    u.set([this.width, this.height, 0, 1], 28);
    u.set([this.lightCount, 6, 1, 64], 32);
    u.set([0, 0, 0, 0], 36);
    f.set([...view.environment.ground, view.floorY], 40);
    f.set([0, 0, 0, 0], 44);
    f.set(this.bounds, 48);
    this.samples = 0;
  }

  /**
   * Queue `spp` samples (`reset` starts over). Returns without waiting.
   * `salt` seeds Stills and Final: the same salt (a frame's time) always gives
   * the same pixels, and each frame of a film gets its own noise instead of
   * one pattern stuck to the screen.
   */
  trace(spp: number, bounces: number, reset: boolean, clamp: number, salt = 0): void {
    if (!this.view) return;
    const p = build();
    this.traceGroup ??= this.groupWith(this.params);
    // Live keeps counting, so frames blended over time average out.
    if (reset && !this.live) this.frame = Math.imul(salt >>> 0, 4099) >>> 0;
    if (reset) this.photonsEmitted = 0;
    // Caustics: photons from the lights through glass and mirrors (Refine, Final, stills).
    const photons = studioOptions.caustics && this.caustics && !this.live && this.lightCount > 0;
    if (photons && this.gridSize < GRID_SLOTS * 32) {
      this.grid.destroy();
      this.grid = device.createBuffer({ label: 'photon grid', size: GRID_SLOTS * 32, usage: U.STORAGE | U.COPY_DST });
      this.gridSize = GRID_SLOTS * 32;
      this.traceGroup = this.groupWith(this.params);
    }
    let left = spp, first = true;
    // One dispatch per submission: writeBuffer and submit run in queue order,
    // so each dispatch sees its own seed and reset flag.
    while (left > 0) {
      const n = Math.min(4, left);
      this.frame = (this.frame + 1) >>> 0;
      const u = new Uint32Array(this.base);
      u.set([this.width, this.height, this.frame, n], 28);
      u.set([this.lightCount, bounces, reset && first ? 1 : 0, clamp], 32);
      u.set([this.live ? 1 : 0, this.view.shadows === false ? 1 : 0, this.instanceCount, 0], 36);
      const f = new Float32Array(this.base);
      f.set([this.bounds[3] / 160, this.photonsEmitted, 0, 0], 44);
      f.set(this.bounds, 48);
      if (photons) {
        // Photons first, in their own submission, so this batch's camera paths see them.
        const pe = device.createCommandEncoder();
        if (reset && first) pe.clearBuffer(this.grid);
        f.set([this.bounds[3] / 160, this.photonsEmitted, 1, PHOTONS_PER_PASS], 44);
        device.queue.writeBuffer(this.params, 0, this.base);
        const pp = pe.beginComputePass();
        pp.setPipeline(p.photons);
        pp.setBindGroup(0, this.traceGroup);
        pp.dispatchWorkgroups(Math.ceil(PHOTONS_PER_PASS / 64));
        pp.end();
        device.queue.submit([pe.finish()]);
        this.photonsEmitted += PHOTONS_PER_PASS;
        f[45] = this.photonsEmitted;
      }
      device.queue.writeBuffer(this.params, 0, this.base);
      const enc = device.createCommandEncoder();
      const pass = enc.beginComputePass();
      pass.setPipeline(p.trace);
      pass.setBindGroup(0, this.traceGroup);
      pass.dispatchWorkgroups(Math.ceil(this.width / 8), Math.ceil(this.height / 8));
      pass.end();
      device.queue.submit([enc.finish()]);
      if (reset && first) this.samples = 0;
      this.samples += n;
      left -= n;
      first = false;
    }
  }

  private groupWith(params: any): any {
    const p = build(), b = this.sceneBuffers;
    return device.createBindGroup({
      layout: p.traceLayout,
      entries: [
        { binding: 0, resource: { buffer: params } },
        { binding: 1, resource: { buffer: b[0] } }, { binding: 2, resource: { buffer: b[1] } },
        { binding: 3, resource: { buffer: b[2] } }, { binding: 4, resource: { buffer: b[3] } },
        { binding: 5, resource: this.mapTexture.createView({ dimension: '2d-array' }) },
        { binding: 6, resource: p.sampler },
        { binding: 7, resource: { buffer: this.accum } }, { binding: 8, resource: { buffer: this.bgsum } },
        { binding: 9, resource: { buffer: this.gbuf } }, { binding: 10, resource: { buffer: this.ebuf } },
        { binding: 11, resource: { buffer: b[4] } }, { binding: 12, resource: { buffer: this.grid } },
      ],
    });
  }

  /**
   * Resolve, denoise and compose into the display buffer: `passes` of Live's
   * filter, or with `guided` Final's (guides traced first, then filtered by
   * each pixel's own noise).
   */
  resolve(passes: number, guided = false): void {
    if (!this.view) return;
    const p = build();
    const enc = device.createCommandEncoder();
    let slot = 0;
    const bd = this.view.backdrop;
    const cam = this.view.camera, prev = this.prevCamera;
    const camera = (c: StudioCamera) => [...c.position, 0, ...c.forward, 0, ...c.right, c.tanY * c.aspect, ...c.up, c.tanY];
    const temporalOn = this.live && !!prev;
    // Final's denoiser hands compose its own coverage (see denoise.wgsl's finish).
    let sums = [this.accum, this.bgsum, this.ebuf];
    const run = (pipe: any, src: number, dst: number, step: number) => {
      const data = new ArrayBuffer(POST_BYTES);
      new Uint32Array(data, 0, 4).set([this.width, this.height, step, temporalOn ? 1 : 0]);
      new Float32Array(data, 16, 8).set([this.view!.exposure, 1.6, 0, 0, bd[0], bd[1], bd[2], this.view!.backdropOpaque ? 1 : 0]);
      new Float32Array(data, 48, 32).set([...camera(cam), ...camera(prev ?? cam)]);
      device.queue.writeBuffer(this.postParams[slot], 0, data);
      const group = device.createBindGroup({
        layout: p.postLayout,
        entries: [
          { binding: 0, resource: { buffer: this.postParams[slot] } },
          { binding: 1, resource: { buffer: sums[0] } }, { binding: 2, resource: { buffer: sums[1] } },
          { binding: 3, resource: { buffer: this.gbuf } }, { binding: 4, resource: { buffer: this.display[src] } },
          { binding: 5, resource: { buffer: this.display[dst] } }, { binding: 6, resource: { buffer: this.packed } },
          { binding: 7, resource: { buffer: sums[2] } },
          { binding: 8, resource: { buffer: this.history[this.historyIndex] } },
          { binding: 9, resource: { buffer: this.history[1 - this.historyIndex] } },
        ],
      });
      slot++;
      const pass = enc.beginComputePass();
      pass.setPipeline(pipe);
      pass.setBindGroup(0, group);
      pass.dispatchWorkgroups(Math.ceil(this.width / 8), Math.ceil(this.height / 8));
      pass.end();
    };
    run(p.resolve, 1, 0, 1);
    let cur = 0;
    if (guided) {
      const out = this.denoise(enc);
      cur = out.display;
      sums = [this.ones, out.coverage, this.ones];
    }
    if (this.live) {
      // Reuse the previous frame where it saw the same surface, then denoise as usual.
      run(p.temporal, cur, 1 - cur, 1); cur = 1 - cur;
      this.historyIndex = 1 - this.historyIndex;
      this.prevCamera = cam;
    }
    for (let i = 0; i < Math.min(passes, 5); i++) { run(p.atrous, cur, 1 - cur, 1 << i); cur = 1 - cur; }
    run(p.compose, cur, 1 - cur, 1);
    this.composed = 1 - cur;
    device.queue.submit([enc.finish()]);
  }
  private composed = 0;
  private aov: any = null;
  private side: any[] = [];
  /** (0, 0, 0, 1) per pixel: compose's sample count and emission once the denoiser has folded them in. */
  private ones: any = null;
  private denoiseParams: any[] = [];
  private denoiseBuffers(): any[] { return [this.aov, this.ones, ...this.side, ...this.denoiseParams]; }

  /**
   * Final's denoiser, from the resolved display[0]. Returns the display buffer
   * holding the denoised light and shading, and the bgsum stand-in carrying its coverage.
   */
  private denoise(enc: any): { display: number; coverage: any } {
    const d = buildDenoise(), p = build(), px = this.width * this.height;
    if (!this.aov) {
      const buf = (label: string, size: number, usage = U.STORAGE) => device.createBuffer({ label, size, usage });
      this.aov = buf('denoise guides', px * 32);
      this.side = [buf('denoise side a', px * 16), buf('denoise side b', px * 16)];
      this.ones = device.createBuffer({ label: 'denoise ones', size: px * 16, usage: U.STORAGE, mappedAtCreation: true });
      const ones = new Float32Array(this.ones.getMappedRange());
      for (let i = 3; i < ones.length; i += 4) ones[i] = 1;
      this.ones.unmap();
      this.denoiseParams = Array.from({ length: DENOISE_PASSES + 2 }, () => buf('denoise params', DENOISE_BYTES, U.UNIFORM | U.COPY_DST));
    }
    const b = this.sceneBuffers, groups = [Math.ceil(this.width / 8), Math.ceil(this.height / 8)] as const;
    const cam = this.view!.camera;
    // Guides with the trace's own camera and seed (its params are still bound).
    const guides = enc.beginComputePass();
    guides.setPipeline(d.aovs);
    guides.setBindGroup(0, device.createBindGroup({
      layout: d.aovLayout,
      entries: [
        { binding: 0, resource: { buffer: this.params } },
        { binding: 1, resource: { buffer: b[0] } }, { binding: 2, resource: { buffer: b[1] } },
        { binding: 3, resource: { buffer: b[2] } }, { binding: 4, resource: { buffer: b[3] } },
        { binding: 5, resource: this.mapTexture.createView({ dimension: '2d-array' }) },
        { binding: 6, resource: p.sampler },
        { binding: 11, resource: { buffer: b[4] } }, { binding: 13, resource: { buffer: this.aov } },
      ],
    }));
    guides.dispatchWorkgroups(...groups);
    guides.end();
    let slot = 0;
    const run = (pipe: any, src: number, dst: number, vin: number, vout: number, step: number) => {
      const params = this.denoiseParams[slot++];
      const data = new ArrayBuffer(DENOISE_BYTES);
      new Uint32Array(data, 0, 4).set([this.width, this.height, step, 0]);
      new Float32Array(data, 16, 4).set([DENOISE_SIGMA, cam.lensRadius, 0, 0]);
      device.queue.writeBuffer(params, 0, data);
      const pass = enc.beginComputePass();
      pass.setPipeline(pipe);
      pass.setBindGroup(0, device.createBindGroup({
        layout: d.layout,
        entries: [
          { binding: 0, resource: { buffer: params } },
          { binding: 2, resource: { buffer: this.aov } },
          { binding: 3, resource: { buffer: this.display[src] } }, { binding: 4, resource: { buffer: this.display[dst] } },
          { binding: 5, resource: { buffer: this.side[vin] } }, { binding: 6, resource: { buffer: this.side[vout] } },
          { binding: 7, resource: { buffer: this.accum } }, { binding: 8, resource: { buffer: this.bgsum } },
          { binding: 9, resource: { buffer: this.ebuf } },
        ],
      }));
      pass.dispatchWorkgroups(...groups);
      pass.end();
    };
    run(d.demod, 0, 1, 1, 0, 1);
    let cur = 1, v = 0;
    for (let i = 0; i < DENOISE_PASSES; i++) { run(d.step, cur, 1 - cur, v, 1 - v, 1 << i); cur = 1 - cur; v = 1 - v; }
    run(d.finish, cur, 1 - cur, v, 1 - v, 1);
    return { display: 1 - cur, coverage: this.side[1 - v] };
  }
  private history: any[] = [];
  private historyIndex = 0;
  private prevCamera: StudioCamera | null = null;

  /** Copy one object's pixels (or all, `objectIndex` < 0) into that layer's canvas. */
  present(key: string, objectIndex: number, includeVoid: boolean): HTMLCanvasElement {
    const p = build();
    let entry = this.canvases.get(key);
    if (!entry) {
      const canvas = document.createElement('canvas');
      canvas.width = this.width; canvas.height = this.height;
      const context = (canvas as any).getContext('webgpu');
      context.configure({ device, format: GPU.getPreferredCanvasFormat(), alphaMode: 'premultiplied' });
      entry = { canvas, context };
      this.canvases.set(key, entry);
    }
    let params = this.presentParams.get(key);
    if (!params) { params = device.createBuffer({ size: 16, usage: U.UNIFORM | U.COPY_DST }); this.presentParams.set(key, params); }
    device.queue.writeBuffer(params, 0, new Uint32Array([this.width, this.height, objectIndex < 0 ? 0 : objectIndex + 1, includeVoid ? 2 : 0]));
    const group = device.createBindGroup({
      layout: p.presentLayout,
      entries: [
        { binding: 0, resource: { buffer: params } },
        { binding: 1, resource: { buffer: this.display[this.composed] } },
        { binding: 2, resource: { buffer: this.gbuf } },
      ],
    });
    const enc = device.createCommandEncoder();
    const pass = enc.beginRenderPass({
      colorAttachments: [{ view: entry.context.getCurrentTexture().createView(), loadOp: 'clear', storeOp: 'store', clearValue: { r: 0, g: 0, b: 0, a: 0 } }],
    });
    pass.setPipeline(p.present);
    pass.setBindGroup(0, group);
    pass.draw(3);
    pass.end();
    device.queue.submit([enc.finish()]);
    return entry.canvas;
  }

  /** How deep Live's reuse goes: pixels with geometry, and how many carry ≥ `deep` frames of history. */
  async historyStats(deep = 8): Promise<{ pixels: number; reused: number; meanFrames: number }> {
    const src = this.history[this.historyIndex];
    const size = this.width * this.height * 32;
    const rb = device.createBuffer({ size, usage: U.MAP_READ | U.COPY_DST });
    const enc = device.createCommandEncoder(); enc.copyBufferToBuffer(src, 0, rb, 0, size); device.queue.submit([enc.finish()]);
    await rb.mapAsync(1);
    const f = new Float32Array(rb.getMappedRange().slice(0));
    rb.unmap(); rb.destroy();
    let pixels = 0, reused = 0, frames = 0;
    for (let i = 0; i < f.length; i += 8) {
      if (f[i + 7]! < 0) continue;
      pixels++; frames += f[i + 3]!;
      if (f[i + 3]! >= deep) reused++;
    }
    return { pixels, reused, meanFrames: pixels ? frames / pixels : 0 };
  }

  /** Waits for submitted work (tests, exports). */
  done(): Promise<void> {
    return device.queue.onSubmittedWorkDone();
  }
}
