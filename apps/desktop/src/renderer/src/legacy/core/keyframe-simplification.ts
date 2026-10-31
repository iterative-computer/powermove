import { simplifyKeyframes } from '../../core/anim/simplify-keyframes';
import type { PMRegistry } from '../registry';

export const DENSE_KEYFRAME_THRESHOLD = 200;
export type PropertyRef = { layerId: string; path: string };

export function projectCompositions(project: any): any[] {
  const result: any[] = [], seen = new Set<any>();
  const visit = (comp: any) => {
    if (!comp || seen.has(comp)) return;
    seen.add(comp); result.push(comp);
    Object.values(comp.comps || {}).forEach(visit);
  };
  visit(project);
  return result;
}

export function simplificationLayer(PM: PMRegistry, id: string): any {
  return projectCompositions(PM.proj).flatMap(comp => comp.layers || []).find(layer => layer.id === id);
}

export function simplifyProperties(PM: PMRegistry, refs: PropertyRef[]) {
  const comps = projectCompositions(PM.proj);
  const layers = comps.flatMap(comp => comp.layers || []);
  // Expressions receive the layer/comp objects and can inspect OTHER tracks.
  // Until dependency analysis exists, retain all keys if any expression exists.
  const expressionConsumer = layers.some(layer => PM.allProps(layer).some((item: any) => item.prop?.expr));
  const nestedOrMapped = comps.length > 1 || layers.some(layer => layer.type === 'precomp'
    || layer.d?.timeRemap || layer.d?.stretch != null || layer.timeStretch != null);
  const summary = { keysBefore: 0, keysAfter: 0, propertiesChanged: 0, propertiesSkipped: 0 };
  for (const ref of refs) {
    const comp = comps.find(comp => comp.layers?.some((layer: any) => layer.id === ref.layerId));
    const layer = comp?.layers.find((layer: any) => layer.id === ref.layerId);
    const prop = layer && PM.findProp(layer, ref.path);
    if (!Array.isArray(prop?.kf)) continue;
    summary.keysBefore += prop.kf.length;
    const locked = layer.lock || (PM.groupAncestors?.(layer, layers) || []).some((group: any) => group.lock);
    // Structured paths, text animator selectors and extension-owned tracks have
    // their own evaluators/consumers. Keep them rather than guessing semantics.
    const supported = !/^(g|mp|ta|ts|u|x|transitionIn|transitionOut)\./.test(ref.path)
      && !['extension', 'shader', 'audio', 'precomp'].includes(layer.type)
      && typeof prop.v === 'number';
    const parentRig = layer.type === 'group' || layers.some(other => other.parent === layer.id || other.group === layer.id || other.matteSource === layer.id);
    const result = expressionConsumer || locked || !supported ? prop.kf : simplifyKeyframes(prop.kf, {
      evaluate: (keys, time) => PM.evalKfs(keys, time),
      expression: prop.expr, fps: comp.fps || PM.proj.fps, from: layer.from,
      duration: layer.dur, compositionDuration: comp.dur, shutter: comp.shutter,
      trimOutside: !nestedOrMapped && !parentRig && !layer.threeD && !!layer.p?.[ref.path],
    });
    if (result !== prop.kf) { prop.kf = result; summary.propertiesChanged++; }
    else if (prop.kf.length > 2) summary.propertiesSkipped++;
    summary.keysAfter += prop.kf.length;
  }
  PM.touch();
  return summary;
}

export function allPropertyRefs(PM: PMRegistry): PropertyRef[] {
  return projectCompositions(PM.proj).flatMap(comp => (comp.layers || []).flatMap((layer: any) =>
    PM.allProps(layer).map((item: any) => ({ layerId: layer.id, path: item.key }))));
}

type KeySnapshot = Map<string, string[]>;
const refKey = (ref: PropertyRef) => JSON.stringify([ref.layerId, ref.path]);
const signatures = (prop: any): string[] => (prop?.kf || []).map((key: any) => JSON.stringify(key));

export function snapshotKeyframes(PM: PMRegistry): KeySnapshot {
  const result: KeySnapshot = new Map();
  for (const comp of projectCompositions(PM.proj)) for (const layer of comp.layers || []) {
    for (const { key, prop } of PM.allProps(layer)) result.set(refKey({ layerId: layer.id, path: key }), signatures(prop));
  }
  return result;
}

/** Called only AFTER successful edits/imports. Neither detection nor dismissal edits animation. */
export function noticeDenseKeyframes(PM: PMRegistry, before: KeySnapshot = new Map(), writes = new Map<string, number>()) {
  const refs: PropertyRef[] = [];
  let total = 0;
  for (const ref of allPropertyRefs(PM)) {
    const layer = simplificationLayer(PM, ref.layerId), prop = layer && PM.findProp(layer, ref.path);
    if (!prop?.kf || prop.kf.length <= DENSE_KEYFRAME_THRESHOLD) continue;
    const key = refKey(ref), previous = new Set(before.get(key) || []);
    const changed = signatures(prop).filter(signature => !previous.has(signature)).length;
    const count = Math.max(changed, writes.get(key) || 0);
    if (count > DENSE_KEYFRAME_THRESHOLD) { refs.push(ref); total += count; }
  }
  if (!refs.length) return null;
  const projectId = PM.proj.id, boundary = PM.agentProjectBoundary || 0;
  const expected = refs.map(ref => signatures(PM.findProp(simplificationLayer(PM, ref.layerId), ref.path)));
  let pending = true;
  PM.toast?.(`This edit wrote ${total.toLocaleString()} keyframes. Simplify repeated values? The animation won't change.`, 0, {
    key: `dense-keyframes:${projectId}`, sticky: true, dismissible: true, corner: 'top-right', error: false,
    dismissLabel: 'Keep as is',
    onClose: () => { pending = false; },
    action: { label: 'Simplify', run: () => {
      if (!pending) return;
      pending = false;
      if (PM.Edit.isLive?.()) { PM.toast('Finish the current edit first. Nothing was simplified.', 4000, { error: false }); return; }
      if (PM.proj.id !== projectId || (PM.agentProjectBoundary || 0) !== boundary
        || refs.some((ref, index) => {
          const layer = simplificationLayer(PM, ref.layerId), prop = layer && PM.findProp(layer, ref.path);
          return JSON.stringify(signatures(prop)) !== JSON.stringify(expected[index]);
        })) { PM.toast('The animation has changed since this edit, so nothing was simplified.', 4000, { error: false }); return; }
      const result = PM.Edit.apply({ type: 'simplify_keyframes', scope: 'properties', properties: refs },
        { label: 'Simplify Keyframes', origin: 'interface' });
      PM.toast(result.ok ? `${result.data.results[0].data.keysBefore.toLocaleString()} → ${result.data.results[0].data.keysAfter.toLocaleString()} keyframes` : result.message, 4000, { error: !result.ok });
    } },
  });
  return `This edit wrote ${total} keyframes on dense properties. The user was offered Simplify / Keep as is; nothing was simplified. Prefer fewer keyframes or expressions instead of baking every frame.`;
}

export function keyframeWrites(executed: any[]): Map<string, number> {
  const result = new Map<string, number>();
  for (const { command, data } of executed) {
    if (!['set_property', 'replace_keyframes'].includes(command.type) || !data?.id || !data?.channel) continue;
    const key = refKey({ layerId: data.id, path: data.channel });
    const count = command.type === 'replace_keyframes' ? command.keyframes.length : command.mode === 'static' ? 0 : 1;
    result.set(key, (result.get(key) || 0) + count);
  }
  return result;
}
