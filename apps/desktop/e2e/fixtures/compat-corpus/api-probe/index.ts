import type { PowermoveAPI } from 'powermove';
import { resolveContent, evaluatedValue, is3DLayer, planeMatrix, countTextUnits, CAPTION_PRESETS, EXPRESSION_NAMES, CHANNELS_3D } from 'powermove';

/*
 * The compatibility corpus's API probe (owner amendment 24). The same source
 * runs at apiVersion 1, 2 and 3 (the test writes the three manifests). The
 * command `<id>.run` calls each member as code written before the engine
 * does, synchronously where today's API is synchronous, and records the
 * shape and values it got in `window.__compatProbe[<id>]`, so the test
 * asserts behavior, not only that the names exist.
 */
type Result = { ok: boolean; detail?: unknown };

export default function activate(api: PowermoveAPI): void {
  const events: Record<string, number> = {};
  let frameViewportMatches: boolean | null = null;
  for (const name of ['project:changed', 'selection', 'time', 'transport', 'fonts', 'layout', 'frame:rendered', 'overlay', 'invalidate', 'theme:changed', 'extensions:changed'] as const) {
    api.events.on(name as any, (payload: any) => {
      events[name] = (events[name] ?? 0) + 1;
      if (name === 'frame:rendered') frameViewportMatches = payload?.viewport === api.render.gl.previewViewport;
    });
  }
  api.commands.register({
    id: `${api.id}.run`,
    label: 'Run API probe',
    run: async () => {
      const out: Record<string, Result> = {};
      const check = (name: string, fn: () => Result) => {
        try { out[name] = fn(); } catch (error) { out[name] = { ok: false, detail: String((error as Error)?.message ?? error) }; }
      };
      const project = api.project.get();
      const layer: any = project.layers.find((l: any) => l.type === 'text') ?? project.layers[0];
      const t = 0.5;
      check('project.get live', () => ({ ok: api.project.get() === project && api.model.curComp() === project }));
      check('project.exportDefaults today ids', () => ({ ok: ['prores', 'mp4', 'webm', 'png', 'still', 'web', 'json', 'rec'].includes(String((project as any).exportDefaults?.format)), detail: (project as any).exportDefaults?.format }));
      check('render.snapshot sync', () => { const url = api.render.snapshot(t, 160); return { ok: typeof url === 'string' && url.startsWith('data:image/jpeg'), detail: typeof url }; });
      check('render.renderFrameTo sync', () => { const c = api.render.renderFrameTo(t, 160, 90, { mblur: false }); return { ok: c instanceof HTMLCanvasElement && c.width === 160 && c.height === 90 }; });
      check('render.raster sync', () => {
        const r: any = api.render.raster(layer, 1, t);
        return { ok: !!r && !(r instanceof Promise) && r.cv?.width > 0 && Number.isFinite(r.w) && Number.isFinite(r.anchorX), detail: r && { w: r.w, h: r.h } };
      });
      check('render.gl.context', () => { const gl = api.render.gl.context; return { ok: gl === null || (gl.canvas === api.render.gl.canvas), detail: gl ? 'context' : 'null' }; });
      check('render.textLayout', () => { const l = api.render.textLayout(layer, t); return { ok: !!l && Array.isArray(l.lines) && l.lines.length > 0 }; });
      check('render.gl.bounds', () => { const b: any = api.render.gl.bounds(layer, t); return { ok: !!b && Number.isFinite(b.x0 ?? b.x ?? NaN) }; });
      check('anim.ev', () => { const v = api.anim.ev(layer, 'opacity', t); return { ok: v === 100, detail: v }; });
      check('anim.worldMatrix', () => { const m = api.anim.worldMatrix(layer, t); return { ok: Array.isArray(m) && m.length === 6 && m.every(Number.isFinite) }; });
      check('anim.resolveContent', () => { const d: any = api.anim.resolveContent(layer, t); return { ok: d?.text === layer.d.text }; });
      check('services.raster', () => ({ ok: typeof api.services.get('raster') === 'function' }));
      check('assets.get', () => ({ ok: api.assets.get('missing') === undefined }));
      check('transport', () => ({ ok: typeof api.transport.time() === 'number' && typeof api.transport.playing() === 'boolean' }));
      const pm = api.host.pm as any;
      check('helpers.resolveContent', () => { const d: any = resolveContent(pm, layer, t); return { ok: d?.text === layer.d.text, detail: d?.text }; });
      check('helpers.evaluatedValue', () => { const v = evaluatedValue(pm, layer, layer.p.opacity, t, 'opacity'); return { ok: v === 100, detail: v }; });
      check('helpers.is3DLayer', () => { const v = is3DLayer(pm, layer); return { ok: v === false, detail: v }; });
      check('helpers.planeMatrix', () => { const m: any = planeMatrix(pm, layer, t); return { ok: !!m && typeof m === 'object' && Object.values(m).length >= 6, detail: Array.isArray(m) ? m.length : typeof m }; });
      check('helpers.countTextUnits', () => { const n = countTextUnits([{ text: 'ab cd' }], 'words'); return { ok: n === 2, detail: n }; });
      check('helpers.constants', () => ({ ok: !!CAPTION_PRESETS && typeof CAPTION_PRESETS === 'object' && !!EXPRESSION_NAMES && !!CHANNELS_3D, detail: [typeof CAPTION_PRESETS, Array.isArray(EXPRESSION_NAMES) ? 'array' : typeof EXPRESSION_NAMES, Array.isArray(CHANNELS_3D) ? 'array' : typeof CHANNELS_3D] }));
      const snapshot = await api.project.snapshot(t, 160);
      out['project.snapshot async'] = { ok: typeof snapshot === 'string' && snapshot.startsWith('data:image/jpeg') };
      const later = await api.render.snapshotAsync(t, 160);
      out['render.snapshotAsync'] = { ok: typeof later === 'string' && later.startsWith('data:image/jpeg') };
      const raster: any = await api.render.rasterAsync(layer, 1, t);
      out['render.rasterAsync'] = { ok: !!raster && raster.cv?.width > 0 };
      out['events'] = { ok: true, detail: { ...events, frameViewportMatches } };
      (window as any).__compatProbe ??= {};
      (window as any).__compatProbe[api.id] = { apiVersion: api.manifest.apiVersion, out };
      return true;
    }
  });
}
