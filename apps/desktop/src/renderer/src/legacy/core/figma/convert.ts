import type { PMRegistry } from '../../registry';
import type { FigmaSelection } from './clipboard';
import { clipboardDoc } from './adapter';
import { convertFigma, type FigmaPlan as NeutralPlan, type FigmaSource } from '../../../../../shared/figma/convert';
import { materializeSvgPaths } from '../svg-import';
import { isProperty } from '../content-properties';
export type FigmaPlan = NeutralPlan & { layers: any[]; roots: string[] };
export function materializeFigma(PM: PMRegistry, plan: NeutralPlan, at: number): FigmaPlan {
  const ids = new Map(plan.commands.map((c) => [c.ref, PM.uid('L')]));
  const layers = plan.commands.map((c) => {
    const d = { ...c.content };
    if (d.paths) d.paths = materializeSvgPaths(PM, { width: d.w ?? 0, height: d.h ?? 0, paths: d.paths, warnings: [] });
    if (d.styles)
      d.styles = d.styles.map((s: any) => ({
        id: PM.uid('style'),
        start: s.start,
        end: s.end,
        p: Object.fromEntries(Object.entries(s.values).map(([k, v]) => [k, PM.P(v)])),
      }));
    const from = PM.snapF(at, PM.proj.fps),
      layer = PM.mkLayer(c.layerType, {
        name: c.name,
        from,
        dur: Math.max(1 / PM.proj.fps, PM.proj.dur - from),
        d,
        p: c.properties,
      });
    layer.id = ids.get(c.ref);
    layer.group = c.group ? ids.get(c.group) : null;
    layer.on = c.visible;
    layer.lock = c.locked;
    layer.blend = c.blend;
    layer.collapsed = false;
    if (c.matte) {
      layer.matteSource = ids.get(c.matte);
      layer.matteMode = PM.P('alpha');
    }
    layer.fx = c.effects.map((e) => {
      const effect = PM.mkEffect?.(e.type, e.version, layer);
      if (!effect) throw new Error(`The ${e.type} effect is unavailable`);
      for (const [k, v] of Object.entries(e.params)) effect.p[k] = PM.P(v);
      return effect;
    });
    layer.figmaSource = {
      ...c.source,
      importedMatteId: layer.matteSource ?? null,
      importedEffectIds: layer.fx.map((f: any) => f.id),
    };
    return layer;
  });
  return { ...plan, layers, roots: layers.filter((l) => !l.group).map((l) => l.id) };
}
export function planFigmaImport(PM: PMRegistry, selection: FigmaSelection, at: number): FigmaPlan {
  return materializeFigma(PM, convertFigma(clipboardDoc(selection), PM.proj), at);
}
const key = (s: FigmaSource) => `${s.fileKey}\0${s.nodeId}\0${s.role}`;
export function figmaMatches(PM: PMRegistry, plan: FigmaPlan): any[] {
  const keys = new Set(plan.layers.map((l) => key(l.figmaSource)));
  return PM.proj.layers.filter((l: any) => l.figmaSource && keys.has(key(l.figmaSource)));
}
/** Reconcile content while retaining all authored animation, timing, parenting,
 * renamed layers and effects not owned by this importer. No source-layer deletion. */
