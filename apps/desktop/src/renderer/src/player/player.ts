import { observeDisplayResolution } from './display-resolution';
import { install as easing } from '../legacy/core/easing';
import { install as model } from '../legacy/core/model';
import { install as animation } from '../legacy/core/anim';
import { install as shaders } from '../legacy/gl/shaders';
import { install as transitions } from '../legacy/gl/transitions';
import { install as raster } from '../legacy/gl/raster';
import { install as compositor } from '../legacy/gl/compositor';
import { install as audio } from '../legacy/core/audio';
import { prepareFrame } from '../legacy/core/frame-preparation';
import { validateScene, type WebScene } from './scene';

export interface PlayerOptions {
  canvas: HTMLCanvasElement;
  scene: string | URL | WebScene;
  baseURL?: string | URL;
  loop?: boolean;
  autoplay?: boolean;
  audio?: boolean;
  /** Ignore the composition background, preserving layer alpha. */
  transparent?: boolean;
  onError?: (error: Error) => void;
}

const hexDigits = (hex: string): string => {
  const s = String(hex ?? '').replace('#', '');
  return s.length === 3 || s.length === 4 ? [...s].map(c => c + c).join('') : s;
};

/** A private engine per player; never installs window.PM or editor UI. */
export function createEngine(project: any): any {
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const PM: any = {
    proj: project, time: 0, playing: false, quality: 1, headlessPlayer: true,
    clamp: (v: number, a: number, b: number) => Math.max(a, Math.min(b, v)),
    uid: (prefix = 'p') => prefix + Math.random().toString(36).slice(2),
    snapF: (t: number, fps: number) => Math.round(t * fps) / fps,
    hex2rgb(hex: string) {
      const s = hexDigits(hex);
      return [0, 2, 4].map(i => (parseInt(s.slice(i, i + 2), 16) || 0) / 255);
    },
    hexAlpha(hex: string) {
      const s = hexDigits(hex);
      return s.length === 8 ? (parseInt(s.slice(6, 8), 16) || 0) / 255 : 1;
    },
    bus: {
      on(event: string, fn: (...args: any[]) => void) {
        if (!listeners.has(event)) listeners.set(event, new Set());
        listeners.get(event)!.add(fn);
        return () => listeners.get(event)?.delete(fn);
      },
      emit(event: string, ...args: any[]) { listeners.get(event)?.forEach(fn => fn(...args)); },
      clear() { listeners.clear(); },
    },
    invalidate() {},
    toast(message: string) { console.warn('[Powermove]', message); },
  };
  easing(PM); model(PM); animation(PM); shaders(PM); transitions(PM);
  // The compositor needs parsed shader uniforms, but no editor panel state.
  const shaderMetadata = new WeakMap<object, any>();
  PM.UIState = {
    getShaderMeta(layer: any) {
      let meta = shaderMetadata.get(layer);
      if (!meta || meta.code !== layer.d.code) {
        meta = { code: layer.d.code, udefs: PM.parseUniforms(layer.d.code || '') };
        shaderMetadata.set(layer, meta);
      }
      return meta;
    },
    setShaderMeta(layer: any, patch: any) { Object.assign(this.getShaderMeta(layer), patch); },
  };
  PM.compOf = (layer: any) => PM.curComp().comps?.[layer.d.comp] || PM.proj.comps?.[layer.d.comp];
  PM.layerDefinition = (id: string) => PM.Kernel.layerTypes.get(id);
  return PM;
}

