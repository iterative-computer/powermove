/* Ported from js/core/editing.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const MAX_EDITS: any = 200;
const FORBIDDEN_KEYS: any = new Set(['__proto__', 'prototype', 'constructor']);
const AUDIO_CONTENT_FIELDS: any = new Set(['asset', 'trim', 'gain', 'fadeIn', 'fadeOut']);
/* Trust classification for every command origin. Trust decisions use the RAW
   meta.origin — an unset or unknown origin gets no privileges (no overrideLock,
   no unlock exception) while keeping its own preserveHandEdits semantics.
   Provenance recording still defaults to 'interface' elsewhere. */
const ORIGIN_TRUST: any = Object.freeze({
  interface: 'human', inspector: 'human', canvas: 'human', timeline: 'human',
  command: 'human', 'command-palette': 'human', 'effects-panel': 'human',
  'shader-panel': 'human', library: 'human', import: 'human',
  agent: 'generated', 'generated-ui': 'generated',
  'generated-tool': 'generated', 'generated-script': 'generated',
});
const isHumanOrigin: any = (origin: any) => ORIGIN_TRUST[origin] === 'human';
const isGeneratedOrigin: any = (origin: any) => ORIGIN_TRUST[origin] === 'generated';
/* Derived from Edit.operations so a new layer-targeted op is lock-checked by
   construction instead of relying on a second hand-maintained list. */
let lockedLayerOpsCache: any = null;
const lockedLayerOps: any = () => lockedLayerOpsCache || (lockedLayerOpsCache = new Set(
  Object.entries(Edit.operations).filter(([, def]: any) => def.target === 'layer').map(([type]: any) => type)));
const LAYER_FIELDS: any = new Set([
  'name', 'from', 'duration', 'visible', 'locked', 'solo', 'shy', 'blend',
  'motionBlur', 'parent', 'color', 'collapsed', 'scaleLinked',
]);
let live: any = null;

