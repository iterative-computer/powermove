import type { ChannelValue, EditCommand, Layer, PowermoveAPI } from 'powermove';
import { inspectorTargets } from './multi-edit';
import type { InspectorAPI } from './multi-edit';

type SelectionAnimationAPI = InspectorAPI & {
  anim: InspectorAPI['anim'] & Pick<PowermoveAPI['anim'], 'hasKeyAt' | 'removeKey' | 'setKeyOn'>;
  history: Pick<PowermoveAPI['history'], 'do'>;
  project: Pick<PowermoveAPI['project'], 'get'>;
};

export function selectionChannels(api: InspectorAPI, layer: Layer, paths: string[]) {
  return paths.flatMap((path) => inspectorTargets(api, layer, path));
}

export function animateSelection(
  api: InspectorAPI,
  layer: Layer,
  paths: string[],
  disable: boolean,
  time: number,
  label: string,
  fallback?: unknown
) {
  const commands: EditCommand[] = [];
  for (const target of selectionChannels(api, layer, paths)) {
    const value = target.prop ? api.anim.evP(target.layer, target.prop, time, target.path) : fallback;
    if (value === undefined) continue;
    if (disable) commands.push({ type: 'replace_keyframes', target: target.layer.id, path: target.path, keyframes: [], preserveHandEdits: false });
    commands.push({ type: 'set_property', target: target.layer.id, path: target.path, value: value as ChannelValue, time, mode: disable ? 'static' : 'keyframe', preserveHandEdits: false });
  }
  return api.edit.apply(commands, { label, origin: 'inspector' });
}

/** Add/remove only the playhead keys, preserving all other animation. */
export function toggleSelectionKey(
  api: SelectionAnimationAPI,
  layer: Layer,
  paths: string[],
  time: number,
  label: string,
  fallback?: unknown
) {
  const targets = selectionChannels(api, layer, paths);
  const remove = targets.length > 0 && targets.every((target) => target.prop && api.anim.hasKeyAt(target.layer, target.prop, time));
  if (targets.some((target) => !target.prop) || targets.every((target) => !target.prop?.kf.length)) {
    return animateSelection(api, layer, paths, false, time, label, fallback);
  }
  return api.history.do(label, () => {
    for (const target of targets) {
      const prop = target.prop!;
      const at = api.anim.hasKeyAt(target.layer, prop, time);
      if (remove && at) api.anim.removeKey(prop, at);
      else if (!at) api.anim.setKeyOn(prop, time - target.layer.from, api.anim.evP(target.layer, prop, time, target.path)!, 'linear', api.project.get().fps);
    }
  });
}
