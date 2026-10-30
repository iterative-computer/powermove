import * as THREE from 'three';
import { extractScene, isPending, MAP_SIZE, type StudioPending, type StudioScene } from './extract';
import { initStudioGpu, studioGpuFailure, studioGpuReady, studioMaxBindingBytes, studioOptions, StudioTarget, type StudioView } from './gpu';

/*
 * Studio in the editor: the compositor asks for one 3D layer's pixels and
 * gets a canvas. Behind it, one trace per (scene, camera, size) serves every
 * layer of a depth set:
 *
 * - Live (the scene or view is changing, or playback): half resolution, one
 *   sample, denoised, every frame.
 * - Refine (nothing changed for a moment): full resolution, accumulating a few
 *   samples per animation frame until it is clean, redrawing the viewer.
 * - Final (export and agent frames): full resolution, a fixed sample count,
 *   before the frame is read.
 *
 * Until WebGPU is ready, or where it is unavailable, layers render with the
 * classic renderer.
 */

export const STUDIO_FINAL_SAMPLES = 64;
const STUDIO_STILL_SAMPLES = 4;
const REFINE_SAMPLES = 512;
const REFINE_DELAY_MS = 140;

type Job = {
  sceneKey: string;
  viewKey: string;
  scene: StudioScene;
  maps: (OffscreenCanvas | Uint8Array)[];
  /** Texture layers that are live screens, and the frame they last showed. */
  screens: { layer: number; comp: string }[];
  screenKey: string;
  live: StudioTarget | null;
  refine: StudioTarget | null;
  final: StudioTarget | null;
  changedAt: number;
  refining: boolean;
  /** Last frame that drew it; unused jobs are dropped (their GPU memory with them). */
  usedAt: number;
};

/** Jobs not drawn for this long are dropped. */
const JOB_TTL_MS = 30_000;

type State = { jobs: Map<string, Job>; tick: number | null; enabled: boolean; frames: number; declined: string; revision: number; preparing: Map<string, StudioPending>; prunedAt: number };
const states = new WeakMap<object, State>();
const state = (PM: any): State => {
  let s = states.get(PM);
  if (!s) {
    s = { jobs: new Map(), tick: null, enabled: true, frames: 0, declined: '', revision: 0, preparing: new Map(), prunedAt: 0 };
    states.set(PM, s);
    const st = s;
    // Editor-facing controls and status (the refine indicator, tests, agents).
    PM.Studio ??= {
      setEnabled: (on: boolean) => setStudioEnabled(PM, on),
      setCaustics: (on: boolean) => { studioOptions.caustics = on; st.revision++; PM.invalidate?.('render'); },
      progress: () => studioProgress(PM),
      liveHistory: async () => { for (const job of st.jobs.values()) if (job.live) return job.live.historyStats(); return null; },
      status: () => ({ enabled: st.enabled, ready: studioGpuReady(), failure: studioGpuFailure(), frames: st.frames, declined: st.declined, preparing: preparingOf(st) }),
    };
    void initStudioGpu().then(ok => { if (ok) PM.invalidate?.('render'); });
  }
  return s;
};

/** Studio is the 3D renderer unless WebGPU is unavailable or it was turned off. */
export function studioEnabled(PM: any): boolean {
  const s = state(PM);
  return s.enabled && studioGpuReady();
}

export function setStudioEnabled(PM: any, on: boolean): void {
  const s = state(PM);
  s.enabled = on;
  s.revision++;
  PM.invalidate?.('render');
}

/** Part of the viewer's preview-cache key: which renderer drew 3D layers. */
export function studioPreviewKey(PM: any): string {
  const s = states.get(PM);
  return s ? `${s.enabled && studioGpuReady()}:${s.revision}` : 'off';
}

/** While Studio is still sharpening a view, the viewer must not reuse cached frames. */
export function studioUnsettled(PM: any): boolean {
  const s = states.get(PM);
  if (!s || !s.enabled || !studioGpuReady()) return false;
  for (const job of s.jobs.values()) if (job.refine && job.refine.samples < REFINE_SAMPLES) return true;
  return false;
}

/** Large models whose BVHs are still building, summed over compositions. */
function preparingOf(s: State): StudioPending | null {
  if (!s.preparing.size) return null;
  let pending = 0, total = 0;
  for (const p of s.preparing.values()) { pending += p.pending; total += p.total; }
  return { pending, total };
}

