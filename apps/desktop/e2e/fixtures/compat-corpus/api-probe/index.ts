import type { PowermoveAPI } from 'powermove';
import * as helpers from 'powermove';

/*
 * The compatibility corpus's API probe (owner amendment 24). The same source
 * runs at apiVersion 1, 2 and 3, in the trusted in-realm context and as a
 * Store install that declares full-access (the test writes the manifests).
 * The command `<id>.run` calls each section 20 member as code written
 * before the engine does (synchronously where today's API is synchronous)
 * and compares what it gets with today's answer in the same window: the
 * current engine's own functions (`api.host.pm`, scaffolding until routing
 * is removed), the legacy helper implementations, or a known value. The
 * command returns its record (and keeps it on `window.__compatProbe`), so
 * the test asserts behavior, not names.
 */
type Result = { ok: boolean; detail?: unknown };
type Fixture = { image?: string; video?: string; audio?: string; text?: string; shape?: string; group?: string; child?: string; three?: string; modelAsset?: string; helperNames?: string[] };

const TOL = 1e-9;
const near = (a: unknown, b: unknown): boolean => {
  if (typeof a === 'number' && typeof b === 'number') return Math.abs(a - b) <= TOL * Math.max(1, Math.abs(b)) || (Number.isNaN(a) && Number.isNaN(b));
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => near(v, b[i]));
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const ka = Object.keys(a as object).sort(), kb = Object.keys(b as object).sort();
    return near(ka, kb) && ka.every((k) => near((a as any)[k], (b as any)[k]));
  }
  return a === b;
};
const json = (v: unknown) => { try { return JSON.parse(JSON.stringify(v)); } catch { return String(v); } };

/** Section 25's render measure: max channel difference, values over 1. */
function compare(a: ArrayLike<number>, b: ArrayLike<number>): { max: number; over1: number; values: number } {
  let max = 0, over1 = 0, values = 0;
  for (let i = 0; i < a.length; i++) {
    if (i % 4 === 3) continue;
    const d = Math.abs(a[i]! - b[i]!);
    values++;
    if (d > max) max = d;
    if (d > 1) over1++;
  }
  return { max, over1, values };
}
const same = (c: { max: number; over1: number; values: number }) => c.max <= 4 && c.over1 <= c.values * 0.001;
const pixels = (canvas: HTMLCanvasElement) => canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;

async function decode(url: string): Promise<Uint8ClampedArray> {
  const image = new Image();
  image.src = url;
  await image.decode();
  const c = document.createElement('canvas');
  c.width = image.width; c.height = image.height;
  c.getContext('2d')!.drawImage(image, 0, 0);
  return c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
}