const clone: any = (value: any) => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
const fail: any = (message: any) => ({ ok: false, message });
const pass: any = (message: any, data: any = {}) => ({ ok: true, message, data });
const commands: any = (value: any) => Array.isArray(value) ? value : [value];
const finite: any = (value: any, label: any) => {
  const number: any = Number(value);
  if (!Number.isFinite(number)) throw new Error(`${label} must be a finite number`);
  return number;
};
const safePatch: any = (value: any, label: any) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} must be an object`);
  const out: any = {};
  for (const [key, child] of Object.entries(value)) {
    if (FORBIDDEN_KEYS.has(key)) throw new Error(`Unsafe ${label} key: ${key}`);
    out[key] = clone(child);
  }
  return out;
};
const findLayer: any = (ref: any) => {
  if (!ref) return PM.firstSel();
  if (ref === '$selection' || ref === 'selection') return PM.firstSel();
  if (typeof ref === 'number') return PM.proj.layers[ref - 1] || null;
  return PM.L(ref) || PM.byName(ref);
};
const channelPath: any = (path: any) => String(path || '')
  .replace(/^properties\./, '')
  .replace(/^transform\./, '');
const property: any = (layer: any, path: any) => {
  const channel: any = channelPath(path);
  const transitionMatch: any = /^(transitionIn|transitionOut)\.p\.([a-zA-Z][a-zA-Z0-9]*)$/.exec(channel);
  if (transitionMatch) {
    const transition: any = layer[transitionMatch[1]];
    return { channel, prop: transition?.p?.[transitionMatch[2]] || null };
  }
  return { channel, prop: PM.findProp(layer, channel) || layer.p[channel] || null };
};
const lockedIntent: any = (layer: any, channel: any) => {
  const intent: any = layer.locked_intent || {};
  const root: any = channel.split('.')[0];
  return intent[channel] || intent[root] || null;
};

function restore(json: any, selection: any) {
  PM.replaceProject(JSON.parse(json), selection ? { selection } : {});
}

function summarize(command: any) {
  const target: any = command.target || command.layer || command.targetId || '';
  const path: any = command.path || command.channel || '';
  return [command.type, target, path].filter(Boolean).join(' · ');
}

function rememberLive(command: any) {
  const copy: any = clone(command);
  const target: any = copy.target || copy.layer || copy.targetId || '';
  const mergeable: any = new Set(['set_property', 'set_layer', 'set_content', 'set_composition', 'set_scene_parameter', 'reorder_layer']);
  if (!mergeable.has(copy.type)) { live.commands.push(copy); return; }
  const key: any = copy.type === 'set_property'
    ? `${copy.type}:${target}:${copy.path || copy.channel}`
    : copy.type === 'set_scene_parameter'
      ? `${copy.type}:${copy.name}`
      : `${copy.type}:${target}`;
  const index: any = live.commands.findIndex((item: any) => item._liveKey === key);
  Object.defineProperty(copy, '_liveKey', { value: key, enumerable: false });
  if (index === -1) live.commands.push(copy);
  else live.commands[index] = copy;
}

function record(label: any, origin: any, applied: any) {
  PM.proj.revision = Math.max(0, Number(PM.proj.revision) || 0) + 1;
  PM.proj.edits = Array.isArray(PM.proj.edits) ? PM.proj.edits : [];
  PM.proj.edits.push({
    id: PM.uid('edit'),
    revision: PM.proj.revision,
    at: Date.now(),
    origin: origin || 'interface',
    label: label || 'Edit source',
    summary: applied.map(summarize),
    operations: clone(applied),
  });
  if (PM.proj.edits.length > MAX_EDITS) PM.proj.edits.splice(0, PM.proj.edits.length - MAX_EDITS);
}

function setProperty(command: any) {
  const layer: any = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  const { channel, prop }: any = property(layer, command.path || command.channel);
  if (!prop) throw new Error(`Property “${channel}” was not found on “${layer.name}”`);
  if (command.preserveHandEdits !== false && lockedIntent(layer, channel)) {
    throw new Error(`Preserved hand-edited ${channel}; explicitly allow overwrite to change it`);
  }
  const current: any = PM.evP(layer, prop, command.time == null ? PM.time : command.time, channel);
  let value: any = command.value;
  if (typeof current === 'number') value = finite(value, channel);
  const time: any = command.time == null ? PM.time : finite(command.time, 'time');
  const mode: any = command.mode || 'auto';
  if (mode === 'keyframe' || (mode === 'auto' && prop.kf.length)) {
    const key: any = PM.setKeyOn(prop, time - layer.from, value, command.ease || 'linear', PM.proj.fps);
    if (command.hold != null) key.hold = !!command.hold;
  } else {
    prop.v = value;
    PM.touch();
  }
  if (command.markIntent) {
    layer.locked_intent = layer.locked_intent || {};
    layer.locked_intent[channel] = {
      by: command.markIntent === true ? 'human' : String(command.markIntent),
      at: Date.now(),
      t: PM.round(time - layer.from, 3),
    };
  }
  return { id: layer.id, channel, value };
}

function replaceKeyframes(command: any) {
  const layer: any = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  const { channel, prop }: any = property(layer, command.path || command.channel);
  if (!prop) throw new Error(`Property “${channel}” was not found on “${layer.name}”`);
  if (command.preserveHandEdits !== false && lockedIntent(layer, channel)) {
    throw new Error(`Preserved hand-edited ${channel}; explicitly allow overwrite to change it`);
  }
  if (!Array.isArray(command.keyframes)) throw new Error('keyframes must be an array');
  const next: any = command.keyframes.map((key: any, index: any) => {
    if (!key || typeof key !== 'object') throw new Error(`Keyframe ${index + 1} is invalid`);
    const time: any = Math.max(0, finite(key.time, `keyframe ${index + 1} time`));
    const value: any = typeof prop.v === 'number' ? finite(key.value, `keyframe ${index + 1} value`) : key.value;
    return { time, value, ease: key.ease || 'linear', hold: !!key.hold };
  });
  if (command.replace !== false) prop.kf = [];
  for (const key of next) {
    const made: any = PM.setKeyOn(prop, key.time, key.value, key.ease, PM.proj.fps);
    made.hold = key.hold;
  }
  if (command.expression !== undefined) prop.expr = command.expression || null;
  PM.touch();
  return { id: layer.id, channel, keyframes: prop.kf.length };
}

function easingCurve(value: any) {
  const preset: any = typeof value === 'string' ? PM.Ease?.PRESETS?.[value] : null;
  const source: any = preset || value;
  if (!Array.isArray(source) || source.length !== 4) throw new Error('Easing curve must contain four handles');
  const curve: any = source.map((part: any, index: any) => finite(part, `easing handle ${index + 1}`));
  curve[0] = PM.clamp(curve[0], 0, 1); curve[2] = PM.clamp(curve[2], 0, 1);
  curve[1] = PM.clamp(curve[1], -4, 4); curve[3] = PM.clamp(curve[3], -4, 4);
  return curve;
}

function setEasing(command: any) {
  const ids: any = new Set((Array.isArray(command.keyframes) ? command.keyframes : [])
    .filter((id: any) => typeof id === 'string' && id).slice(0, 1000));
  if (!ids.size) throw new Error('Select at least one keyframe');
  const curve: any = easingCurve(command.curve);
  const found: any = [];
  for (const layer of PM.proj.layers) {
    for (const item of PM.allProps(layer)) {
      for (const key of item.prop.kf) if (ids.has(key.i)) found.push(key);
    }
  }
  if (!found.length) throw new Error('The selected keyframes are no longer available');
  found.forEach((key: any) => {
    key.eo = [curve[0], curve[1]];
    key.ei = [curve[2], curve[3]];
    key.hold = false;
  });
  PM.touch();
  return { keyframes: found.length, curve };
}

function setExpression(command: any) {
  const layer: any = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  const { channel, prop }: any = property(layer, command.path || command.channel);
  if (!prop) throw new Error(`Property “${channel}” was not found on “${layer.name}”`);
  prop.expr = command.expression || null;
  PM.touch();
  return { id: layer.id, channel, expression: prop.expr };
}

function setContent(command: any) {
  const layer: any = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  const patch: any = safePatch(command.patch, 'content patch');
  if (layer.type === 'audio') setAudioContent(layer, patch);
  else Object.assign(layer.d, patch);
  if (layer.type === 'shader' && Object.hasOwn(patch, 'code')) {
    PM.syncShaderUniforms && PM.syncShaderUniforms(layer);
    if (layer._shaderKey && PM.GL) PM.GL.dropProgram(layer._shaderKey);
  }
  PM.touch();
  return { id: layer.id, keys: Object.keys(patch) };
}

function setAudioContent(layer: any, patch: any) {
  for (const key of Object.keys(patch)) {
    if (!AUDIO_CONTENT_FIELDS.has(key)) throw new Error(`Audio content field “${key}” is not editable`);
  }
  const current: any = layer.d && typeof layer.d === 'object' ? layer.d : {};
  const next: any = { ...current, ...patch };
  if (next.asset != null && (typeof next.asset !== 'string' || !next.asset)) throw new Error('Audio source must be a media asset ID or null');
  const number: any = (key: any, fallback: any, min: any, max: any = Infinity) => {
    const value: any = next[key] == null ? fallback : finite(next[key], key);
    return PM.clamp(value, min, max);
  };
  layer.d = {
    asset: next.asset || null,
    trim: number('trim', 0, 0),
    gain: number('gain', 1, 0, 4),
    fadeIn: number('fadeIn', 0, 0),
    fadeOut: number('fadeOut', 0, 0),
  };
  layer.p = {};
  layer.fx = [];
  layer.masks = [];
  layer.parent = null;
  layer.blend = 'normal';
  layer.mblur = false;
}

function setLayer(command: any) {
  const layer: any = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  const patch: any = safePatch(command.patch, 'layer patch');
  for (const key of Object.keys(patch)) if (!LAYER_FIELDS.has(key)) throw new Error(`Layer field “${key}” is not editable`);
  if (layer.type === 'audio') {
    if (patch.parent !== undefined && patch.parent !== null) throw new Error('Audio layers do not support parenting');
    if (patch.blend !== undefined && patch.blend !== 'normal') throw new Error('Audio layers do not support blend modes');
    if (patch.motionBlur !== undefined && patch.motionBlur !== false) throw new Error('Audio layers do not support motion blur');
  }
  if (patch.name != null) layer.name = String(patch.name).trim() || layer.name;
  if (patch.from != null) layer.from = Math.max(0, finite(patch.from, 'layer start'));
  if (patch.duration != null) layer.dur = Math.max(1 / PM.proj.fps, finite(patch.duration, 'layer duration'));
  if (patch.visible != null) layer.on = !!patch.visible;
  if (patch.locked != null) layer.lock = !!patch.locked;
  if (patch.solo != null) layer.solo = !!patch.solo;
  if (patch.shy != null) layer.shy = !!patch.shy;
  if (patch.blend != null) {
    if (!PM.BLENDS.includes(patch.blend)) throw new Error(`Unknown blend mode: ${patch.blend}`);
    layer.blend = patch.blend;
  }
  if (patch.motionBlur != null) layer.mblur = !!patch.motionBlur;
  if (patch.parent !== undefined) {
    const parent: any = patch.parent == null ? null : findLayer(patch.parent);
    if (patch.parent != null && (!parent || parent.id === layer.id)) throw new Error('Invalid parent layer');
    if (parent && PM.wouldCycle(layer, parent.id)) throw new Error('Parenting would create a cycle');
    layer.parent = parent ? parent.id : null;
  }
  if (patch.color != null) layer.color = String(patch.color);
  if (patch.collapsed != null) layer.collapsed = !!patch.collapsed;
  if (patch.scaleLinked != null) layer.scaleLinked = !!patch.scaleLinked;
  PM.touch();
  return { id: layer.id, keys: Object.keys(patch) };
}

function setComposition(command: any) {
  const patch: any = safePatch(command.patch, 'composition patch');
  const allowed: any = new Set(['name', 'width', 'height', 'fps', 'duration', 'background', 'backgroundFill', 'shutter', 'workArea']);
  for (const key of Object.keys(patch)) if (!allowed.has(key)) throw new Error(`Composition field “${key}” is not editable`);
  const p: any = PM.proj;
  if (patch.name != null) p.name = String(patch.name).trim() || p.name;
  if (patch.width != null) p.w = Math.max(16, Math.round(finite(patch.width, 'width')));
  if (patch.height != null) p.h = Math.max(16, Math.round(finite(patch.height, 'height')));
  if (patch.fps != null) p.fps = PM.clamp(Math.round(finite(patch.fps, 'fps')), 1, 240);
  if (patch.duration != null) {
    p.dur = Math.max(.1, finite(patch.duration, 'duration'));
    if (!patch.workArea) p.work = [0, p.dur];
  }
  if (patch.background != null) {
    p.bg = String(patch.background);
    p.backgroundFill = PM.normalizeFill({ type: 'solid', color: p.bg }, p.bg);
  }
  if (patch.backgroundFill != null) {
    p.backgroundFill = PM.normalizeFill(patch.backgroundFill, p.bg);
    p.bg = p.backgroundFill.stops[0].color;
  }
  if (patch.shutter != null) p.shutter = PM.clamp(finite(patch.shutter, 'shutter'), 0, 2);
  if (patch.workArea != null) {
    if (!Array.isArray(patch.workArea) || patch.workArea.length !== 2) throw new Error('workArea must contain start and end');
    const start: any = Math.max(0, finite(patch.workArea[0], 'work area start'));
    const end: any = Math.min(p.dur, finite(patch.workArea[1], 'work area end'));
    if (end <= start) throw new Error('workArea end must be after its start');
    p.work = [start, end];
  }
  PM.touch();
  return { keys: Object.keys(patch) };
}

function addLayer(command: any) {
  const type: any = command.layerType || command.kind;
  if (!PM.TYPE_META[type]) throw new Error(`Unknown layer type: ${type}`);
  if (type === 'audio' && (command.parent != null || (command.blend != null && command.blend !== 'normal') || command.motionBlur === true)) {
    throw new Error('Audio layers do not support parenting, blend modes, or motion blur');
  }
  const opts: any = {
    name: command.name,
    from: command.from,
    dur: command.duration,
    d: safePatch(command.content || {}, 'content'),
    p: safePatch(command.properties || {}, 'properties'),
    color: command.color,
  };
  const layer: any = PM.mkLayer(type, opts);
  if (type === 'audio') setAudioContent(layer, opts.d);
  if (command.id != null) {
    if (PM.L(command.id)) throw new Error(`Layer id already exists: ${command.id}`);
    layer.id = String(command.id);
  }
  layer.from = command.from == null ? PM.snapF(PM.time, PM.proj.fps) : Math.max(0, finite(command.from, 'layer start'));
  layer.dur = command.duration == null
    ? Math.max(.1, PM.proj.dur - layer.from)
    : Math.max(1 / PM.proj.fps, finite(command.duration, 'layer duration'));
  PM.addLayer(layer, command.index == null ? 0 : PM.clamp(Math.round(command.index), 0, PM.proj.layers.length));
  if (command.parent != null) {
    const parent: any = findLayer(command.parent);
    if (!parent || parent.id === layer.id || PM.wouldCycle(layer, parent.id)) throw new Error('Invalid parent layer');
    layer.parent = parent.id;
  }
  if (command.blend != null) {
    if (!PM.BLENDS.includes(command.blend)) throw new Error(`Unknown blend mode: ${command.blend}`);
    layer.blend = command.blend;
  }
  if (command.motionBlur != null) layer.mblur = !!command.motionBlur;
  if (command.visible != null) layer.on = !!command.visible;
  if (command.solo != null) layer.solo = !!command.solo;
  if (command.shy != null) layer.shy = !!command.shy;
  if (command.collapsed != null) layer.collapsed = !!command.collapsed;
  if (type === 'shader' && PM.syncShaderUniforms) PM.syncShaderUniforms(layer);
  if (command.select !== false) PM.selectLayers(layer.id);
  return { id: layer.id, name: layer.name, layer };
}

function deleteLayers(command: any) {
  const refs: any = Array.isArray(command.targets) ? command.targets : [command.target || command.layer].filter(Boolean);
  const layers: any = refs.length ? refs.map(findLayer).filter(Boolean) : PM.selLayers();
  if (!layers.length) throw new Error('No layers to delete');
  PM.removeLayers(layers.map((layer: any) => layer.id));
  return { ids: layers.map((layer: any) => layer.id) };
}

function reorderLayer(command: any) {
  const layer: any = findLayer(command.target || command.layer);
  if (!layer) throw new Error('Layer not found');
  const from: any = PM.proj.layers.indexOf(layer);
  const to: any = PM.clamp(Math.round(finite(command.index, 'layer index')), 0, PM.proj.layers.length - 1);
  if (from !== to) {
    PM.proj.layers.splice(from, 1);
    PM.proj.layers.splice(to, 0, layer);
    PM.bus.emit('layers');
  }
  return { id: layer.id, index: to };
}

function addEffect(command: any) {
  const layer: any = findLayer(command.target || command.layer);
  if (!layer) throw new Error('Layer not found');
  if (PM.TYPE_META[layer.type] && PM.TYPE_META[layer.type].effects === false) throw new Error(`${PM.TYPE_META[layer.type].label} layers do not support visual effects`);
  const effect: any = PM.mkEffect(command.effect);
  if (!effect) throw new Error(`Unknown effect: ${command.effect}`);
  effect.open = command.open !== false;
  const values: any = safePatch(command.parameters || {}, 'effect parameters');
  for (const [key, value] of Object.entries(values)) if (effect.p[key]) effect.p[key].v = value;
  layer.fx.push(effect); PM.touch();
  return { id: layer.id, effectId: effect.id };
}

function removeEffect(command: any) {
  const layer: any = findLayer(command.target || command.layer);
  if (!layer) throw new Error('Layer not found');
  if (PM.TYPE_META[layer.type] && PM.TYPE_META[layer.type].effects === false) throw new Error(`${PM.TYPE_META[layer.type].label} layers do not support visual effects`);
  const before: any = layer.fx.length;
  layer.fx = layer.fx.filter((effect: any) => effect.id !== command.effect && effect.type !== command.effect);
  if (before === layer.fx.length) throw new Error(`Effect not found: ${command.effect}`);
  PM.touch();
  return { id: layer.id, removed: before - layer.fx.length };
}

function setEffect(command: any) {
  const layer: any = findLayer(command.target || command.layer);
  if (!layer) throw new Error('Layer not found');
  if (PM.TYPE_META[layer.type] && PM.TYPE_META[layer.type].effects === false) throw new Error(`${PM.TYPE_META[layer.type].label} layers do not support visual effects`);
  const effect: any = layer.fx.find((item: any) => item.id === command.effect || item.type === command.effect);
  if (!effect) throw new Error(`Effect not found: ${command.effect}`);
  const patch: any = safePatch(command.patch, 'effect patch');
  const allowed: any = new Set(['enabled', 'open']);
  for (const key of Object.keys(patch)) if (!allowed.has(key)) throw new Error(`Effect field “${key}” is not editable`);
  if (patch.enabled != null) effect.on = !!patch.enabled;
  if (patch.open != null) effect.open = !!patch.open;
  PM.touch();
  return { id: layer.id, effectId: effect.id };
}

function setTransition(command: any) {
  const layer: any = findLayer(command.layer || command.target || command.targetId);
  if (!layer) throw new Error('Layer not found');
  if (command.edge !== 'in' && command.edge !== 'out') throw new Error('Transition edge must be “in” or “out”');
  const field: any = command.edge === 'in' ? 'transitionIn' : 'transitionOut';
  if (command.transition == null) {
    layer[field] = null;
    PM.touch();
    return { id: layer.id, edge: command.edge, transition: null };
  }
  if (!command.transition || typeof command.transition !== 'object' || Array.isArray(command.transition)) {
    throw new Error('Transition must be an object or null');
  }
  const type: any = command.transition.type;
  if (typeof type !== 'string' || !type) throw new Error('Transition type is required');
  const definition: any = PM.transitionDef?.(type);
  if (!definition) throw new Error(`Unknown transition: ${type}`);
  const duration: any = command.transition.dur == null ? 0.5 : finite(command.transition.dur, 'transition duration');
  if (duration < 0.02 || duration > 600) throw new Error('Transition duration must be between 0.02 and 600 seconds');
  const values: any = command.transition.p == null ? {} : safePatch(command.transition.p, 'transition parameters');
  const params: any = new Map(definition.params.map((param: any) => [param.k, param]));
  for (const [key, value] of Object.entries(values)) {
    const param: any = params.get(key);
    if (!param) throw new Error(`Unknown transition parameter: ${key}`);
    if (param.type === 'color') {
      if (!colorValue(value)) throw new Error(`Transition parameter “${key}” must be a hex color`);
    } else if (param.type === 'toggle') {
      if (typeof value !== 'boolean') throw new Error(`Transition parameter “${key}” must be a boolean`);
    } else if (typeof value !== 'number' || !Number.isFinite(value)) {
      throw new Error(`Transition parameter “${key}” must be a finite number`);
    }
  }
  const transition: any = PM.mkTransition(type);
  if (!transition) throw new Error(`Unknown transition: ${type}`);
  transition.dur = duration;
  /* Same type: keep the existing channels so keyframes/expressions survive a
     duration or single-value edit (the inspector re-emits the whole command). */
  const existing: any = layer[field];
  if (existing && existing.type === type && !existing.missing && existing.p) {
    for (const key of Object.keys(transition.p)) if (existing.p[key]) transition.p[key] = existing.p[key];
  }
  for (const [key, value] of Object.entries(values)) transition.p[key].v = value;
  layer[field] = transition;
  PM.touch();
  return { id: layer.id, edge: command.edge, transition: type };
}

function setSceneParameter(command: any) {
  const name: any = String(command.name || '').trim();
  if (!name) throw new Error('Scene parameter name is required');
  const current: any = PM.proj.params[name] || { name, label: command.label || name, control: command.control || 'num' };
  /* Generated controls keep a reference to their parameter while they are
     mounted. Preserve that object identity so a source edit is visible in the
     control immediately, just like an inspector or canvas edit. */
  Object.assign(current, {
    ...current,
    label: command.label || current.label || name,
    control: command.control || current.control || 'num',
    value: clone(command.value),
    min: command.min == null ? current.min : command.min,
    max: command.max == null ? current.max : command.max,
    options: command.options == null ? (current.options || []) : clone(command.options),
  });
  PM.proj.params[name] = current;
  PM.touch();
  return { name, value: PM.proj.params[name].value };
}

function addMarker(command: any) {
  const time: any = PM.clamp(finite(command.time == null ? PM.time : command.time, 'marker time'), 0, PM.proj.dur);
  const marker: any = {
    id: command.id || PM.uid('marker'),
    t: time,
    name: String(command.name || `M${PM.proj.markers.length + 1}`),
  };
  PM.proj.markers.push(marker);
  PM.proj.markers.sort((a: any, b: any) => a.t - b.t);
  PM.touch();
  return marker;
}

function createSection(command: any) {
  if (!command.section || typeof command.section !== 'object') throw new Error('Section manifest is required');
  PM.proj.library = PM.proj.library && typeof PM.proj.library === 'object' ? PM.proj.library : { sections: [], looks: [] };
  PM.proj.library.sections = Array.isArray(PM.proj.library.sections) ? PM.proj.library.sections : [];
  if (PM.proj.library.sections.some((section: any) => section.id === command.section.id)) throw new Error('Section id already exists');
  PM.proj.library.sections.unshift(clone(command.section));
  while (PM.proj.library.sections.length > 24) PM.proj.library.sections.pop();
  PM.bus.emit('library');
  return { id: command.section.id };
}

function updateSection(command: any) {
  const sections: any = PM.proj.library && Array.isArray(PM.proj.library.sections) ? PM.proj.library.sections : [];
  const section: any = sections.find((item: any) => item.id === command.sectionId);
  if (!section) throw new Error('Section not found');
  if (!Array.isArray(command.layers) || !command.layers.length) throw new Error('Section layers are required');
  section.layers = clone(command.layers);
  section.thumb = command.thumb || section.thumb || null;
  section.at = Number(command.at) || Date.now();
  section.tags = [...new Set(section.layers.map((layer: any) => layer.type).filter(Boolean))];
  section.versions = Array.isArray(section.versions) ? section.versions : [];
  section.versions.push(clone(command.version || { id: PM.uid('SV'), layers: section.layers, thumb: section.thumb, at: section.at }));
  if (section.versions.length > 12) section.versions.splice(0, section.versions.length - 12);
  PM.bus.emit('library');
  return { id: section.id, versions: section.versions.length };
}

function keyframeLayers(command: any) {
  const ids: any = new Set((Array.isArray(command.keyframes) ? command.keyframes : [])
    .filter((id: any) => typeof id === 'string' && id));
  if (!ids.size) return [];
  return PM.proj.layers.filter((layer: any) => PM.allProps(layer)
    .some((item: any) => item.prop.kf.some((key: any) => ids.has(key.i))));
}

function lockedMessage(layer: any) {
  return `Layer “${layer.name}” is locked`;
}

function policyCommand(sourceCommand: any, meta: any = {}) {
  if (!sourceCommand || typeof sourceCommand !== 'object' || Array.isArray(sourceCommand)) {
    throw new Error('Edit command must be an object');
  }
  const command: any = clone(sourceCommand);
  const origin: any = meta.origin; // raw — unset/unknown origins are untrusted
  if (!isHumanOrigin(origin)) delete command.overrideLock;
  /* A generated panel control can carry an explicit human gesture (its command
     is constructed by trusted binding code, not by the manifest — the manifest
     button path strips these fields in workspace.js). That gesture may
     overwrite hand-intent, but it still cannot bypass the layer lock. */
  if (isGeneratedOrigin(origin) && !(origin === 'generated-ui' && command.markIntent === 'human')) {
    command.preserveHandEdits = true;
  }

  if (command.type === 'delete_layers') {
    const refs: any = Array.isArray(command.targets) ? command.targets : [command.target || command.layer].filter(Boolean);
    const layers: any = refs.length ? refs.map(findLayer).filter(Boolean) : PM.selLayers();
    if (!layers.length) return { command };
    if (command.overrideLock === true) return { command };
    const locked: any = layers.filter((layer: any) => layer.lock);
    if (!locked.length) return { command };
    const editable: any = layers.filter((layer: any) => !layer.lock);
    const names: any = locked.map((layer: any) => `“${layer.name}”`).join(', ');
    if (!editable.length) throw new Error(`All targeted layers are locked: ${names}`);
    command.targets = editable.map((layer: any) => layer.id);
    delete command.target;
    delete command.layer;
    return {
      command,
      message: `Skipped locked layers: ${names}`,
      skippedLocked: locked.map((layer: any) => layer.id),
    };
  }

  if (command.type === 'set_easing') {
    if (command.overrideLock !== true) {
      const locked: any = keyframeLayers(command).find((layer: any) => layer.lock);
      if (locked) throw new Error(lockedMessage(locked));
    }
    return { command };
  }

  if (lockedLayerOps().has(command.type)) {
    const layer: any = findLayer(command.target || command.layer || command.targetId);
    /* The bare {locked:false} unlock is a HUMAN affordance. Agent/generated/
       unknown origins must not unlock — otherwise a two-command batch
       [unlock, edit] defeats the whole matrix. */
    const unlockOnly: any = isHumanOrigin(origin)
      && command.type === 'set_layer'
      && command.patch && typeof command.patch === 'object' && !Array.isArray(command.patch)
      && Object.keys(command.patch).length === 1 && command.patch.locked === false;
    if (layer?.lock && command.overrideLock !== true && !unlockOnly) throw new Error(lockedMessage(layer));
  }
  return { command };
}

function runOne(sourceCommand: any, meta: any = {}) {
  const policy: any = policyCommand(sourceCommand, meta);
  const command: any = policy.command;
  let data: any;
  switch (command.type) {
    case 'set_property': data = setProperty(command); break;
    case 'replace_keyframes': data = replaceKeyframes(command); break;
    case 'set_easing': data = setEasing(command); break;
    case 'set_expression': data = setExpression(command); break;
    case 'set_content': data = setContent(command); break;
    case 'set_layer': data = setLayer(command); break;
    case 'set_composition': data = setComposition(command); break;
    case 'add_layer': data = addLayer(command); break;
    case 'delete_layers': data = deleteLayers(command); break;
    case 'reorder_layer': data = reorderLayer(command); break;
    case 'add_effect': data = addEffect(command); break;
    case 'remove_effect': data = removeEffect(command); break;
    case 'set_effect': data = setEffect(command); break;
    case 'set_transition': data = setTransition(command); break;
    case 'set_scene_parameter': data = setSceneParameter(command); break;
    case 'add_marker': data = addMarker(command); break;
    case 'create_section': data = createSection(command); break;
    case 'update_section': data = updateSection(command); break;
    default: throw new Error(`Unknown source edit: ${command.type}`);
  }
  if (policy.message) data = { ...data, message: policy.message, skippedLocked: policy.skippedLocked };
  return { command, data, message: policy.message };
}

function recordedCommand(command: any, submitted: any) {
  const recorded: any = clone(command);
  /* Preserve the caller's overwrite request in provenance while the command
     dispatched above uses the stricter origin-derived policy. */
  if (submitted?.preserveHandEdits === false) recorded.preserveHandEdits = false;
  return recorded;
}

function changed(notify: any = true) {
  PM.touch();
  if (notify) {
    PM.bus.emit('layers');
    PM.bus.emit('project');
  }
  PM.invalidate();
  if (notify && PM.Inspector && PM.Inspector.refresh) PM.Inspector.refresh();
}

const colorValue: any = (value: any) => typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value);
const primitiveControl: any = (value: any) => typeof value === 'boolean' ? 'toggle'
  : typeof value === 'number' ? 'slider'
    : colorValue(value) ? 'color'
      : typeof value === 'string' ? 'text' : null;
const numericRange: any = (value: any, hint: any = '') => {
  const n: any = Number(value) || 0;
  if (hint === 'opacity') return { min: 0, max: 100, step: 1 };
  if (hint.includes('rotation') || hint === 'angle') return { min: -360, max: 360, step: 1 };
  if (hint.includes('scale')) return { min: 0, max: 1000, step: .5 };
  if (hint.includes('position') || hint.includes('anchor')) return { min: -Math.max(PM.proj.w, PM.proj.h) * 2, max: Math.max(PM.proj.w, PM.proj.h) * 2, step: 1 };
  const span: any = Math.max(1, Math.abs(n) * 2);
  return { min: n < 0 ? -span : 0, max: span, step: span >= 100 ? 1 : span / 100 };
};

/* This catalog is the single capability map used by the agent and generated
   panels. It is derived from the live source, so adding a real editable field
   automatically makes it available instead of requiring another prompt-only
   whitelist. */
function sourceCatalog() {
  const p: any = PM.proj;
  const fill: any = PM.normalizeFill(p.backgroundFill, p.bg);
  const composition: any = [
    { path: 'composition.name', label: 'Name', control: 'text', value: p.name },
    { path: 'composition.width', label: 'Width', control: 'slider', value: p.w, min: 16, max: 16384, step: 1, unit: 'px' },
    { path: 'composition.height', label: 'Height', control: 'slider', value: p.h, min: 16, max: 16384, step: 1, unit: 'px' },
    { path: 'composition.fps', label: 'Frame rate', control: 'slider', value: p.fps, min: 1, max: 240, step: 1, unit: 'fps' },
    { path: 'composition.duration', label: 'Duration', control: 'slider', value: p.dur, min: .1, max: Math.max(60, p.dur * 4), step: 1 / Math.max(1, p.fps), unit: 's' },
    { path: 'composition.shutter', label: 'Shutter', control: 'slider', value: p.shutter ?? .5, min: 0, max: 2, step: .01 },
    { path: 'composition.workArea.start', label: 'Work area start', control: 'slider', value: p.work?.[0] ?? 0, min: 0, max: p.dur, step: 1 / Math.max(1, p.fps), unit: 's' },
    { path: 'composition.workArea.end', label: 'Work area end', control: 'slider', value: p.work?.[1] ?? p.dur, min: 0, max: p.dur, step: 1 / Math.max(1, p.fps), unit: 's' },
    { path: 'composition.backgroundFill', label: 'Background fill', control: 'fill', value: fill },
    { path: 'composition.background', label: 'Background color', control: 'color', value: p.bg },
    { path: 'composition.background.type', label: 'Background type', control: 'select', value: fill.type, options: ['solid', 'linear', 'radial', 'none'] },
    { path: 'composition.background.startColor', label: 'Background start', control: 'color', value: fill.stops[0]?.color || p.bg },
    { path: 'composition.background.endColor', label: 'Background end', control: 'color', value: fill.stops[1]?.color || fill.stops[0]?.color || p.bg },
    { path: 'composition.background.angle', label: 'Background angle', control: 'slider', value: fill.angle, min: -180, max: 180, step: 1, unit: '°' },
    { path: 'composition.background.midpoint', label: 'Background midpoint', control: 'slider', value: fill.stops[1]?.position ?? 100, min: 0, max: 100, step: 1, unit: '%' },
  ];
  const parentOptions: any = [{ v: null, label: 'None' }, ...p.layers.map((layer: any) => ({ v: layer.id, label: layer.name }))];
  const layers: any = p.layers.map((layer: any) => {
    const fields: any = [
      { path: 'layer.name', label: 'Layer name', control: 'text', value: layer.name },
      { path: 'layer.from', label: 'Start time', control: 'slider', value: layer.from, min: 0, max: p.dur, step: 1 / Math.max(1, p.fps), unit: 's' },
      { path: 'layer.duration', label: 'Duration', control: 'slider', value: layer.dur, min: 1 / Math.max(1, p.fps), max: Math.max(p.dur, layer.dur), step: 1 / Math.max(1, p.fps), unit: 's' },
      { path: 'layer.visible', label: 'Visible', control: 'toggle', value: layer.on },
      { path: 'layer.locked', label: 'Locked', control: 'toggle', value: layer.lock },
      { path: 'layer.solo', label: 'Solo', control: 'toggle', value: layer.solo },
      { path: 'layer.shy', label: 'Shy', control: 'toggle', value: layer.shy },
      { path: 'layer.collapsed', label: 'Collapsed', control: 'toggle', value: layer.collapsed },
      { path: 'layer.color', label: 'Label color', control: 'color', value: layer.color },
    ];
    if (layer.type !== 'audio') fields.splice(fields.length - 1, 0,
      { path: 'layer.motionBlur', label: 'Motion blur', control: 'toggle', value: layer.mblur },
      { path: 'layer.blend', label: 'Blend mode', control: 'select', value: layer.blend, options: [...PM.BLENDS] },
      { path: 'layer.parent', label: 'Parent', control: 'select', value: layer.parent, options: parentOptions.filter((option: any) => option.v !== layer.id) });
    const content: any = Object.entries(layer.d || {}).map(([key, value]: any) => {
      const control: any = primitiveControl(value); if (!control) return null;
      return { path: `content.${key}`, label: key, control, value, ...(control === 'slider' ? numericRange(value, key) : {}) };
    }).filter(Boolean);
    const properties: any = PM.allProps(layer).map((item: any) => {
      const value: any = PM.evP(layer, item.prop, PM.time, item.key);
      const control: any = primitiveControl(value); if (!control) return null;
      const fxId: any = item.key.split('.')[0];
      const fx: any = (layer.fx || []).find((effect: any) => effect.id === fxId);
      const fxParam: any = fx && PM.FX?.[fx.type]?.params?.find((param: any) => param.k === item.key.slice(fxId.length + 1));
      const shaderDef: any = layer.type === 'shader' && item.key.startsWith('u.')
        ? (layer._udefs || PM.parseUniforms?.(layer.d.code) || []).find((def: any) => `u.${def.name}` === item.key) : null;
      const meta: any = fxParam || shaderDef || PM.CH?.[item.key] || {};
      return {
        path: `properties.${item.key}`, label: meta.label || item.label || item.key,
        group: item.group, control: meta.type === 'color' ? 'color' : control, value,
        animatable: true, ...((meta.type === 'color' ? 'color' : control) === 'slider'
          ? { ...numericRange(value, item.key), ...Object.fromEntries(['min', 'max', 'step', 'unit'].filter((key: any) => meta[key] !== undefined).map((key: any) => [key, meta[key]])) }
          : {}),
      };
    }).filter(Boolean);
    for (const [field, label] of [['transitionIn', 'Transition in'], ['transitionOut', 'Transition out']] as any) {
      const transition: any = layer[field];
      const definition: any = transition && PM.transitionDef?.(transition.type);
      if (!definition || transition.missing) continue;
      for (const param of definition.params) {
        const prop: any = transition.p?.[param.k];
        if (!prop) continue;
        const value: any = PM.evP(layer, prop, PM.time, `${field}.p.${param.k}`);
        const control: any = param.type === 'color' ? 'color' : param.type === 'toggle' ? 'toggle' : 'slider';
        properties.push({
          path: `${field}.p.${param.k}`, label: param.label || param.k, group: label,
          control, value, animatable: true,
          ...(control === 'slider' ? { ...numericRange(value, param.k), ...Object.fromEntries(['min', 'max', 'step', 'unit'].filter((key: any) => param[key] !== undefined).map((key: any) => [key, param[key]])) } : {}),
        });
      }
    }
    return { id: layer.id, name: layer.name, type: layer.type, controls: [...fields, ...content, ...properties] };
  });
  return { target: '$composition', composition, layers, operations: Object.keys(Edit.operations) };
}

/* Higher-order generated tools compile to the same primitive edit language
   before a transaction starts. History therefore records the real layer/
   property changes, while one tool activation remains one atomic undo step. */
function expandCommands(input: any) {
  const expanded: any = [];
  for (const command of commands(input).filter(Boolean)) {
    if (command.type !== 'transform_layers') { expanded.push(command); continue; }
    if (!PM.Capabilities?.compile) throw new Error('Layer transformation capabilities are unavailable');
    const result: any = PM.Capabilities.compile(command.transform, command.state || {});
    if (!result.ok) throw new Error(result.message);
    expanded.push(...result.commands);
  }
  return expanded;
}

const Edit: any = {
  operations: Object.freeze({
    set_property: { target: 'layer', fields: ['path', 'value', 'time', 'mode', 'ease'] },
    replace_keyframes: { target: 'layer', fields: ['path', 'keyframes', 'replace', 'expression'] },
    set_easing: { target: 'keyframes', fields: ['keyframes', 'curve'] },
    set_expression: { target: 'layer', fields: ['path', 'expression'] },
    set_content: { target: 'layer', fields: ['patch'] },
    set_layer: { target: 'layer', fields: ['patch'] },
    set_composition: { target: 'project', fields: ['patch'] },
    add_layer: { target: 'project', fields: ['layerType', 'name', 'content', 'properties', 'parent', 'blend', 'motionBlur', 'visible', 'solo', 'shy', 'collapsed'] },
    delete_layers: { target: 'project', fields: ['targets'] },
    reorder_layer: { target: 'layer', fields: ['index'] },
    add_effect: { target: 'layer', fields: ['effect', 'parameters'] },
    remove_effect: { target: 'layer', fields: ['effect'] },
    set_effect: { target: 'layer', fields: ['effect', 'patch'] },
    set_transition: { target: 'layer', fields: ['layer', 'edge', 'transition'] },
    set_scene_parameter: { target: 'project', fields: ['name', 'value'] },
    add_marker: { target: 'project', fields: ['time', 'name'] },
    create_section: { target: 'project', fields: ['section'] },
    update_section: { target: 'project', fields: ['sectionId', 'layers', 'thumb', 'version', 'at'] },
    transform_layers: { target: 'layer-collection', fields: ['transform', 'state'] },
  }),

  apply(input: any, meta: any = {}) {
    let list: any;
    try { list = expandCommands(input); }
    catch (error: any) { return fail(String(error.message || error)); }
    if (!list.length) return fail('No source edits supplied');
    if (meta.baseRevision != null && Number(meta.baseRevision) !== Number(PM.proj.revision || 0)) {
      return fail(`Project changed: expected revision ${meta.baseRevision}, found ${PM.proj.revision || 0}`);
    }
    if (live) {
      try {
        const executed: any = list.map((command: any) => runOne(command, meta));
        const results: any = executed.map(({ command, data }: any) => ({ command: clone(command), data }));
        executed.forEach(({ command }: any) => rememberLive(command));
        changed(false);
        const notices: any = executed.map((item: any) => item.message).filter(Boolean);
        return pass(['Source updated', ...notices].join('. '), { results, revision: PM.proj.revision || 0 });
      } catch (error: any) { return fail(String(error.message || error)); }
    }
    const before: any = JSON.stringify(PM.proj);
    const selection: any = clone(PM.sel);
    PM.hist.begin(meta.label || 'Edit source', meta.historyGroup || null);
    try {
      const executed: any = list.map((command: any) => runOne(command, meta));
      const applied: any = executed.map((item: any, index: any) => recordedCommand(item.command, list[index]));
      const results: any = executed.map(({ command, data }: any) => ({ command: clone(command), data }));
      record(meta.label, meta.origin, applied);
      changed();
      PM.hist.commit(meta.label || 'Edit source');
      const notices: any = executed.map((item: any) => item.message).filter(Boolean);
      return pass([meta.label || 'Source updated', ...notices].join('. '), { results, revision: PM.proj.revision });
    } catch (error: any) {
      PM.hist.cancel();
      restore(before, selection);
      return fail(String(error.message || error));
    }
  },

  begin(label: any, meta: any = {}) {
    if (live) throw new Error('A source edit is already active');
    live = { label: label || 'Edit source', origin: meta.origin || 'interface', before: JSON.stringify(PM.proj), selection: clone(PM.sel), commands: [] };
    PM.hist.begin(live.label);
  },

  dispatch(command: any) {
    if (!live) return Edit.apply(command);
    return Edit.apply(command, { origin: live.origin, label: live.label });
  },

  commit(label: any) {
    if (!live) return fail('No source edit is active');
    const transaction: any = live;
    live = null;
    if (!transaction.commands.length || JSON.stringify(PM.proj) === transaction.before) {
      PM.hist.cancel();
      return pass('No source changes');
    }
    record(label || transaction.label, transaction.origin, transaction.commands);
    changed();
    PM.hist.commit(label || transaction.label);
    return pass(label || transaction.label, { revision: PM.proj.revision });
  },

  cancel() {
    if (!live) return false;
    const transaction: any = live;
    live = null;
    PM.hist.cancel();
    /* A click on a scrubbable field begins and immediately cancels an empty
       gesture before opening its text input. Do not replace the project in
       that common case or control bindings would retain stale object refs. */
    if (transaction.commands.length || JSON.stringify(PM.proj) !== transaction.before) {
      restore(transaction.before, transaction.selection);
    }
    return true;
  },

  describeLayer(ref: any) {
    const layer: any = findLayer(ref);
    if (!layer) return null;
    return {
      id: layer.id,
      name: layer.name,
      type: layer.type,
      content: Object.entries(layer.d).filter(([, value]: any) => ['string', 'number', 'boolean'].includes(typeof value)).map(([key, value]: any) => ({
        path: `content.${key}`, value, command: { type: 'set_content', target: layer.id, patch: { [key]: value } },
      })),
      properties: PM.allProps(layer).map((item: any) => ({
        path: `properties.${item.key}`, label: item.label, group: item.group,
        value: PM.evP(layer, item.prop, PM.time, item.key), animatable: true,
        command: { type: 'set_property', target: layer.id, path: item.key, value: PM.evP(layer, item.prop, PM.time, item.key) },
      })).concat((['transitionIn', 'transitionOut'] as any).flatMap((field: any) => {
        const transition: any = layer[field];
        const definition: any = transition && PM.transitionDef?.(transition.type);
        if (!definition || transition.missing) return [];
        return definition.params.filter((param: any) => transition.p?.[param.k]).map((param: any) => ({
          path: `${field}.p.${param.k}`, label: param.label || param.k,
          group: field === 'transitionIn' ? 'Transition in' : 'Transition out',
          value: PM.evP(layer, transition.p[param.k], PM.time, `${field}.p.${param.k}`), animatable: true,
          command: {
            type: 'set_property', target: layer.id, path: `${field}.p.${param.k}`,
            value: PM.evP(layer, transition.p[param.k], PM.time, `${field}.p.${param.k}`),
          },
        }));
      })),
      operations: Object.keys(Edit.operations),
    };
  },

  sourceCatalog,
};

PM.Edit = Edit;
}