/** Viewer status for the refine indicator: samples so far and the goal, or a large model still being prepared. */
export function studioProgress(PM: any): { samples: number; goal: number; refining: boolean; preparing?: StudioPending } | null {
  const s = state(PM);
  const preparing = preparingOf(s);
  if (preparing) return { samples: 0, goal: REFINE_SAMPLES, refining: false, preparing };
  for (const job of s.jobs.values()) {
    if (job.refine) return { samples: job.refine.samples, goal: REFINE_SAMPLES, refining: job.refining };
  }
  return null;
}

const mapCanvases = new WeakMap<object, OffscreenCanvas>();
function mapCanvas(source: any, flipY: boolean): OffscreenCanvas {
  let c = mapCanvases.get(source);
  if (!c) {
    c = new OffscreenCanvas(MAP_SIZE, MAP_SIZE);
    const ctx = c.getContext('2d')!;
    // three uploads flipY textures bottom row first; match its UVs.
    if (flipY) { ctx.translate(0, MAP_SIZE); ctx.scale(1, -1); }
    ctx.drawImage(source, 0, 0, MAP_SIZE, MAP_SIZE);
    mapCanvases.set(source, c);
  }
  return c;
}

function cameraOf(camera: THREE.Camera, w: number, h: number): StudioView['camera'] | null {
  const pc = camera as THREE.PerspectiveCamera;
  if (!pc.isPerspectiveCamera) return null;
  camera.updateMatrixWorld(true);
  const e = camera.matrixWorld.elements;
  const right = new THREE.Vector3(e[0], e[1], e[2]).normalize();
  const up = new THREE.Vector3(e[4], e[5], e[6]).normalize();
  const forward = new THREE.Vector3(-e[8], -e[9], -e[10]).normalize();
  return {
    position: [e[12], e[13], e[14]],
    forward: [forward.x, forward.y, forward.z],
    right: [right.x, right.y, right.z],
    up: [up.x, up.y, up.z],
    tanY: Math.tan(THREE.MathUtils.degToRad(pc.fov) / 2) / Math.max(pc.zoom, 1e-3),
    aspect: w / Math.max(1, h),
    lensRadius: 0,
    focus: 1,
  };
}

/** The studio backdrop when a set is on and no background color was chosen. */
export const STUDIO_BACKDROP = '#F6F5F8';

function viewOf(runtime: any, camera: StudioView['camera'], bounds: THREE.Box3): StudioView {
  const raw = runtime.raw;
  const env = raw?.environment;
  const channel = (node: any, prefix: string, key: string, fallback: any) => {
    const prop = node?.p?.[key];
    if (!prop) return fallback;
    try { return runtime.evaluation(prop, `${prefix}.${key}`) ?? prop.v; } catch { return prop.v; }
  };
  const value = (key: string, fallback: any) => channel(env, 'environment', key, fallback);
  // The lens: an f-number opens the aperture; focus defaults to the aim point.
  const fStop = Number(channel(raw?.camera, 'camera', 'fStop', 0));
  const focus = Number(channel(raw?.camera, 'camera', 'focusDistance', 0));
  const fov = Number(channel(raw?.camera, 'camera', 'fov', 45));
  if (fStop > 0) {
    // Focal length from the field of view on a 24 mm-tall sensor, in scene units
    // scaled like the scene (1 unit ≈ 100 mm at product scale).
    const focalMm = 12 / Math.tan(THREE.MathUtils.degToRad(fov) / 2);
    camera.lensRadius = (focalMm / (2 * fStop)) / 100;
  }
  const target = raw?.camera ? new THREE.Vector3(Number(channel(raw.camera, 'camera', 'targetX', 0)), Number(channel(raw.camera, 'camera', 'targetY', 0)), Number(channel(raw.camera, 'camera', 'targetZ', 0))) : null;
  camera.focus = focus > 0 ? focus : target ? target.distanceTo(new THREE.Vector3(...camera.position)) : 1;
  const ambient = Number(value('ambient', 0.4));
  const sky = new THREE.Color(value('ambientColor', '#ffffff'));
  // three's hemisphere light lights a diffuse surface with albedo × color ×
  // intensity; a dome of that radiance does the same.
  const zenith = sky.clone().multiplyScalar(ambient);
  const ground = zenith.clone().multiplyScalar(0.35);
  const horizon = zenith.clone().lerp(ground, 0.5);
  const catcher = !!env?.shadowCatcher && !bounds.isEmpty();
  const background = env?.background ? new THREE.Color(env.background) : catcher ? new THREE.Color(STUDIO_BACKDROP) : null;
  const display = background ? background.clone().convertLinearToSRGB() : null;
  return {
    camera,
    exposure: Number(value('exposure', 1)),
    backdrop: display ? [display.r, display.g, display.b] : [0, 0, 0],
    backdropOpaque: !!background,
    shadowCatcher: catcher,
    shadows: env?.shadows !== false,
    floorY: catcher ? bounds.min.y : 0,
    environment: { zenith: [zenith.r, zenith.g, zenith.b], horizon: [horizon.r, horizon.g, horizon.b], ground: [ground.r, ground.g, ground.b] },
  };
}

