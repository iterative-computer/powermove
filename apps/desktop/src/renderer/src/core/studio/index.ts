import * as THREE from 'three';
import { extractScene, MAP_SIZE, type StudioScene } from './extract';
import { initStudioGpu, studioGpuFailure, studioGpuReady, StudioTarget, type StudioView } from './gpu';

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
const REFINE_SAMPLES = 512;
const REFINE_DELAY_MS = 140;

type Job = {
  sceneKey: string;
  viewKey: string;
  scene: StudioScene;
  maps: OffscreenCanvas[];
  live: StudioTarget | null;
  refine: StudioTarget | null;
  final: StudioTarget | null;
  changedAt: number;
  refining: boolean;
};

type State = { jobs: Map<string, Job>; tick: number | null; enabled: boolean; frames: number; declined: string; revision: number };
const states = new WeakMap<object, State>();
const state = (PM: any): State => {
  let s = states.get(PM);
  if (!s) {
    s = { jobs: new Map(), tick: null, enabled: true, frames: 0, declined: '', revision: 0 };
    states.set(PM, s);
    const st = s;
    // Editor-facing controls and status (the refine indicator, tests, agents).
    PM.Studio ??= {
      setEnabled: (on: boolean) => setStudioEnabled(PM, on),
      progress: () => studioProgress(PM),
      status: () => ({ enabled: st.enabled, ready: studioGpuReady(), failure: studioGpuFailure(), frames: st.frames, declined: st.declined }),
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

/** Viewer status for the refine indicator: samples so far and the goal. */
export function studioProgress(PM: any): { samples: number; goal: number; refining: boolean } | null {
  for (const job of state(PM).jobs.values()) {
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

function viewOf(runtime: any, camera: StudioView['camera']): StudioView {
  const raw = runtime.raw;
  const env = raw?.environment;
  const value = (key: string, fallback: any) => {
    const prop = env?.p?.[key];
    if (!prop) return fallback;
    try { return runtime.evaluation(prop, `environment.${key}`) ?? prop.v; } catch { return prop.v; }
  };
  const ambient = Number(value('ambient', 0.4));
  const sky = new THREE.Color(value('ambientColor', '#ffffff'));
  // three's hemisphere light lights a diffuse surface with albedo × color ×
  // intensity; a dome of that radiance does the same.
  const zenith = sky.clone().multiplyScalar(ambient);
  const ground = zenith.clone().multiplyScalar(0.35);
  const horizon = zenith.clone().lerp(ground, 0.5);
  const background = env?.background ? new THREE.Color(env.background) : null;
  const display = background ? background.clone().convertLinearToSRGB() : null;
  return {
    camera,
    exposure: Number(value('exposure', 1)),
    backdrop: display ? [display.r, display.g, display.b] : [0, 0, 0],
    backdropOpaque: !!background,
    shadowCatcher: false,
    environment: { zenith: [zenith.r, zenith.g, zenith.b], horizon: [horizon.r, horizon.g, horizon.b], ground: [ground.r, ground.g, ground.b] },
  };
}

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
export function studioLayer(PM: any, runtime: any, w: number, h: number, objectId: string, depthIds: Set<string>, bottom: boolean, camera: THREE.Camera): { canvas: HTMLCanvasElement; version: string } | null {
  if (!studioEnabled(PM)) return null;
  const width = Math.max(2, Math.min(8192, Math.round(w))), height = Math.max(2, Math.min(8192, Math.round(h)));
  const cam = cameraOf(camera, width, height);
  const s = state(PM);
  if (!cam) { s.declined = 'orthographic camera'; return null; }
  s.frames++;
  const ids = [...depthIds].sort();
  const setKey = ids.join(',');
  const sceneKey = key([runtime.revision, setKey]);
  const view = viewOf(runtime, cam);
  const viewKey = key([sceneKey, width, height, cam.position, cam.forward, cam.up, cam.tanY, view.exposure, view.backdrop, view.backdropOpaque]);
  const final = !!PM._studioFinal;
  let job = s.jobs.get(setKey);
  if (!job || job.sceneKey !== sceneKey) {
    const scene = extractScene(runtime.scene, id => depthIds.has(id), { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 });
    const maps = scene.maps.map(m => mapCanvas(m.source, m.flipY));
    const keepTargets = job;
    job = { sceneKey, viewKey: '', scene, maps, live: keepTargets?.live ?? null, refine: keepTargets?.refine ?? null, final: keepTargets?.final ?? null, changedAt: performance.now(), refining: false };
    for (const t of [job.live, job.refine, job.final]) if (t) t.setScene(scene, maps);
    s.jobs.set(setKey, job);
  }
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

  if (final) {
    const t = target('final', width, height);
    if (job.viewKey !== `final:${viewKey}` || t.samples < STUDIO_FINAL_SAMPLES) {
      t.setView(view);
      t.trace(STUDIO_FINAL_SAMPLES, 8, true, 64);
      t.resolve(0);
      job.viewKey = `final:${viewKey}`;
    }
    return { canvas: t.present(`${objectId}:final`, index, bottom), version: `final:${viewKey}:${t.samples}` };
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

/** Forget scenes for compositions that no longer exist (memory). */
export function pruneStudio(PM: any, keep: (setKey: string) => boolean): void {
  const s = state(PM);
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
