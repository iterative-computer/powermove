import type { PowermoveAPI, Layer, Effect, EffectParamDefinition, RasterSurface, RasterWindow, Bounds, Channel } from 'powermove';
import { createCaretMeasure, cursorBox, paintCursor, type TextGeometry, type CursorSettings } from './cursor';
import { typedState, typedCount, cursorVisible } from './typing.js';

type NativeSurface = RasterSurface & {
  cv: HTMLCanvasElement; w: number; h: number; anchorX: number; anchorY: number;
  key?: string; selection?: Bounds; fontOffset?: { x?: number; y?: number };
};
type Raster = (layer: Layer, scale?: number, time?: number, uploaded?: (key: string) => unknown, crop?: RasterWindow) => NativeSurface | null;

function enabled(api: PowermoveAPI, layer: Layer, effect: Effect, time: number) {
  const on = effect.on as unknown;
  return on && typeof on === 'object'
    ? Boolean(api.anim.evP(layer, on as Channel, time, `${effect.id}.$enabled`))
    : Boolean(on);
}

function settingsAt(api: PowermoveAPI, layer: Layer, fx: Effect, time: number, defaults: EffectParamDefinition[]): CursorSettings {
  return Object.fromEntries(defaults.map(p => [p.k,
    fx.p[p.k] ? api.anim.evP(layer, fx.p[p.k], time, `${fx.id}.${p.k}`) ?? p.def : p.def
  ]));
}

function checkedSurface(surface: NativeSurface | null): NativeSurface {
  if (!surface?.cv || !Number.isFinite(surface.anchorX) || !Number.isFinite(surface.anchorY)) {
    throw new Error('Typewriter needs the native text raster canvas and anchor geometry. This host did not provide them.');
  }
  return surface;
}

export function installTypingRaster(api: PowermoveAPI, defaults: EffectParamDefinition[]) {
  // The compatibility service boundary shadows the native raster capability and
  // restores it on disposal. No private host object or project mutation is used.
  const original = api.services.get<Raster>('raster');
  if (typeof original !== 'function') throw new Error('This Powermove version does not expose the raster service required for character-based Typewriter.');
  const measurement = createCaretMeasure();
  const cache = new Map<string, NativeSurface>();
  let pixelBudget = 0;
  const clear = () => { cache.clear(); pixelBudget = 0; measurement.clear(); };

  const raster: Raster = (layer, scale = 1, time = api.transport.time(), uploaded, crop) => {
    if (layer.type !== 'text') return original(layer, scale, time, uploaded, crop);
    const fx = layer.fx.find(effect => effect.type === 'typewriter' && enabled(api, layer, effect, time));
    if (!fx) return original(layer, scale, time, uploaded, crop);
    const settings = settingsAt(api, layer, fx, time, defaults);
    const resolved = api.anim.resolveContent(layer, time) as unknown as TextGeometry;
    const sourceText = String(resolved.text ?? '');
    const state = typedState(sourceText, settings.progress);
    const fps = api.model.curComp().fps;
    const show = cursorVisible(time, layer.from, fps, state, settings, (t: number) =>
      typedCount(api.anim.evP(layer, fx.p.progress, t, `${fx.id}.progress`), state.length));
    const key = JSON.stringify(['typewriter-3', layer.id, api.anim.version(), scale, time, resolved, settings, crop]);
    const cached = cache.get(key);
    if (cached) return cached;

    // Evaluate source text once, then render a detached prefix through the real
    // text renderer. It handles font outlines, kerning, shaping and text styles.
    // No future character reaches the raster; there is no clip edge or grid.
    const full = checkedSurface(original(layer, scale, time, undefined, crop));
    const typedLayer = {
      ...layer, id: `${layer.id}:typewriter:${state.count}`,
      d: { ...resolved, text: state.text },
      fx: layer.fx.filter(effect => effect.type !== 'typewriter')
    } as unknown as Layer;
    const typed = state.count > 0
      ? state.complete ? full : checkedSurface(original(typedLayer, scale, time, undefined, crop))
      : null;
    const typedWidth = typed ? Number(typed.w) || typed.cv.width / scale : 0;
    const typedHeight = typed ? Number(typed.h) || typed.cv.height / scale : 0;
    const caret = measurement.measure(resolved, state.text);
    const box = cursorBox(caret, settings, typed?.fontOffset ?? full.fontOffset ?? {});
    const w = Number(full.w) || full.cv.width / scale;
    const h = Number(full.h) || full.cv.height / scale;
    const sx = full.cv.width / w, sy = full.cv.height / h;
    // Keep the full source's origin and selection bounds stable while allowing
    // space for a cursor after the last letter, even on a tightly fitted raster.
    const left = Math.min(-full.anchorX, typed ? -typed.anchorX : 0, box.x - 2);
    const top = Math.min(-full.anchorY, typed ? -typed.anchorY : 0, box.y - 2);
    const right = Math.max(w - full.anchorX, typed ? typedWidth - typed.anchorX : 0, box.x + box.width + 2);
    const bottom = Math.max(h - full.anchorY, typed ? typedHeight - typed.anchorY : 0, box.y + box.height + 2);
    const width = Math.ceil((right - left) * sx), height = Math.ceil((bottom - top) * sy);
    if (![width, height].every(Number.isFinite) || width < 1 || height < 1 || width > 32768 || height > 32768 || width * height > 67108864) {
      throw new Error('Typewriter text surface exceeds the supported render size. Reduce the text size or preview resolution.');
    }
    const canvas = document.createElement('canvas');
    canvas.width = width; canvas.height = height;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('Typewriter could not create its text surface.');
    ctx.setTransform(sx, 0, 0, sy, -left * sx, -top * sy);
    if (typed) ctx.drawImage(typed.cv, -typed.anchorX, -typed.anchorY, typedWidth, typedHeight);
    if (show) paintCursor(ctx, box, settings, time);
    const result: NativeSurface = {
      ...full, cv: canvas, key, w: width / sx, h: height / sy,
      anchorX: -left, anchorY: -top,
      selection: full.selection,
      fontOffset: typed?.fontOffset ?? full.fontOffset
    };
    cache.set(key, result); pixelBudget += width * height;
    while (cache.size > 1 && pixelBudget > 8_000_000) {
      const oldest = cache.keys().next().value!;
      const entry = cache.get(oldest)!;
      pixelBudget -= entry.cv.width * entry.cv.height;
      cache.delete(oldest);
    }
    return result;
  };
  const registration = api.services.register<Raster>('raster', raster);
  api.on('project:changed', clear);
  api.on('fonts', clear);
  api.onDispose(() => { registration.dispose(); clear(); measurement.dispose(); api.transport.invalidate(); });
}