/** What a live screen's picture depends on: the time and every project edit. */
function screenKeyOf(PM: any, time: number): string { return `${time}:${PM.animVersion?.() ?? ''}`; }

function key(parts: unknown[]): string { return parts.map(p => (Array.isArray(p) ? p.join(',') : String(p))).join('|'); }

function schedule(PM: any): void {
  const s = state(PM);
  if (s.tick !== null) return;
  const run = () => {
    s.tick = null;
    let more = false;
    const now = performance.now();
    for (const job of s.jobs.values()) {
      if (!job.refine?.view || PM.playing) continue;
      if (now - job.changedAt < REFINE_DELAY_MS) { more = true; continue; }
      if (job.refine.samples >= REFINE_SAMPLES) { job.refining = false; continue; }
      job.refining = true;
      // A few samples per frame keeps the editor responsive while it sharpens.
      job.refine.trace(job.refine.samples === 0 ? 2 : 4, 6, job.refine.samples === 0, 64);
      job.refine.resolve(0);
      more = true;
    }
    PM.invalidate?.('render');
    if (more) schedule(PM);
  };
  s.tick = requestAnimationFrame(run) as unknown as number;
}

/**
 * One 3D layer's pixels from Studio, or null to use the classic renderer.
 * `objectId` is the layer's scene id; `depthIds` the objects sharing its
 * depth (one trace each); `bottom` whether the layer owns the backdrop.
 */
