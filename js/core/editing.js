/* Powermove — one source-edit language shared by UI, agents, and generated controls. */
(() => {
const PM = window.PM;

const MAX_EDITS = 200;
const FORBIDDEN_KEYS = new Set(['__proto__', 'prototype', 'constructor']);
const LAYER_FIELDS = new Set([
  'name', 'from', 'duration', 'visible', 'locked', 'solo', 'shy', 'blend',
  'motionBlur', 'parent', 'color', 'collapsed',
]);
let live = null;

const clone = (value) => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
const fail = (message) => ({ ok: false, message });
const pass = (message, data = {}) => ({ ok: true, message, data });
const commands = (value) => Array.isArray(value) ? value : [value];
const finite = (value, label) => {
  const number = Number(value);
  if (!Number.isFinite(number)) throw new Error(`${label} must be a finite number`);
  return number;
};
const safePatch = (value, label) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} must be an object`);
  const out = {};
  for (const [key, child] of Object.entries(value)) {
    if (FORBIDDEN_KEYS.has(key)) throw new Error(`Unsafe ${label} key: ${key}`);
    out[key] = clone(child);
  }
  return out;
};
const findLayer = (ref) => {
  if (!ref) return PM.firstSel();
  if (ref === '$selection' || ref === 'selection') return PM.firstSel();
  if (typeof ref === 'number') return PM.proj.layers[ref - 1] || null;
  return PM.L(ref) || PM.byName(ref);
};
const channelPath = (path) => String(path || '')
  .replace(/^properties\./, '')
  .replace(/^transform\./, '');
const property = (layer, path) => {
  const channel = channelPath(path);
  return { channel, prop: PM.findProp(layer, channel) || layer.p[channel] || null };
};
const lockedIntent = (layer, channel) => {
  const intent = layer.locked_intent || {};
  const root = channel.split('.')[0];
  return intent[channel] || intent[root] || null;
};

function restore(json, selection) {
  PM.proj = JSON.parse(json);
  if (selection) {
    PM.sel.layers = (selection.layers || []).filter(id => PM.L(id));
    PM.sel.keys = selection.keys || [];
    PM.sel.chan = selection.chan || null;
  } else PM.sel.layers = PM.sel.layers.filter(id => PM.L(id));
  PM.touch();
  PM.bus.emit('layers'); PM.bus.emit('sel'); PM.bus.emit('project');
  PM.invalidate();
}

function summarize(command) {
  const target = command.target || command.layer || command.targetId || '';
  const path = command.path || command.channel || '';
  return [command.type, target, path].filter(Boolean).join(' · ');
}

function rememberLive(command) {
  const copy = clone(command);
  const target = copy.target || copy.layer || copy.targetId || '';
  const mergeable = new Set(['set_property', 'set_layer', 'set_content', 'set_composition', 'set_scene_parameter', 'reorder_layer']);
  if (!mergeable.has(copy.type)) { live.commands.push(copy); return; }
  const key = copy.type === 'set_property'
    ? `${copy.type}:${target}:${copy.path || copy.channel}`
    : copy.type === 'set_scene_parameter'
      ? `${copy.type}:${copy.name}`
      : `${copy.type}:${target}`;
  const index = live.commands.findIndex(item => item._liveKey === key);
  Object.defineProperty(copy, '_liveKey', { value: key, enumerable: false });
  if (index === -1) live.commands.push(copy);
  else live.commands[index] = copy;
}

function record(label, origin, applied) {
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

function setProperty(command) {
  const layer = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  if (layer.lock && command.overrideLock !== true) throw new Error(`Layer “${layer.name}” is locked`);
  const { channel, prop } = property(layer, command.path || command.channel);
  if (!prop) throw new Error(`Property “${channel}” was not found on “${layer.name}”`);
  if (command.preserveHandEdits !== false && lockedIntent(layer, channel)) {
    throw new Error(`Preserved hand-edited ${channel}; explicitly allow overwrite to change it`);
  }
  const current = PM.evP(layer, prop, command.time == null ? PM.time : command.time, channel);
  let value = command.value;
  if (typeof current === 'number') value = finite(value, channel);
  const time = command.time == null ? PM.time : finite(command.time, 'time');
  const mode = command.mode || 'auto';
  if (mode === 'keyframe' || (mode === 'auto' && prop.kf.length)) {
    const key = PM.setKeyOn(prop, time - layer.from, value, command.ease || 'power', PM.proj.fps);
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

function replaceKeyframes(command) {
  const layer = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  if (layer.lock && command.overrideLock !== true) throw new Error(`Layer “${layer.name}” is locked`);
  const { channel, prop } = property(layer, command.path || command.channel);
  if (!prop) throw new Error(`Property “${channel}” was not found on “${layer.name}”`);
  if (command.preserveHandEdits !== false && lockedIntent(layer, channel)) {
    throw new Error(`Preserved hand-edited ${channel}; explicitly allow overwrite to change it`);
  }
  if (!Array.isArray(command.keyframes)) throw new Error('keyframes must be an array');
  const next = command.keyframes.map((key, index) => {
    if (!key || typeof key !== 'object') throw new Error(`Keyframe ${index + 1} is invalid`);
    const time = Math.max(0, finite(key.time, `keyframe ${index + 1} time`));
    const value = typeof prop.v === 'number' ? finite(key.value, `keyframe ${index + 1} value`) : key.value;
    return { time, value, ease: key.ease || 'power', hold: !!key.hold };
  });
  if (command.replace !== false) prop.kf = [];
  for (const key of next) {
    const made = PM.setKeyOn(prop, key.time, key.value, key.ease, PM.proj.fps);
    made.hold = key.hold;
  }
  if (command.expression !== undefined) prop.expr = command.expression || null;
  PM.touch();
  return { id: layer.id, channel, keyframes: prop.kf.length };
}

function setExpression(command) {
  const layer = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  const { channel, prop } = property(layer, command.path || command.channel);
  if (!prop) throw new Error(`Property “${channel}” was not found on “${layer.name}”`);
  prop.expr = command.expression || null;
  PM.touch();
  return { id: layer.id, channel, expression: prop.expr };
}

function setContent(command) {
  const layer = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  if (layer.lock && command.overrideLock !== true) throw new Error(`Layer “${layer.name}” is locked`);
  const patch = safePatch(command.patch, 'content patch');
  Object.assign(layer.d, patch);
  if (layer.type === 'shader' && Object.hasOwn(patch, 'code')) {
    PM.syncShaderUniforms && PM.syncShaderUniforms(layer);
    if (layer._shaderKey && PM.GL) PM.GL.dropProgram(layer._shaderKey);
  }
  PM.touch();
  return { id: layer.id, keys: Object.keys(patch) };
}

function setLayer(command) {
  const layer = findLayer(command.target || command.layer || command.targetId);
  if (!layer) throw new Error('Layer not found');
  const patch = safePatch(command.patch, 'layer patch');
  for (const key of Object.keys(patch)) if (!LAYER_FIELDS.has(key)) throw new Error(`Layer field “${key}” is not editable`);
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
    const parent = patch.parent == null ? null : findLayer(patch.parent);
    if (patch.parent != null && (!parent || parent.id === layer.id)) throw new Error('Invalid parent layer');
    if (parent && PM.wouldCycle(layer, parent.id)) throw new Error('Parenting would create a cycle');
    layer.parent = parent ? parent.id : null;
  }
  if (patch.color != null) layer.color = String(patch.color);
  if (patch.collapsed != null) layer.collapsed = !!patch.collapsed;
  PM.touch();
  return { id: layer.id, keys: Object.keys(patch) };
}

function setComposition(command) {
  const patch = safePatch(command.patch, 'composition patch');
  const allowed = new Set(['name', 'width', 'height', 'fps', 'duration', 'background', 'backgroundFill', 'shutter', 'workArea']);
  for (const key of Object.keys(patch)) if (!allowed.has(key)) throw new Error(`Composition field “${key}” is not editable`);
  const p = PM.proj;
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
    const start = Math.max(0, finite(patch.workArea[0], 'work area start'));
    const end = Math.min(p.dur, finite(patch.workArea[1], 'work area end'));
    if (end <= start) throw new Error('workArea end must be after its start');
    p.work = [start, end];
  }
  PM.touch();
  return { keys: Object.keys(patch) };
}

function addLayer(command) {
  const type = command.layerType || command.kind;
  if (!PM.TYPE_META[type]) throw new Error(`Unknown layer type: ${type}`);
  const opts = {
    name: command.name,
    from: command.from,
    dur: command.duration,
    d: safePatch(command.content || {}, 'content'),
    p: safePatch(command.properties || {}, 'properties'),
    color: command.color,
  };
  const layer = PM.mkLayer(type, opts);
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
    const parent = findLayer(command.parent);
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

function deleteLayers(command) {
  const refs = Array.isArray(command.targets) ? command.targets : [command.target || command.layer].filter(Boolean);
  const layers = refs.length ? refs.map(findLayer).filter(Boolean) : PM.selLayers();
  if (!layers.length) throw new Error('No layers to delete');
  PM.removeLayers(layers.map(layer => layer.id));
  return { ids: layers.map(layer => layer.id) };
}

function reorderLayer(command) {
  const layer = findLayer(command.target || command.layer);
  if (!layer) throw new Error('Layer not found');
  const from = PM.proj.layers.indexOf(layer);
  const to = PM.clamp(Math.round(finite(command.index, 'layer index')), 0, PM.proj.layers.length - 1);
  if (from !== to) {
    PM.proj.layers.splice(from, 1);
    PM.proj.layers.splice(to, 0, layer);
    PM.bus.emit('layers');
  }
  return { id: layer.id, index: to };
}

function addEffect(command) {
  const layer = findLayer(command.target || command.layer);
  if (!layer) throw new Error('Layer not found');
  const effect = PM.mkEffect(command.effect);
  if (!effect) throw new Error(`Unknown effect: ${command.effect}`);
  effect.open = command.open !== false;
  const values = safePatch(command.parameters || {}, 'effect parameters');
  for (const [key, value] of Object.entries(values)) if (effect.p[key]) effect.p[key].v = value;
  layer.fx.push(effect); PM.touch();
  return { id: layer.id, effectId: effect.id };
}

function removeEffect(command) {
  const layer = findLayer(command.target || command.layer);
  if (!layer) throw new Error('Layer not found');
  const before = layer.fx.length;
  layer.fx = layer.fx.filter(effect => effect.id !== command.effect && effect.type !== command.effect);
  if (before === layer.fx.length) throw new Error(`Effect not found: ${command.effect}`);
  PM.touch();
  return { id: layer.id, removed: before - layer.fx.length };
}

function setEffect(command) {
  const layer = findLayer(command.target || command.layer);
  if (!layer) throw new Error('Layer not found');
  const effect = layer.fx.find(item => item.id === command.effect || item.type === command.effect);
  if (!effect) throw new Error(`Effect not found: ${command.effect}`);
  const patch = safePatch(command.patch, 'effect patch');
  const allowed = new Set(['enabled', 'open']);
  for (const key of Object.keys(patch)) if (!allowed.has(key)) throw new Error(`Effect field “${key}” is not editable`);
  if (patch.enabled != null) effect.on = !!patch.enabled;
  if (patch.open != null) effect.open = !!patch.open;
  PM.touch();
  return { id: layer.id, effectId: effect.id };
}

function setSceneParameter(command) {
  const name = String(command.name || '').trim();
  if (!name) throw new Error('Scene parameter name is required');
  const current = PM.proj.params[name] || { name, label: command.label || name, control: command.control || 'num' };
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

function addMarker(command) {
  const time = PM.clamp(finite(command.time == null ? PM.time : command.time, 'marker time'), 0, PM.proj.dur);
  const marker = {
    id: command.id || PM.uid('marker'),
    t: time,
    name: String(command.name || `M${PM.proj.markers.length + 1}`),
  };
  PM.proj.markers.push(marker);
  PM.proj.markers.sort((a, b) => a.t - b.t);
  PM.touch();
  return marker;
}

function createSection(command) {
  if (!command.section || typeof command.section !== 'object') throw new Error('Section manifest is required');
  PM.proj.library = PM.proj.library && typeof PM.proj.library === 'object' ? PM.proj.library : { sections: [], looks: [] };
  PM.proj.library.sections = Array.isArray(PM.proj.library.sections) ? PM.proj.library.sections : [];
  if (PM.proj.library.sections.some(section => section.id === command.section.id)) throw new Error('Section id already exists');
  PM.proj.library.sections.unshift(clone(command.section));
  while (PM.proj.library.sections.length > 24) PM.proj.library.sections.pop();
  PM.bus.emit('library');
  return { id: command.section.id };
}

function updateSection(command) {
  const sections = PM.proj.library && Array.isArray(PM.proj.library.sections) ? PM.proj.library.sections : [];
  const section = sections.find(item => item.id === command.sectionId);
  if (!section) throw new Error('Section not found');
  if (!Array.isArray(command.layers) || !command.layers.length) throw new Error('Section layers are required');
  section.layers = clone(command.layers);
  section.thumb = command.thumb || section.thumb || null;
  section.at = Number(command.at) || Date.now();
  section.tags = [...new Set(section.layers.map(layer => layer.type).filter(Boolean))];
  section.versions = Array.isArray(section.versions) ? section.versions : [];
  section.versions.push(clone(command.version || { id: PM.uid('SV'), layers: section.layers, thumb: section.thumb, at: section.at }));
  if (section.versions.length > 12) section.versions.splice(0, section.versions.length - 12);
  PM.bus.emit('library');
  return { id: section.id, versions: section.versions.length };
}

function runOne(command) {
  if (!command || typeof command !== 'object' || Array.isArray(command)) throw new Error('Edit command must be an object');
  switch (command.type) {
    case 'set_property': return setProperty(command);
    case 'replace_keyframes': return replaceKeyframes(command);
    case 'set_expression': return setExpression(command);
    case 'set_content': return setContent(command);
    case 'set_layer': return setLayer(command);
    case 'set_composition': return setComposition(command);
    case 'add_layer': return addLayer(command);
    case 'delete_layers': return deleteLayers(command);
    case 'reorder_layer': return reorderLayer(command);
    case 'add_effect': return addEffect(command);
    case 'remove_effect': return removeEffect(command);
    case 'set_effect': return setEffect(command);
    case 'set_scene_parameter': return setSceneParameter(command);
    case 'add_marker': return addMarker(command);
    case 'create_section': return createSection(command);
    case 'update_section': return updateSection(command);
    default: throw new Error(`Unknown source edit: ${command.type}`);
  }
}

function changed(notify = true) {
  PM.touch();
  if (notify) {
    PM.bus.emit('layers');
    PM.bus.emit('project');
  }
  PM.invalidate();
  if (notify && PM.Inspector && PM.Inspector.refresh) PM.Inspector.refresh();
}

const colorValue = value => typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value);
const primitiveControl = value => typeof value === 'boolean' ? 'toggle'
  : typeof value === 'number' ? 'slider'
    : colorValue(value) ? 'color'
      : typeof value === 'string' ? 'text' : null;
const numericRange = (value, hint = '') => {
  const n = Number(value) || 0;
  if (hint === 'opacity') return { min: 0, max: 100, step: 1 };
  if (hint.includes('rotation') || hint === 'angle') return { min: -360, max: 360, step: 1 };
  if (hint.includes('scale')) return { min: 0, max: 1000, step: .5 };
  if (hint.includes('position') || hint.includes('anchor')) return { min: -Math.max(PM.proj.w, PM.proj.h) * 2, max: Math.max(PM.proj.w, PM.proj.h) * 2, step: 1 };
  const span = Math.max(1, Math.abs(n) * 2);
  return { min: n < 0 ? -span : 0, max: span, step: span >= 100 ? 1 : span / 100 };
};

/* This catalog is the single capability map used by the agent and generated
   panels. It is derived from the live source, so adding a real editable field
   automatically makes it available instead of requiring another prompt-only
   whitelist. */
function sourceCatalog() {
  const p = PM.proj;
  const fill = PM.normalizeFill(p.backgroundFill, p.bg);
  const composition = [
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
  const parentOptions = [{ v: null, label: 'None' }, ...p.layers.map(layer => ({ v: layer.id, label: layer.name }))];
  const layers = p.layers.map(layer => {
    const fields = [
      { path: 'layer.name', label: 'Layer name', control: 'text', value: layer.name },
      { path: 'layer.from', label: 'Start time', control: 'slider', value: layer.from, min: 0, max: p.dur, step: 1 / Math.max(1, p.fps), unit: 's' },
      { path: 'layer.duration', label: 'Duration', control: 'slider', value: layer.dur, min: 1 / Math.max(1, p.fps), max: Math.max(p.dur, layer.dur), step: 1 / Math.max(1, p.fps), unit: 's' },
      { path: 'layer.visible', label: 'Visible', control: 'toggle', value: layer.on },
      { path: 'layer.locked', label: 'Locked', control: 'toggle', value: layer.lock },
      { path: 'layer.solo', label: 'Solo', control: 'toggle', value: layer.solo },
      { path: 'layer.shy', label: 'Shy', control: 'toggle', value: layer.shy },
      { path: 'layer.motionBlur', label: 'Motion blur', control: 'toggle', value: layer.mblur },
      { path: 'layer.collapsed', label: 'Collapsed', control: 'toggle', value: layer.collapsed },
      { path: 'layer.blend', label: 'Blend mode', control: 'select', value: layer.blend, options: [...PM.BLENDS] },
      { path: 'layer.parent', label: 'Parent', control: 'select', value: layer.parent, options: parentOptions.filter(option => option.v !== layer.id) },
      { path: 'layer.color', label: 'Label color', control: 'color', value: layer.color },
    ];
    const content = Object.entries(layer.d || {}).map(([key, value]) => {
      const control = primitiveControl(value); if (!control) return null;
      return { path: `content.${key}`, label: key, control, value, ...(control === 'slider' ? numericRange(value, key) : {}) };
    }).filter(Boolean);
    const properties = PM.allProps(layer).map(item => {
      const value = PM.evP(layer, item.prop, PM.time, item.key);
      const control = primitiveControl(value); if (!control) return null;
      const fxId = item.key.split('.')[0];
      const fx = (layer.fx || []).find(effect => effect.id === fxId);
      const fxParam = fx && PM.FX?.[fx.type]?.params?.find(param => param.k === item.key.slice(fxId.length + 1));
      const shaderDef = layer.type === 'shader' && item.key.startsWith('u.')
        ? (layer._udefs || PM.parseUniforms?.(layer.d.code) || []).find(def => `u.${def.name}` === item.key) : null;
      const meta = fxParam || shaderDef || PM.CH?.[item.key] || {};
      return {
        path: `properties.${item.key}`, label: meta.label || item.label || item.key,
        group: item.group, control: meta.type === 'color' ? 'color' : control, value,
        animatable: true, ...((meta.type === 'color' ? 'color' : control) === 'slider'
          ? { ...numericRange(value, item.key), ...Object.fromEntries(['min', 'max', 'step', 'unit'].filter(key => meta[key] !== undefined).map(key => [key, meta[key]])) }
          : {}),
      };
    }).filter(Boolean);
    return { id: layer.id, name: layer.name, type: layer.type, controls: [...fields, ...content, ...properties] };
  });
  return { target: '$composition', composition, layers, operations: Object.keys(Edit.operations) };
}

/* Higher-order generated tools compile to the same primitive edit language
   before a transaction starts. History therefore records the real layer/
   property changes, while one tool activation remains one atomic undo step. */
function expandCommands(input) {
  const expanded = [];
  for (const command of commands(input).filter(Boolean)) {
    if (command.type !== 'transform_layers') { expanded.push(command); continue; }
    if (!PM.Capabilities?.compile) throw new Error('Layer transformation capabilities are unavailable');
    const result = PM.Capabilities.compile(command.transform, command.state || {});
    if (!result.ok) throw new Error(result.message);
    expanded.push(...result.commands);
  }
  return expanded;
}

const Edit = {
  operations: Object.freeze({
    set_property: { target: 'layer', fields: ['path', 'value', 'time', 'mode', 'ease'] },
    replace_keyframes: { target: 'layer', fields: ['path', 'keyframes', 'replace', 'expression'] },
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
    set_scene_parameter: { target: 'project', fields: ['name', 'value'] },
    add_marker: { target: 'project', fields: ['time', 'name'] },
    create_section: { target: 'project', fields: ['section'] },
    update_section: { target: 'project', fields: ['sectionId', 'layers', 'thumb', 'version'] },
    transform_layers: { target: 'layer-collection', fields: ['transform', 'state'] },
  }),

  apply(input, meta = {}) {
    let list;
    try { list = expandCommands(input); }
    catch (error) { return fail(String(error.message || error)); }
    if (!list.length) return fail('No source edits supplied');
    if (meta.baseRevision != null && Number(meta.baseRevision) !== Number(PM.proj.revision || 0)) {
      return fail(`Project changed: expected revision ${meta.baseRevision}, found ${PM.proj.revision || 0}`);
    }
    if (live) {
      try {
        const results = list.map(command => ({ command: clone(command), data: runOne(command) }));
        list.forEach(rememberLive);
        changed(false);
        return pass('Source updated', { results, revision: PM.proj.revision || 0 });
      } catch (error) { return fail(String(error.message || error)); }
    }
    const before = JSON.stringify(PM.proj);
    const selection = clone(PM.sel);
    PM.hist.begin(meta.label || 'Edit source');
    try {
      const results = list.map(command => ({ command: clone(command), data: runOne(command) }));
      record(meta.label, meta.origin, list);
      changed();
      PM.hist.commit(meta.label || 'Edit source');
      return pass(meta.label || 'Source updated', { results, revision: PM.proj.revision });
    } catch (error) {
      PM.hist.cancel();
      restore(before, selection);
      return fail(String(error.message || error));
    }
  },

  begin(label, meta = {}) {
    if (live) throw new Error('A source edit is already active');
    live = { label: label || 'Edit source', origin: meta.origin || 'interface', before: JSON.stringify(PM.proj), selection: clone(PM.sel), commands: [] };
    PM.hist.begin(live.label);
  },

  dispatch(command) {
    if (!live) return Edit.apply(command);
    return Edit.apply(command, { origin: live.origin, label: live.label });
  },

  commit(label) {
    if (!live) return fail('No source edit is active');
    const transaction = live;
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
    const transaction = live;
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

  describeLayer(ref) {
    const layer = findLayer(ref);
    if (!layer) return null;
    return {
      id: layer.id,
      name: layer.name,
      type: layer.type,
      content: Object.entries(layer.d).filter(([, value]) => ['string', 'number', 'boolean'].includes(typeof value)).map(([key, value]) => ({
        path: `content.${key}`, value, command: { type: 'set_content', target: layer.id, patch: { [key]: value } },
      })),
      properties: PM.allProps(layer).map(item => ({
        path: `properties.${item.key}`, label: item.label, group: item.group,
        value: PM.evP(layer, item.prop, PM.time, item.key), animatable: true,
        command: { type: 'set_property', target: layer.id, path: item.key, value: PM.evP(layer, item.prop, PM.time, item.key) },
      })),
      operations: Object.keys(Edit.operations),
    };
  },

  sourceCatalog,
};

PM.Edit = Edit;
})();