export async function createPlayer(options: PlayerOptions) {
  if (!options.canvas || options.canvas.tagName !== 'CANVAS') throw new Error('A canvas element is required');
  let base = new URL(options.baseURL || document.baseURI, document.baseURI);
  let source: WebScene;
  if (typeof options.scene === 'string' || options.scene instanceof URL) {
    const url = new URL(options.scene, base);
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Could not load scene (${response.status})`);
    source = await response.json();
    base = new URL('.', response.url || url.href);
  } else source = options.scene;
  validateScene(source);
  const scene = structuredClone(source);
  const PM = createEngine(scene.project);
  PM.audioDisabled = options.audio === false;
  for (const definition of scene.effects) PM.Kernel.effects.register('web-export', definition);
  for (const definition of scene.transitions) PM.Kernel.transitions.register('web-export', definition);
  for (const definition of scene.layerTypes) PM.Kernel.layerTypes.register('web-export', definition);
  const fonts: FontFace[] = [];
  const fontSources = new Map<string, ArrayBuffer>();
  PM.fontSource = (family: string) => fontSources.get(family);
  let dead = false, raf = 0, origin = 0, originTime = 0, transportGeneration = 0;
  let looping = options.loop ?? true;
  let pending: Promise<void> = Promise.resolve();
  let initialized = false;
  let display: ReturnType<typeof observeDisplayResolution> | undefined;
  const assertAlive = () => { if (dead) throw new Error('Player has been destroyed'); };
  const report = (error: unknown) => {
    const failure = error instanceof Error ? error : new Error(String(error));
    PM.playing = false; PM.Audio?.pause();
    if (options.onError) options.onError(failure); else console.error('[Powermove]', failure);
  };
  const render = (time: number) => {
    const job = pending.then(async () => {
      if (dead) return;
      display?.update();
      PM.time = time;
      await prepareFrame(PM, time);
      if (dead) return;
      PM.GL.render(time, { transparent: !!options.transparent, mblur: true, mbSamples: 6, shutter: PM.proj.shutter ?? .5 });
      if (PM.GL.errors.size) throw new Error([...PM.GL.errors.values()].join('\n'));
    });
    pending = job.catch(() => {});
    return job;
  };
  const dispose = () => {
    if (dead) return;
    dead = true;
    cancelAnimationFrame(raf);
    display?.destroy();
    PM.playing = false;
    PM.Audio?.destroy();
    // A pending decode must settle before its media and GPU resources disappear.
    void pending.finally(() => {
      PM.assets?.clear(); PM.rasterDispose?.(); PM.bus.clear();
      PM.GL?.gl?.getExtension('WEBGL_lose_context')?.loseContext();
      fonts.forEach(face => document.fonts.delete(face));
      fontSources.clear();
    });
  };
  try {
    for (const font of scene.fonts) {
      const response = await fetch(new URL(font.src, base));
      if (!response.ok) throw new Error(`Could not load font ${font.family}`);
      const data = await response.arrayBuffer();
      fontSources.set(font.family, data);
      const face = new FontFace(font.family, data, { weight: font.weight, style: font.style, ...(font.unicodeRange ? { unicodeRange: font.unicodeRange } : {}) });
      await face.load(); document.fonts.add(face); fonts.push(face);
    }
    PM.MediaStore = { async get(meta: any) {
      const location = scene.assets[meta.id];
      if (!location) return null;
      const response = await fetch(new URL(location, base));
      if (!response.ok) throw new Error(`Could not load media ${meta.name || meta.id}`);
      return response.blob();
    } };
    PM.MediaImport = { async mapBounded(items: any[], _limit: number, fn: (item: any) => Promise<any>) {
      const results = [];
      for (const item of items) results.push(await fn(item));
      return results;
    } };
    audio(PM); raster(PM); compositor(PM);
    if (!PM.GL.init(options.canvas, { alpha: true, quiet: true })) throw new Error('This animation requires WebGL2');
    PM.GL.resize(PM.proj.w, PM.proj.h);
    const gl = PM.GL.gl;
    const viewport = gl.getParameter(gl.MAX_VIEWPORT_DIMS);
    const limit = Math.min(8192, gl.getParameter(gl.MAX_TEXTURE_SIZE), gl.getParameter(gl.MAX_RENDERBUFFER_SIZE), viewport[0], viewport[1]);
    display = observeDisplayResolution(options.canvas, PM.proj.w, PM.proj.h, limit,
      (width, height) => PM.GL.resize(width, height),
      () => { if (initialized && !dead && !PM.playing) void render(PM.time).catch(report); });
    const restored = await PM.assets.restoreProject(PM.proj);
    if (restored.missing.length) throw new Error('Could not decode media: ' + restored.missing.map((a: any) => a.name).join(', '));
    await render(0);
    initialized = true;
  } catch (error) { dispose(); throw error; }

  const tick = async (now: number, generation: number) => {
    if (dead || !PM.playing || generation !== transportGeneration) return;
    let time = originTime + (now - origin) / 1000;
    if (time >= PM.proj.dur) {
      if (looping) {
        time %= PM.proj.dur; originTime = time; origin = now;
        if (options.audio !== false) PM.Audio.seek(time);
      } else {
        player.pause();
        time = Math.max(0, PM.proj.dur - 1 / PM.proj.fps);
      }
    }
    try {
      await render(time);
      if (generation !== transportGeneration) return;
      if (options.audio !== false && PM.playing) PM.Audio.tick(time);
      if (!dead && PM.playing) raf = requestAnimationFrame(now => tick(now, generation));
    } catch (error) { report(error); }
  };
  const player = {
    get duration(): number { return PM.proj.dur; },
    get currentTime(): number { return PM.time; },
    get playing(): boolean { return PM.playing; },
    get loop(): boolean { return looping; },
    set loop(value: boolean) { assertAlive(); looping = !!value; },
    get parameters() { return structuredClone(PM.proj.params); },
    play() {
      assertAlive(); if (PM.playing) return;
      PM.playing = true; origin = performance.now(); originTime = PM.time;
      const generation = ++transportGeneration;
      if (options.audio !== false) PM.Audio.start(PM.time);
      raf = requestAnimationFrame(now => tick(now, generation));
    },
    pause() { assertAlive(); ++transportGeneration; PM.playing = false; cancelAnimationFrame(raf); PM.Audio.pause(); },
    async seek(seconds: number) {
      assertAlive();
      if (!Number.isFinite(seconds)) throw new Error('Seek time must be finite');
      const time = Math.max(0, Math.min(Math.max(0, PM.proj.dur - 1 / PM.proj.fps), seconds));
      originTime = time; origin = performance.now();
      await render(time);
      if (PM.playing && options.audio !== false) PM.Audio.seek(time);
    },
    async setParameter(name: string, value: unknown) {
      assertAlive();
      const param = Object.hasOwn(PM.proj.params, name) && PM.proj.params[name];
      if (!param) throw new Error(`Unknown parameter: ${name}`);
      if (param.control === 'num' && (typeof value !== 'number' || !Number.isFinite(value)
        || (param.min != null && value < param.min) || (param.max != null && value > param.max))) throw new Error(`Invalid value for ${name}`);
      if (param.control === 'color' && (typeof value !== 'string' || !/^#(?:[\da-f]{2}){3,4}$/i.test(value))) throw new Error(`Invalid color for ${name}`);
      if (param.control === 'toggle' && typeof value !== 'boolean') throw new Error(`Invalid toggle for ${name}`);
      if (param.control === 'select' && !param.options?.some((o: any) => o.v === value)) throw new Error(`Invalid option for ${name}`);
      param.value = value; PM.touch(); PM.bus.emit('layers');
      await render(PM.time);
    },
    async setText(layerId: string, text: string) {
      assertAlive();
      const layer = PM.proj.layers.find((l: any) => l.id === layerId);
      if (layer?.type !== 'text' || typeof text !== 'string') throw new Error('setText requires a root text layer ID and a string');
      layer.d.text = PM.P(text); PM.touch(); PM.rasterClear();
      await render(PM.time);
    },
    destroy: dispose,
  };
  PM.invalidate = () => { if (initialized && !dead && !PM.playing) void render(PM.time).catch(report); };
  if (options.autoplay) player.play();
  return player;
}