export default function activate(api: PowermoveAPI): void {
  /* Every event, its payloads in order, from activation on. */
  const EVENTS = ['scene3d:selection', 'inspector:changed', 'project:changed', 'selection', 'time', 'transport', 'fonts', 'layout', 'theme:changed', 'extension:loaded', 'extension:unloaded', 'extensions:ready', 'extensions:changed', 'frame:rendered', 'overlay', 'invalidate'] as const;
  const log: Array<{ name: string; payload: unknown; at: number }> = [];
  let frameViewport: { matches: boolean; viewport: unknown } | null = null;
  for (const name of EVENTS) {
    api.events.on(name as any, (payload: any) => {
      log.push({ name, payload: name === 'frame:rendered' ? { time: payload?.time, version: payload?.version, quality: payload?.quality } : json(payload), at: performance.now() });
      if (name === 'frame:rendered') frameViewport = { matches: payload?.viewport === api.render.gl.previewViewport, viewport: json(payload?.viewport) };
    });
  }
  const until = (name: string, from: number, ms = 4000) => new Promise<boolean>((resolve) => {
    const started = performance.now();
    const tick = () => {
      if (log.slice(from).some((e) => e.name === name)) return resolve(true);
      if (performance.now() - started > ms) return resolve(false);
      setTimeout(tick, 20);
    };
    tick();
  });

  api.commands.register({
    id: `${api.id}.run`,
    label: 'Run API probe',
    run: async (fixture: Fixture = {}) => {
      const out: Record<string, Result> = {};
      const check = (name: string, fn: () => Result) => {
        try { out[name] = fn(); } catch (error) { out[name] = { ok: false, detail: `threw: ${String((error as Error)?.message ?? error)}` }; }
      };
      const checkAsync = async (name: string, fn: () => Promise<Result>) => {
        try { out[name] = await fn(); } catch (error) { out[name] = { ok: false, detail: `threw: ${String((error as Error)?.message ?? error)}` }; }
      };
      const pm = api.host.pm as any;
      const project = api.project.get();
      const byId = (id?: string) => (id ? api.model.layer(id) : null) as any;
      const text = byId(fixture.text) ?? (project.layers.find((l: any) => l.type === 'text') as any);
      const shape = byId(fixture.shape);
      const group = byId(fixture.group);
      const child = byId(fixture.child);
      const t = 0.5;

      /* ── project.* ── */
      check('project.get live', () => ({ ok: api.project.get() === project && api.model.curComp() === project && api.project.latest() === project }));
      check('project.revision', () => { const r = api.project.revision(); return { ok: Number.isInteger(r) && r >= 0, detail: r }; });
      check('project.exportDefaults today ids', () => ({ ok: ['prores', 'mp4', 'webm', 'png', 'still', 'web', 'json', 'rec'].includes(String((project as any).exportDefaults?.format)), detail: (project as any).exportDefaults?.format }));
      check('project.select/selection', () => {
        api.project.select([text.id]);
        const s: any = api.project.selection();
        return { ok: Array.isArray(s.layers) && s.layers.length === 1 && s.layers[0] === text.id && near(json(s.layers), json(pm.sel?.layers)), detail: json(s.layers) };
      });
      check('project.setTime/time', () => {
        // Frame-aligned times read back exactly; others snap to the frame, as today.
        api.project.setTime(9 / 30); const v = api.project.time();
        api.project.setTime(0.25); const snapped = api.project.time();
        api.project.setTime(t);
        return { ok: v === 9 / 30 && snapped === pm.snapF(0.25, project.fps) && api.project.time() === t, detail: [v, snapped] };
      });
      /* ── anim.* (pm-wasm with the engine; today's functions for the reference) ── */
      const opacity = text.p.opacity;
      check('anim.ev', () => { const a = api.anim.ev(text, 'opacity', t), b = pm.ev(text, 'opacity', t); return { ok: near(a, b) && a === 100, detail: [a, b] }; });
      check('anim.evP', () => { const a = api.anim.evP(text, text.p['position.x'], t, 'position.x'), b = pm.evP(text, text.p['position.x'], t, 'position.x'); return { ok: near(a, b), detail: [a, b] }; });
      check('anim.active', () => { const a = api.anim.active(text, t), b = !!pm.active(text, t), c = api.anim.active(text, 99); return { ok: a === b && a === true && c === !!pm.active(text, 99), detail: [a, b, c] }; });
      check('anim.findProp identity', () => { const p = api.anim.findProp(text, 'opacity'); return { ok: p === opacity && p === pm.findProp(text, 'opacity') }; });
      check('anim.allProps', () => { const a = api.anim.allProps(text), b = pm.allProps(text); return { ok: a.length === b.length && a.length > 4 && a.every((e: any, i: number) => e.key === b[i].key && e.prop === b[i].prop), detail: a.length }; });
      check('anim.hasKeyAt', () => { const p = text.p['position.x']; const a = api.anim.hasKeyAt(text, p, 0), b = pm.hasKeyAt(text, p, 0); return { ok: a === b && !!a, detail: !!a }; });
      check('anim.wouldCycle', () => { const a = child ? api.anim.wouldCycle(group, child.id) : null, b = child ? !!pm.wouldCycle(group, child.id) : null; return { ok: !!child && a === b && a === true && api.anim.wouldCycle(text, null) === false, detail: [a, b] }; });
      check('anim.resolveContent', () => { const a: any = api.anim.resolveContent(text, t), b = pm.resolveContent(text, t); return { ok: near(json(a), json(b)) && a?.text === text.d.text }; });
      check('anim.worldMatrix', () => { const a = api.anim.worldMatrix(child ?? text, t), b = pm.worldMatrix(child ?? text, t); return { ok: near(a, b) && a.length === 6, detail: a }; });
      check('anim.localMatrix', () => { const a = api.anim.localMatrix(text, t), b = pm.localMatrix(text, t); return { ok: near(a, b), detail: a }; });
      check('anim.transformParentMatrix', () => { const a = api.anim.transformParentMatrix(child ?? text, t), b = pm.transformParentMatrix(child ?? text, t); return { ok: near(a, b), detail: a }; });
      check('anim.mul', () => { const m: any = [2, 0, 0, 2, 5, 7]; const a = api.anim.mul(m, m), b = pm.mul(m, m); return { ok: near(a, b) && near(a, [4, 0, 0, 4, 15, 21]), detail: a }; });
      check('anim.version/touch', () => { const v = api.anim.version(); api.anim.touch(); const w = api.anim.version(); return { ok: Number.isFinite(v) && w > v, detail: [v, w] }; });
      check('anim.setKey/hasKeyAt/removeKey/applyEaseTo/setKeyOn', () => {
        const k = api.anim.setKey(text, 'opacity', 1, 50);
        const has = api.anim.hasKeyAt(text, opacity, 1);
        const value = api.anim.ev(text, 'opacity', 1);
        api.anim.applyEaseTo([k as any], 'linear');
        const eased = (k as any)?.e ?? (k as any)?.ease;
        const k2 = api.anim.setKeyOn(opacity, 2, 75);
        const value2 = api.anim.ev(text, 'opacity', 2);
        api.anim.removeKey(opacity, k2 as any);
        api.anim.removeKey(opacity, k as any);
        const after = api.anim.ev(text, 'opacity', 1);
        const todayAfter = pm.ev(text, 'opacity', 1);
        api.anim.touch();
        return { ok: !!k && has === k && value === 50 && value2 === 75 && near(after, todayAfter) && opacity.kf.length === 0, detail: { value, value2, after, todayAfter, eased: json(eased), left: opacity.kf.length } };
      });
      check('anim.expressionErrors', () => {
        const errors = api.anim.expressionErrors;
        return { ok: typeof errors.get === 'function' && typeof errors.has === 'function' && errors.has(opacity) === false };
      });

      /* ── model.* ── */
      check('model factories', () => {
        const p = api.model.P(5); const k = api.model.KF(1, 2, 'linear');
        const layer = api.model.mkLayer('solid', { name: 'Detached' });
        const mask = api.model.mkMask('rect');
        const proj = api.model.mkProject({ name: 'Detached project' });
        const clone = api.model.cloneLayer(text);
        const fill: any = api.model.normalizeFill('#ff0000');
        return { ok: p.v === 5 && Array.isArray(p.kf) && k.t === 1 && k.v === 2 && layer.type === 'solid' && !project.layers.includes(layer) && !!mask && proj.name === 'Detached project' && clone.id !== text.id && clone.d.text === text.d.text && !!fill, detail: { fill: json(fill) } };
      });
      check('model tables', () => ({ ok: typeof api.model.CH === 'object' && Object.keys(api.model.CH).length > 5 && api.model.BLENDS.length > 3 && !!api.model.TYPE_META && api.model.MASK_SHAPES.length > 0 && api.model.CH === pm.CH }));
      check('model lookups', () => {
        const parts = { layer: api.model.layer(text.id) === text, byName: api.model.byName(text.name) === text, missing: api.model.layer('missing') == null, todayMissing: pm.L?.('missing') ?? null };
        return { ok: parts.layer && parts.byName && parts.missing, detail: parts };
      });

      /* ── groups.* ── */
      check('groups', () => {
        if (!group || !child) return { ok: false, detail: 'no group fixture' };
        const anc = api.groups.ancestors(child);
        const roots = api.groups.transformRoots([group.id, child.id]);
        const span = api.groups.span(group);
        const expanded = api.groups.expand([group.id]);
        const bounds: any = api.groups.bounds(group, t);
        const today: any = pm.groupBounds?.(group, t) ?? pm.Groups?.bounds?.(group, t) ?? null;
        const parts = { ancestors: anc.map((l: any) => l.id), roots: roots.map((l: any) => l.id), span: json(span), expanded };
        const todayParts = { ancestors: (pm.groupAncestors?.(child) ?? anc).map((l: any) => l.id) };
        return { ok: anc[0] === group && roots.some((l: any) => l === group) && Number.isFinite(span.from) && expanded.includes(child.id) && !!bounds && Number.isFinite(bounds.w) && (today === null || near(json(bounds), json(today))), detail: { parts, todayParts, bounds: json(bounds), today: json(today) } };
      });

      /* ── space3d.* and util.* ── */
      check('space3d', () => {
        const m = api.space3d.planeMatrix(text, t);
        const p = api.space3d.projectPoint(m, { x: 10, y: 20 });
        const inv = api.space3d.inversePlane(m);
        const ref = helpers.planeMatrix(pm, text, t);
        return { ok: Array.isArray(m) && m.length === 9 && Number.isFinite(p.x) && !!inv && api.space3d.is3DLayer(text) === false && near(json(m), json(ref)) && api.space3d.perspectiveAmount(text, t) >= 0 && api.space3d.world3D(text, t).length === 16, detail: m };
      });
      check('util', () => {
        const u = api.util;
        return { ok: u.round(1.2345, 2) === 1.23 && u.clamp(5, 0, 2) === 2 && u.lerp(0, 10, 0.5) === 5 && u.snapF(0.51, 10) === 0.5 && u.tc(1.5, 30) === pm.tc(1.5, 30) && u.parseTc(u.tc(2, 30), 30) === 2 && /^p/.test(u.uid('p')) && near(u.hex2rgb('#ff8000'), [1, 128 / 255, 0]) && u.rgb2hex(1, 0, 0).toLowerCase() === '#ff0000', detail: u.tc(1.5, 30) };
      });

      /* ── transport.* ── */
      check('transport', () => {
        api.transport.setTime(24 / 30);
        const a = api.transport.time();
        api.transport.step(1);
        const b = api.transport.time();
        api.transport.setTime(t);
        const perf: any = api.transport.perf;
        return { ok: Math.abs(a - 24 / 30) < 1e-12 && Math.abs(b - 25 / 30) < 1e-9 && api.transport.playing() === false && typeof api.transport.quality === 'number' && perf && Number.isFinite(perf.fps) && typeof api.transport.invalidate === 'function', detail: { a, b, quality: api.transport.quality, previewResolution: api.transport.previewResolution } };
      });

      /* ── render.* ── */
      check('render.gl.bounds', () => { const a: any = api.render.gl.bounds(text, t), b: any = pm.GL.bounds(text, t); return { ok: !!a && near([a.x0, a.y0, a.x1, a.y1].map((v: number) => Math.round(v * 1e6)), [b.x0, b.y0, b.x1, b.y1].map((v: number) => Math.round(v * 1e6))), detail: [json(a), json(b)] }; });
      check('render.gl.pick', () => {
        // Bounds are layer space: the title's center in composition space.
        const b: any = api.render.gl.bounds(text, t);
        const m = api.anim.worldMatrix(text, t);
        const cx = (b.x0 + b.x1) / 2, cy = (b.y0 + b.y1) / 2;
        const x = m[0] * cx + m[2] * cy + m[4], y = m[1] * cx + m[3] * cy + m[5];
        const a = api.render.gl.pick(x, y, t), today = pm.GL.pick(x, y, t);
        const outside = api.render.gl.pick(-9999, -9999, t), todayOutside = pm.GL.pick(-9999, -9999, t);
        return { ok: a === today && !!a && outside == todayOutside, detail: { a: a?.id, today: today?.id, text: text.id, outside: outside?.id ?? outside, todayOutside: todayOutside?.id ?? todayOutside } };
      });
      check('render.gl.previewViewport/canvas', () => ({ ok: api.render.gl.canvas instanceof HTMLCanvasElement && api.render.gl.previewViewport === pm.GL.previewViewport }));
      check('render.gl.compileError', () => { const e = api.render.gl.compileError('powermove:no-such-program'); return { ok: e === null || typeof e === 'string', detail: e }; });
      check('render.gl.context', () => {
        const gl = api.render.gl.context;
        if (!gl) return { ok: false, detail: 'null after a viewer called init' };
        const px = new Uint8Array(4);
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.readPixels(Math.floor(gl.drawingBufferWidth / 2), Math.floor(gl.drawingBufferHeight / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
        return { ok: gl.canvas === api.render.gl.canvas && api.render.gl.context === gl && px[3] === 255 && gl.getContextAttributes()?.preserveDrawingBuffer === true, detail: Array.from(px) };
      });
      check('render.raster sync', () => {
        const r: any = api.render.raster(text, 1.5, t);
        const today: any = pm.__rasterBase(text, 1.5, t);
        const geometry = (s: any) => s && { w: s.w, h: s.h, cw: s.cv.width, ch: s.cv.height, anchorX: s.anchorX, anchorY: s.anchorY };
        return { ok: !!r && !(r instanceof Promise) && near(geometry(r), geometry(today)) && near(json(r.selection ?? null), json(today.selection ?? null)) && near(r.fontOffset ?? null, today.fontOffset ?? null), detail: [geometry(r), geometry(today)] };
      });
      check('render.textLayout', () => { const a: any = api.render.textLayout(text, t), b: any = pm.textCaretLayout(text, t); return { ok: !!a && a.lines.length === b.lines.length && a.lines.every((l: any, i: number) => l.text === b.lines[i].text && Math.abs(l.width - b.lines[i].width) <= 0.25), detail: a?.lines?.map((l: any) => l.width) }; });
      let engineFrame: HTMLCanvasElement | null = null;
      check('render.renderFrameTo sync, pixels as today', () => {
        const c = api.render.renderFrameTo(t, 240, 135, { mblur: false });
        engineFrame = c;
        const today = pm.renderFrameTo(t, 240, 135, { mblur: false });
        const a = pixels(c), b = pixels(today);
        // The title's rows (y 20..60 at this size) are text-render parity,
        // gated strictly by the typewriter test; reported here, the rest asserted.
        const band = (from: number, to: number) => compare(a.subarray(from * 240 * 4, to * 240 * 4), b.subarray(from * 240 * 4, to * 240 * 4));
        const rest = [band(0, 20), band(60, 135)];
        const d = { above: rest[0], below: rest[1], title: band(20, 60), whole: compare(a, b) };
        return { ok: c instanceof HTMLCanvasElement && !(c as any).then && c.width === 240 && c.height === 135 && rest.every(same), detail: d };
      });
      await checkAsync('render.snapshot sync, the engine frame as today\'s JPEG', async () => {
        const url = api.render.snapshot(t, 240);
        if (typeof url !== 'string' || !url.startsWith('data:image/jpeg')) return { ok: false, detail: typeof url };
        const ours = await decode(url);
        const reference = await decode((engineFrame ?? api.render.renderFrameTo(t, 240, 135, { mblur: false })).toDataURL('image/jpeg', 0.74));
        const d = compare(ours, reference);
        return { ok: ours.length === 240 * 135 * 4 && d.max === 0, detail: d };
      });
      await checkAsync('async siblings equal the sync members', async () => {
        const a = await api.render.renderFrameToAsync(t, 240, 135, { mblur: false });
        const d = compare(pixels(a), pixels(engineFrame!));
        const s = await api.render.snapshotAsync(t, 240);
        const r: any = await api.render.rasterAsync(text, 1.5, t);
        const p = await api.project.snapshot(t, 240);
        return { ok: d.max === 0 && s === api.render.snapshot(t, 240) && p === s && !!r && r.cv?.width > 0, detail: d };
      });

      /* ── services ── */
      check('services.raster: captured, wrapped, disposed', () => {
        const original: any = api.services.get('raster');
        if (typeof original !== 'function') return { ok: false, detail: typeof original };
        let calls = 0;
        const registration = api.services.register('raster', (...args: any[]) => { calls++; return original(...args); });
        const viaRender = api.render.raster(text, 1, t);
        const wrapped = api.services.get('raster') !== original;
        registration.dispose();
        return { ok: wrapped && api.services.get('raster') === original && !!viaRender, detail: { calls } };
      });
      check('services viewer/shaderHooks/compositions', () => ({ ok: !!api.services.get('viewer') && !!api.services.get('shaderHooks'), detail: ['viewer', 'shaderHooks', 'compositions', 'captions', 'tool', 'timeline'].map((n) => [n, typeof api.services.get(n)]) }));
      check('uiState.getShaderMeta', () => { const m = api.uiState.getShaderMeta(text); return { ok: m === null || typeof m === 'object', detail: json(m) }; });

      /* ── media.* and assets.* live fields ── */
      const assetOf = (layerId?: string) => { const l = byId(layerId); return l ? (api.media.assets.get(l.d?.asset ?? l.d?.src ?? l.asset) as any) : undefined; };
      check('media.timing', () => {
        const v = byId(fixture.video);
        if (!v) return { ok: false, detail: 'no video fixture' };
        const today = pm.MediaTiming;
        const cut = v.from + 0.25;
        const parts = { timed: [api.media.timing.isTimed(v), today?.isTimed?.(v)], rate: [api.media.timing.rate(v), today?.rate?.(v)], earliest: [api.media.timing.earliestStart(v), today?.earliestStart?.(v)], start: [json(api.media.timing.startPatch(v, v.from + 0.1)), json(today?.startPatch?.(v, v.from + 0.1))], end: [json(api.media.timing.endPatch(v, cut)), json(today?.endPatch?.(v, cut))] };
        return { ok: !!today && Object.values(parts).every(([a, b]) => near(a, b)) && parts.timed[0] === true, detail: parts };
      });
      check('media.commandForAsset', () => { const asset: any = assetOf(fixture.image); const c: any = asset ? api.media.commandForAsset(asset.id, 0) : null; return { ok: !!c && c.type === 'add_layer', detail: json(c) }; });
      check('media.fonts', () => { const f: any = api.media.fonts; return { ok: !!f && typeof f === 'object', detail: Object.keys(f ?? {}).slice(0, 6) }; });
      check('media.audio.drawWaveform', () => {
        const a = byId(fixture.audio);
        const c = document.createElement('canvas'); c.width = 200; c.height = 40;
        const drew = a ? api.media.audio.drawWaveform(c.getContext('2d')!, a) : null;
        return { ok: typeof drew === 'boolean', detail: drew };
      });
      check('assets.get live fields: image', () => {
        const a = assetOf(fixture.image);
        // Today: a decoded ImageBitmap (an <img> when decoding fails), and an object URL.
        const width = a?.el instanceof ImageBitmap ? a.el.width : a?.el instanceof HTMLImageElement ? a.el.naturalWidth : 0;
        return { ok: !!a && typeof a.url === 'string' && a.url.length > 0 && width > 0 && (api.assets.get(a.id) as any)?.id === a.id, detail: a && { url: String(a.url).slice(0, 12), el: a.el?.constructor?.name, width } };
      });
      check('assets.get live fields: video', () => {
        const a = assetOf(fixture.video);
        const peaks = a?.peaks == null || Array.isArray(a.peaks) || ArrayBuffer.isView(a.peaks);
        return { ok: !!a && typeof a.url === 'string' && a.el instanceof HTMLVideoElement && a.el.src === a.url && a.el.videoWidth > 0 && Number.isFinite(a.el.duration) && a.audioBlob instanceof Blob && a.audioBlob.size > 0 && peaks && (a.audioDecodeError ?? null) === null, detail: a && { url: typeof a.url, el: a.el?.constructor?.name, vw: a.el?.videoWidth, dur: a.el?.duration, audioBlob: a.audioBlob?.size, peaks: a.peaks?.length, error: a.audioDecodeError ?? null } };
      });
      check('assets.get live fields: audio', () => {
        const a = assetOf(fixture.audio);
        // Today an audio asset has its Blob and peaks (no element or URL).
        const peaks = Array.isArray(a?.peaks) || ArrayBuffer.isView(a?.peaks);
        return { ok: !!a && a.audioBlob instanceof Blob && a.audioBlob.size > 0 && peaks && a.peaks.length > 0 && (a.audioDecodeError ?? null) === null, detail: a && { url: typeof a.url, audioBlob: a.audioBlob?.size, peaks: a.peaks?.length } };
      });
      check('assets.get live fields: model', () => {
        const a: any = fixture.modelAsset ? api.media.assets.get(fixture.modelAsset) : undefined;
        const o = a?.object3d;
        let meshes = 0;
        o?.traverse?.((n: any) => { if (n.isMesh) meshes++; });
        return { ok: !!o && typeof o.traverse === 'function' && meshes > 0, detail: { object3d: o?.type, meshes } };
      });
      check('assets.get missing, kind', () => ({ ok: api.assets.get('missing') === undefined && api.media.assets.kind(new File([''], 'a.png', { type: 'image/png' })) === 'image' && api.media.assets.kind(new File([''], 'a.wav', { type: 'audio/wav' })) === 'audio' }));
      await checkAsync('media.assets.source/audio', async () => {
        const v = assetOf(fixture.video);
        const source: any = v ? await api.media.assets.source(v.id, { timeoutMs: 10_000 }) : null;
        const audio = v ? await api.media.assets.audio(v.id, { timeoutMs: 10_000 }) : null;
        return { ok: !!source && typeof source.url === 'string' && audio instanceof Blob, detail: { source: source && Object.keys(source), audio: audio?.size } };
      });

      /* ── scene3d ── */
      check('scene3d reads', () => {
        const s = api.scene3d;
        const rendering: any = s.getRendering();
        const state: any = s.previewState();
        const view = s.getView();
        const auto = s.getAutoKey();
        const three = byId(fixture.three);
        const described: any = three ? s.describe(three.id) : null;
        return { ok: !!rendering && ['draft', 'rendered'].includes(state.mode) && ['camera', 'editor'].includes(view) && typeof auto === 'boolean' && (!three || !!described), detail: { view, mode: state.mode, described: !!described } };
      });

      /* ── EDITOR_HELPER_EXPORTS from this compiled bundle ── */
      check('editor helpers: every export, from the bundle', () => {
        const expected = fixture.helperNames ?? [];
        const missing = expected.filter((n) => (helpers as any)[n] === undefined);
        return { ok: expected.length > 0 && missing.length === 0, detail: { expected: expected.length, missing } };
      });
      check('editor helpers: evaluation shims answer as today', () => {
        const legacyPm = { ...pm, EngineDoc: undefined };
        const pairs: Array<[string, unknown, unknown]> = [
          ['resolveContent', helpers.resolveContent(pm, text, t), helpers.resolveContent(legacyPm, text, t)],
          ['evaluatedValue', helpers.evaluatedValue(pm, text, text.p.opacity, t, 'opacity'), helpers.evaluatedValue(legacyPm, text, text.p.opacity, t, 'opacity')],
          ['is3DLayer', helpers.is3DLayer(pm, text), helpers.is3DLayer(legacyPm, text)],
          ['planeMatrix', helpers.planeMatrix(pm, text, t), helpers.planeMatrix(legacyPm, text, t)],
          ['countTextUnits', helpers.countTextUnits([{ text: 'ab cd' }] as any, 'words'), 2],
          ['isProperty', helpers.isProperty(text.p.opacity), true],
          ['cueIndexAt', helpers.cueIndexAt([{ start: 0, end: 1 }, { start: 1, end: 2 }] as any, 1.5), 1]
        ];
        const bad = pairs.filter(([, a, b]) => !near(json(a), json(b))).map(([n, a, b]) => ({ n, a: json(a), b: json(b) }));
        return { ok: bad.length === 0, detail: bad };
      });

      /* ── events: today's names, payloads and order ── */
      await checkAsync('events', async () => {
        const from = log.length;
        api.project.select([]);
        api.project.select([text.id]);
        api.transport.setTime(0.6);
        api.project.apply({ type: 'set_property', target: text.id, path: 'opacity', value: 100 } as any);
        api.transport.invalidate();
        const frame = await until('frame:rendered', from);
        const names = log.slice(from).map((e) => e.name);
        const timeEvent = log.slice(from).find((e) => e.name === 'time');
        const selection: any = log.slice(from).find((e) => e.name === 'selection')?.payload;
        const fr = log.slice(from).find((e) => e.name === 'frame:rendered')?.payload as any;
        const seen = [...new Set(log.map((e) => e.name))].sort();
        // Today's order: the time event before the frame drawn for it.
        const ok = frame && names.includes('selection') && Array.isArray(selection?.layers) && timeEvent?.payload === 0.6
          && names.includes('invalidate') && names.includes('overlay') && names.includes('project:changed')
          && Number.isFinite(fr?.time) && Number.isFinite(fr?.quality) && frameViewport?.matches === true
          && names.indexOf('time') < names.lastIndexOf('frame:rendered');
        return { ok, detail: { seen, order: names.slice(0, 30), frame: fr, frameViewport } };
      });

      // Last: today's undo restores the project's objects from history (the
      // layer objects held above are then no longer the project's).
      check('project.apply/undo/redo', () => {
        const count = () => api.project.get().layers.length;
        const before = count();
        const result: any = api.project.apply({ type: 'add_layer', layerType: 'solid', name: 'Probe solid', duration: 1 } as any);
        const added = count();
        api.project.undo();
        const undone = count();
        api.project.redo();
        const redone = count();
        api.project.undo();
        return { ok: !!result && !(result instanceof Promise) && result.ok === true && added === before + 1 && undone === before && redone === before + 1 && count() === before && api.project.get() === pm.proj, detail: { before, added, undone, redone } };
      });

      const record = { apiVersion: api.manifest.apiVersion, out, events: [...new Set(log.map((e) => e.name))].sort() };
      (window as any).__compatProbe ??= {};
      (window as any).__compatProbe[api.id] = record;
      return record;
    }
  });
}