export function studioLayer(PM: any, runtime: any, w: number, h: number, objectId: string, depthIds: Set<string>, bottom: boolean, camera: THREE.Camera, renderScreen?: (comp: string) => Uint8Array | null, time: number = PM.time): { canvas: HTMLCanvasElement; version: string } | null {
  if (!studioEnabled(PM)) return null;
  // A splat layer is drawn by the splat rasterizer, composited by layer order.
  if (hasSplats(runtime, objectId)) return null;
  const width = Math.max(2, Math.min(8192, Math.round(w))), height = Math.max(2, Math.min(8192, Math.round(h)));
  const cam = cameraOf(camera, width, height);
  const s = state(PM);
  if (!cam) { s.declined = 'orthographic camera'; return null; }
  s.frames++;
  const now = performance.now();
  if (now - s.prunedAt > 5_000) {
    s.prunedAt = now;
    pruneStudio(PM, k => (s.jobs.get(k)?.usedAt ?? now) > now - JOB_TTL_MS);
  }
  const ids = [...depthIds].sort();
  const setKey = ids.join(',');
  const sceneKey = key([runtime.revision, setKey]);
  const final = !!PM._studioFinal;
  let job = s.jobs.get(setKey);
  if (!job || job.sceneKey !== sceneKey) {
    // Exports build every BVH now; the editor builds large ones in workers
    // and draws with the classic renderer until they are ready.
    const env = { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 };
    const include = (id: string) => depthIds.has(id);
    // Each finished build clears the status; the next frame sets it again if more are pending.
    const onReady = () => { s.preparing.delete(setKey); s.revision++; PM.invalidate?.('render'); };
    const extracted = final ? extractScene(runtime.scene, include, env) : extractScene(runtime.scene, include, env, { onReady });
    if (isPending(extracted)) {
      s.preparing.set(setKey, extracted);
      return null;
    }
    s.preparing.delete(setKey);
    const scene = extracted;
    // Beyond one storage binding (shading data: 96 bytes a triangle) the classic renderer draws it.
    if (scene.tris.byteLength > studioMaxBindingBytes()) {
      s.declined = `${scene.triangles} triangles is more than this GPU can trace`;
      return null;
    }
    const screens: { layer: number; comp: string }[] = [];
    const maps = scene.maps.map((m, layer) => {
      if ('screen' in m) { screens.push({ layer, comp: m.screen }); return renderScreen?.(m.screen) ?? new Uint8Array(MAP_SIZE * MAP_SIZE * 4); }
      return mapCanvas(m.source, m.flipY);
    });
    const keepTargets = job;
    job = { sceneKey, viewKey: '', scene, maps, screens, screenKey: screenKeyOf(PM, time), live: keepTargets?.live ?? null, refine: keepTargets?.refine ?? null, final: keepTargets?.final ?? null, changedAt: performance.now(), refining: false, usedAt: 0 };
    for (const t of [job.live, job.refine, job.final]) if (t) t.setScene(scene, maps);
    s.jobs.set(setKey, job);
  }
  job.usedAt = now;
  // Live screens follow time and edits to their compositions without
  // rebuilding the 3D scene: only their texture layers are rewritten.
  if (job.screens.length && job.screenKey !== screenKeyOf(PM, time)) {
    job.screenKey = screenKeyOf(PM, time);
    for (const { layer, comp } of job.screens) {
      const pixels = renderScreen?.(comp);
      if (!pixels) continue;
      job.maps[layer] = pixels;
      for (const t of [job.live, job.refine, job.final]) t?.writeMap(layer, pixels);
    }
    job.viewKey = '';
  }
  const view = viewOf(runtime, cam, job.scene.bounds);
  const viewKey = key([sceneKey, width, height, cam.position, cam.forward, cam.up, cam.tanY, cam.lensRadius, cam.focus, view.exposure, view.backdrop, view.backdropOpaque, view.shadowCatcher, view.shadows, studioOptions.caustics]);
  const index = job.scene.objects.indexOf(objectId);
  const target = (which: 'live' | 'refine' | 'final', tw: number, th: number): StudioTarget => {
    let t = job![which];
    if (!t || t.width !== tw || t.height !== th) {
      t?.destroy();
      t = new StudioTarget(tw, th);
      t.live = which === 'live';
      t.setScene(job!.scene, job!.maps);
      job![which] = t;
    }
    return t;
  };

  if (final || PM._studioStill) {
    const t = target('final', width, height);
    // Final: the composition's quality, split across motion-blur shutter samples.
    // Stills: a fixed four samples, denoised, the same every time.
    const still = !final;
    const goal = still ? STUDIO_STILL_SAMPLES : Math.max(4, Math.ceil(Number(runtime.raw?.environment?.samples ?? STUDIO_FINAL_SAMPLES) / Math.max(1, PM._studioShutterSamples || 1)));
    const tag = `${still ? 'still' : 'final'}:${viewKey}`;
    if (job.viewKey !== tag || t.samples < goal) {
      t.setView(view);
      t.trace(goal, 8, true, 64);
      t.resolve(still ? 3 : 0);
      job.viewKey = tag;
    }
    return { canvas: t.present(`${objectId}:final`, index, bottom), version: `${job.viewKey}:${t.samples}` };
  }

  const refine = target('refine', width, height);
  if (job.viewKey !== viewKey) {
    job.viewKey = viewKey;
    job.changedAt = performance.now();
    job.refining = false;
    const live = target('live', Math.max(2, width >> 1), Math.max(2, height >> 1));
    live.setView(view);
    live.trace(1, 2, true, 6);
    live.resolve(4);
    refine.setView(view);
    schedule(PM);
    return { canvas: live.present(`${objectId}:live`, index, bottom), version: `live:${viewKey}:${live.samples}` };
  }
  if (refine.samples === 0) {
    schedule(PM);
    const live = job.live!;
    return { canvas: live.present(`${objectId}:live`, index, bottom), version: `live:${viewKey}:${live.samples}` };
  }
  return { canvas: refine.present(`${objectId}:refine`, index, bottom), version: `refine:${viewKey}:${refine.samples}` };
}

const splatLayers = new WeakMap<object, { revision: unknown; ids: Set<string> }>();
/** Whether the layer's object is a Gaussian splat capture (cached per scene revision). */
function hasSplats(runtime: any, objectId: string): boolean {
  let entry = splatLayers.get(runtime);
  if (!entry || entry.revision !== runtime.revision) {
    const ids = new Set<string>();
    runtime.scene?.traverse((o: any) => { if (o.userData?.splat && o.userData.sceneId) ids.add(o.userData.sceneId); });
    entry = { revision: runtime.revision, ids };
    splatLayers.set(runtime, entry);
  }
  return entry.ids.has(objectId);
}

/** Forget scenes for compositions that no longer exist (memory). */
export function pruneStudio(PM: any, keep: (setKey: string) => boolean): void {
  const s = state(PM);
  for (const k of s.preparing.keys()) if (!keep(k)) s.preparing.delete(k);
  for (const [k, job] of s.jobs) {
    if (keep(k)) continue;
    for (const t of [job.live, job.refine, job.final]) t?.destroy();
    s.jobs.delete(k);
  }
}

/** Waits until every queued Studio frame finished on the GPU (exports, tests). */
export async function studioIdle(): Promise<void> {
  if (!studioGpuReady()) return;
  await new Promise(requestAnimationFrame);
}