function mergeValue(old: any, next: any): any {
  if (isProperty(old)) return old.kf.length || old.expr ? old : { ...old, v: isProperty(next) ? next.v : next };
  if (Array.isArray(old) && Array.isArray(next)) return next.map((v, i) => mergeValue(old[i], v));
  if (old && next && typeof old === 'object' && typeof next === 'object')
    return Object.fromEntries(
      Object.entries(next).map(([k, v]) => [k, k === 'id' && old[k] ? old[k] : mergeValue(old[k], v)]),
    );
  return next;
}
export function applyFigmaPlan(
  PM: PMRegistry,
  plan: FigmaPlan,
  update = false,
  assets: Record<string, any> = {},
): { missing: string[] } {
  if (plan.commands.some((c) => c.layerType === 'image' && !c.raster))
    throw new Error('A Figma layer needs a raster fallback. Use Send to Powermove in Figma.');
  const missing: string[] = [];
  const result = PM.Edit.mutate(
    update ? 'Update from Figma' : 'Paste Figma layers',
    () => {
      const bySource = new Map<string, any>();
      for (const l of PM.proj.layers) if (l.figmaSource) bySource.set(key(l.figmaSource), l);
      const idMap = new Map<string, string>();
      if (update)
        for (const l of plan.layers) {
          const old = bySource.get(key(l.figmaSource));
          if (old) idMap.set(l.id, old.id);
        }
      const incoming = new Set(plan.layers.map((l) => key(l.figmaSource)));
      const files = new Set(plan.layers.map((l) => l.figmaSource.fileKey));
      if (update)
        for (const old of PM.proj.layers)
          if (old.figmaSource && files.has(old.figmaSource.fileKey) && !incoming.has(key(old.figmaSource)))
            missing.push(old.name);
      const added: any[] = [];
      for (const layer of plan.layers) {
        if (layer.group) layer.group = idMap.get(layer.group) ?? layer.group;
        if (layer.matteSource) layer.matteSource = idMap.get(layer.matteSource) ?? layer.matteSource;
        layer.figmaSource.importedMatteId = layer.matteSource ?? null;
        const old = update && bySource.get(key(layer.figmaSource));
        if (!old) {
          added.push(layer);
          continue;
        }
        const owned = new Set(old.figmaSource.importedEffectIds ?? []),
          oldEffects = old.fx.filter((fx: any) => !owned.has(fx.id));
        if (old.name === old.figmaSource.name) old.name = layer.name;
        old.blend = mergeValue(old.blend, layer.blend);
        if ((old.matteSource ?? null) === (old.figmaSource.importedMatteId ?? null))
          old.matteSource = layer.matteSource ?? null;
        old.type = layer.type;
        old.d = { ...old.d, ...mergeValue(old.d, layer.d) };
        for (const [k, p] of Object.entries(layer.p)) old.p[k] = mergeValue(old.p[k], p);
        const previous = old.fx.filter((fx: any) => owned.has(fx.id));
        const imported = layer.fx.map((fx: any, i: number) =>
          previous[i]?.type === fx.type ? { ...fx, id: previous[i].id, p: mergeValue(previous[i].p, fx.p) } : fx,
        );
        old.fx = [...imported, ...oldEffects];
        old.figmaSource = { ...layer.figmaSource, importedEffectIds: imported.map((fx: any) => fx.id) };
      }
      // Anchor each new layer to the nearest surviving incoming sibling, preserving
      // user stack edits on existing layers and unrelated Powermove layers.
      for (const layer of [...added].reverse()) {
        const index = plan.layers.indexOf(layer);
        const next = plan.layers
          .slice(index + 1)
          .find((l) => PM.proj.layers.some((e: any) => e.id === (idMap.get(l.id) ?? l.id)));
        const previous = plan.layers
          .slice(0, index)
          .reverse()
          .find((l) => PM.proj.layers.some((e: any) => e.id === (idMap.get(l.id) ?? l.id)));
        const target = next
          ? PM.proj.layers.findIndex((l: any) => l.id === (idMap.get(next.id) ?? next.id))
          : previous
            ? PM.proj.layers.findIndex((l: any) => l.id === (idMap.get(previous.id) ?? previous.id)) + 1
            : 0;
        PM.proj.layers.splice(Math.max(0, target), 0, layer);
      }
      PM.ProjectIndex?.invalidate();
      PM.selectLayers(plan.roots.map((id) => idMap.get(id) ?? id));
    },
    { origin: 'import', importedAssets: assets },
  );
  if (!result.ok) throw new Error(result.message);
  return { missing };
}
