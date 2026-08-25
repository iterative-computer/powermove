/* Powermove — safe, composable capabilities for generated tools.
   Tool manifests never execute arbitrary JavaScript. They resolve a bounded
   selector + expression graph into ordinary PM.Edit source commands, which
   keeps preview, apply, provenance, undo, and every native editor surface on
   the same project source. */
(() => {
const PM = window.PM;

const MAX_EDITS = 32;
const MAX_EXPR_DEPTH = 8;
const MAX_EXPR_NODES = 160;
const SAFE_KEY = /^[a-z][a-z0-9_.-]{0,79}$/i;
const LAYER_PATHS = new Set([
  'layer.name', 'layer.from', 'layer.duration', 'layer.visible', 'layer.locked',
  'layer.solo', 'layer.shy', 'layer.blend', 'layer.motionBlur', 'layer.parent',
  'layer.color', 'layer.collapsed',
]);
const SCOPES = new Set(['selection', 'all', 'visible']);
const ORDERS = new Set(['stack', 'reverseStack', 'selection', 'reverseSelection', 'start', 'reverseStart', 'name', 'random']);
const OPS = new Set(['add', 'subtract', 'multiply', 'divide', 'min', 'max', 'clamp', 'round', 'floor', 'ceil', 'abs', 'negate', 'frames', 'equal', 'if']);
const OP_ARITY = Object.freeze({
  add: [2, 8], subtract: [2, 2], multiply: [2, 8], divide: [2, 2], min: [1, 8], max: [1, 8],
  clamp: [3, 3], round: [1, 1], floor: [1, 1], ceil: [1, 1], abs: [1, 1], negate: [1, 1],
  frames: [1, 1], equal: [2, 2], if: [3, 3],
});
const REFS = new Set(['current', 'index', 'count', 'fps', 'playhead', 'composition.duration']);
const AGGREGATES = new Set(['min', 'max', 'first', 'last', 'sum', 'average']);
const DEFAULT_CURVE = Object.freeze([.62, .05, 0, 1]);

const clone = value => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
const cleanText = (value, fallback = '', max = 100) => typeof value === 'string' && value.trim()
  ? value.trim().slice(0, max) : fallback;
const finite = value => typeof value === 'number' && Number.isFinite(value);
const stateKey = value => typeof value === 'string' && SAFE_KEY.test(value) ? value : '';
const pathAllowed = path => LAYER_PATHS.has(path)
  || (/^properties\.[a-z0-9_.-]{1,120}$/i.test(path))
  || (/^content\.[a-z0-9_.-]{1,120}$/i.test(path));

function sanitizeCurve(value, fallback = null) {
  let source = value;
  if (typeof source === 'string') {
    source = PM.Ease?.PRESETS?.[source] || (() => { try { return JSON.parse(source); } catch { return null; } })();
  }
  if (!Array.isArray(source) || source.length !== 4 || !source.every(finite)) return fallback ? [...fallback] : null;
  return [
    PM.clamp(source[0], 0, 1), PM.clamp(source[1], -4, 4),
    PM.clamp(source[2], 0, 1), PM.clamp(source[3], -4, 4),
  ].map(value => PM.round(value, 3));
}

function sanitizeExpression(value, budget = { nodes: 0 }, depth = 0) {
  budget.nodes++;
  if (budget.nodes > MAX_EXPR_NODES || depth > MAX_EXPR_DEPTH) return null;
  if (value === null || typeof value === 'string' || typeof value === 'boolean' || finite(value)) return clone(value);
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  if (stateKey(value.state)) {
    const out = { state: value.state };
    if (value.fallback === null || typeof value.fallback === 'string' || typeof value.fallback === 'boolean' || finite(value.fallback)) out.fallback = clone(value.fallback);
    return out;
  }
  if (REFS.has(value.ref)) return { ref: value.ref };
  if (AGGREGATES.has(value.aggregate) && pathAllowed(value.path)) return { aggregate: value.aggregate, path: value.path };
  if (OPS.has(value.op)) {
    const sourceArgs = (Array.isArray(value.args) ? value.args : []).slice(0, 8);
    const args = [];
    for (const sourceArg of sourceArgs) {
      const arg = sanitizeExpression(sourceArg, budget, depth + 1);
      if (arg === null && sourceArg !== null) return null;
      args.push(arg);
    }
    const [minArgs, maxArgs] = OP_ARITY[value.op];
    if (args.length < minArgs || args.length > maxArgs) return null;
    return { op: value.op, args };
  }
  return null;
}

function sanitizeOrder(value) {
  if (ORDERS.has(value)) return value;
  if (value && typeof value === 'object' && stateKey(value.state)) {
    return { state: value.state, fallback: ORDERS.has(value.fallback) ? value.fallback : 'stack' };
  }
  return 'stack';
}

function sanitizeTransform(raw) {
  let source = raw;
  if (typeof source === 'string') {
    if (source.length > 80_000) return null;
    try { source = JSON.parse(source); } catch { return null; }
  }
  if (!source || typeof source !== 'object' || Array.isArray(source)) return null;
  const selectorSource = source.selector && typeof source.selector === 'object' ? source.selector : {};
  const selector = {
    scope: SCOPES.has(selectorSource.scope) ? selectorSource.scope : 'selection',
    /* Generated tools respect locked layers. Unlocking is a separate explicit
       edit, so a broad transform can never silently bypass that boundary. */
    includeLocked: false,
    types: (Array.isArray(selectorSource.types) ? selectorSource.types : [])
      .filter(type => typeof type === 'string' && PM.TYPE_META?.[type]).slice(0, 16),
  };
  const edits = (Array.isArray(source.edits) ? source.edits : []).slice(0, MAX_EDITS).map(edit => {
    const path = cleanText(edit?.path, '', 140);
    if (!pathAllowed(path)) return null;
    const expression = sanitizeExpression(edit?.value);
    if (expression === null && edit?.value !== null) return null;
    return { path, value: expression };
  }).filter(Boolean);
  if (!edits.length) return null;
  return {
    version: 1,
    label: cleanText(source.label, 'Transform layers', 80),
    selector,
    order: sanitizeOrder(source.order),
    seed: cleanText(source.seed, 'powermove', 80),
    edits,
  };
}

function sanitizeControlAction(raw) {
  let source = raw;
  if (typeof source === 'string') {
    if (source.length > 80_000) return null;
    try { source = JSON.parse(source); } catch { return null; }
  }
  if (!source || typeof source !== 'object' || Array.isArray(source)) return null;
  if (source.type === 'transform' && ['preview', 'apply'].includes(source.mode)) {
    const transform = sanitizeTransform(source.transform);
    return transform ? { type: 'transform', mode: source.mode, transform } : null;
  }
  if (source.type === 'history' && source.command === 'undo') return { type: 'history', command: 'undo' };
  if (source.type === 'reset') return { type: 'reset' };
  if (source.type === 'easing' && ['preview', 'apply'].includes(source.mode)) {
    const curveState = stateKey(source.curveState);
    if (!curveState || (source.scope && source.scope !== 'selected-keyframes')) return null;
    return {
      type: 'easing', mode: source.mode, scope: 'selected-keyframes', curveState,
      defaultCurve: sanitizeCurve(source.defaultCurve, DEFAULT_CURVE),
    };
  }
  if (source.type === 'script' && ['preview', 'apply'].includes(source.mode)) {
    const code = typeof source.code === 'string' && source.code.trim().length <= 40_000 ? source.code.trim() : '';
    if (!code) return null;
    return {
      type: 'script', mode: source.mode, code,
      label: cleanText(source.label, 'Run generated tool', 80),
      requiredTypes: (Array.isArray(source.requiredTypes) ? source.requiredTypes : [])
        .filter(type => typeof type === 'string' && PM.TYPE_META?.[type]).slice(0, 16),
    };
  }
  return null;
}

function getPath(layer, path) {
  if (path.startsWith('layer.')) {
    const key = path.slice(6);
    const sourceKey = ({ duration: 'dur', visible: 'on', locked: 'lock', motionBlur: 'mblur' })[key] || key;
    return layer[sourceKey];
  }
  if (path.startsWith('content.')) return layer.d?.[path.slice(8)];
  if (path.startsWith('properties.')) {
    const key = path.slice(11), prop = PM.findProp(layer, key);
    return prop ? PM.evP(layer, prop, PM.time, key) : undefined;
  }
  return undefined;
}

function resolveTargets(selector) {
  const projectLayers = PM.proj?.layers || [];
  let targets;
  if (selector.scope === 'selection') {
    const selected = new Set(PM.sel?.layers || []);
    targets = projectLayers.filter(layer => selected.has(layer.id));
  } else if (selector.scope === 'visible') targets = projectLayers.filter(layer => layer.on);
  else targets = [...projectLayers];
  if (selector.types.length) {
    const types = new Set(selector.types); targets = targets.filter(layer => types.has(layer.type));
  }
  if (!selector.includeLocked) targets = targets.filter(layer => !layer.lock);
  return targets;
}

function orderName(order, state) {
  if (typeof order === 'string') return order;
  const value = state?.[order.state];
  return ORDERS.has(value) ? value : order.fallback;
}

function stableHash(value) {
  let hash = 2166136261;
  for (const char of String(value)) { hash ^= char.charCodeAt(0); hash = Math.imul(hash, 16777619); }
  return hash >>> 0;
}

function orderedTargets(targets, order, state, seed) {
  const next = [...targets], selectedOrder = new Map((PM.sel?.layers || []).map((id, index) => [id, index]));
  switch (orderName(order, state)) {
    case 'reverseStack': return next.reverse();
    case 'selection': return next.sort((a, b) => (selectedOrder.get(a.id) ?? 1e9) - (selectedOrder.get(b.id) ?? 1e9));
    case 'reverseSelection': return next.sort((a, b) => (selectedOrder.get(b.id) ?? -1) - (selectedOrder.get(a.id) ?? -1));
    case 'start': return next.sort((a, b) => a.from - b.from || PM.proj.layers.indexOf(a) - PM.proj.layers.indexOf(b));
    case 'reverseStart': return next.sort((a, b) => b.from - a.from || PM.proj.layers.indexOf(a) - PM.proj.layers.indexOf(b));
    case 'name': return next.sort((a, b) => a.name.localeCompare(b.name));
    case 'random': return next.sort((a, b) => stableHash(`${seed}:${a.id}`) - stableHash(`${seed}:${b.id}`));
    default: return next;
  }
}

function aggregateValue(kind, path, targets) {
  const values = targets.map(layer => getPath(layer, path)).filter(value => finite(value));
  if (!values.length) return 0;
  if (kind === 'min') return Math.min(...values);
  if (kind === 'max') return Math.max(...values);
  if (kind === 'first') return values[0];
  if (kind === 'last') return values.at(-1);
  if (kind === 'sum') return values.reduce((sum, value) => sum + value, 0);
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function evaluate(expression, context) {
  if (expression === null || typeof expression !== 'object') return expression;
  if (Object.hasOwn(expression, 'state')) return context.state?.[expression.state] ?? expression.fallback ?? 0;
  if (expression.ref) {
    if (expression.ref === 'current') return context.current;
    if (expression.ref === 'index') return context.index;
    if (expression.ref === 'count') return context.targets.length;
    if (expression.ref === 'fps') return PM.proj.fps;
    if (expression.ref === 'playhead') return PM.time;
    if (expression.ref === 'composition.duration') return PM.proj.dur;
  }
  if (expression.aggregate) return aggregateValue(expression.aggregate, expression.path, context.targets);
  const args = (expression.args || []).map(arg => evaluate(arg, context));
  switch (expression.op) {
    case 'add': return args.reduce((sum, value) => Number(sum) + Number(value), 0);
    case 'subtract': return Number(args[0]) - Number(args[1] || 0);
    case 'multiply': return args.reduce((product, value) => Number(product) * Number(value), 1);
    case 'divide': return Number(args[1]) === 0 ? Number(args[0]) : Number(args[0]) / Number(args[1]);
    case 'min': return Math.min(...args.map(Number));
    case 'max': return Math.max(...args.map(Number));
    case 'clamp': return PM.clamp(Number(args[0]), Number(args[1]), Number(args[2]));
    case 'round': return Math.round(Number(args[0]));
    case 'floor': return Math.floor(Number(args[0]));
    case 'ceil': return Math.ceil(Number(args[0]));
    case 'abs': return Math.abs(Number(args[0]));
    case 'negate': return -Number(args[0]);
    case 'frames': return Number(args[0]) / Math.max(1, PM.proj.fps || 30);
    case 'equal': return args[0] === args[1];
    case 'if': return args[0] ? args[1] : args[2];
    default: return 0;
  }
}

function normalizedValue(path, value) {
  if (path === 'layer.from') return Math.max(0, PM.snapF(Number(value) || 0, PM.proj.fps));
  if (path === 'layer.duration') return Math.max(1 / Math.max(1, PM.proj.fps), PM.snapF(Number(value) || 0, PM.proj.fps));
  return value;
}

function compile(raw, state = {}) {
  const transform = sanitizeTransform(raw);
  if (!transform) return { ok: false, message: 'The generated transformation is invalid', commands: [], changes: [] };
  const targets = orderedTargets(resolveTargets(transform.selector), transform.order, state, transform.seed);
  if (!targets.length) return { ok: false, message: transform.selector.scope === 'selection' ? 'Select at least one unlocked layer' : 'No unlocked layers match this tool', commands: [], changes: [] };
  const byLayer = new Map();
  const changes = [];
  for (let index = 0; index < targets.length; index++) {
    const layer = targets[index];
    for (const edit of transform.edits) {
      const before = getPath(layer, edit.path);
      if (before === undefined) continue;
      const value = normalizedValue(edit.path, evaluate(edit.value, { current: before, index, layer, targets, state }));
      if (Object.is(before, value)) continue;
      const entry = byLayer.get(layer.id) || { layer, layerPatch: {}, contentPatch: {}, properties: [] };
      if (edit.path.startsWith('layer.')) entry.layerPatch[edit.path.slice(6)] = value;
      else if (edit.path.startsWith('content.')) entry.contentPatch[edit.path.slice(8)] = value;
      else entry.properties.push({ type: 'set_property', target: layer.id, path: edit.path.slice(11), value, time: PM.time, mode: 'auto', preserveHandEdits: true });
      byLayer.set(layer.id, entry);
      changes.push({ target: layer.id, name: layer.name, path: edit.path, before: clone(before), after: clone(value) });
    }
  }
  const commands = [];
  for (const entry of byLayer.values()) {
    if (Object.keys(entry.layerPatch).length) commands.push({ type: 'set_layer', target: entry.layer.id, patch: entry.layerPatch });
    if (Object.keys(entry.contentPatch).length) commands.push({ type: 'set_content', target: entry.layer.id, patch: entry.contentPatch });
    commands.push(...entry.properties);
  }
  if (!commands.length) return { ok: false, message: 'This transformation would not change the selected source', transform, targets, commands, changes };
  return { ok: true, message: `${changes.length} source change${changes.length === 1 ? '' : 's'} ready`, transform, targets, commands, changes };
}

function preview(raw, state) { return compile(raw, state); }

function apply(raw, state, meta = {}) {
  const result = compile(raw, state);
  if (!result.ok) return result;
  const applied = PM.Edit.apply(result.commands, {
    label: meta.label || result.transform.label,
    origin: meta.origin || 'generated-tool',
    baseRevision: meta.baseRevision,
  });
  return { ...result, ...applied, preview: result };
}

function selectionSummary() {
  const selected = PM.selLayers?.() || [];
  const unlocked = selected.filter(layer => !layer.lock).length;
  if (!selected.length) return 'No layers selected';
  if (unlocked !== selected.length) return `${unlocked} of ${selected.length} layers editable`;
  return `${selected.length} layer${selected.length === 1 ? '' : 's'} selected`;
}

function selectedKeyframes() {
  return PM.resolveSelectedKeys?.() || [];
}

function keyframeSummary() {
  const count = selectedKeyframes().length;
  return count ? `${count} keyframe${count === 1 ? '' : 's'} selected` : 'No keyframes selected';
}

function easingResult(action, state = {}) {
  const keys = selectedKeyframes();
  if (!keys.length) return { ok: false, message: 'Select at least one keyframe', commands: [], changes: [] };
  const curve = sanitizeCurve(state[action.curveState], action.defaultCurve || DEFAULT_CURVE);
  const name = PM.Ease?.nameOf?.([curve[0], curve[1]], [curve[2], curve[3]]) || 'custom';
  const changes = keys.map(key => ({
    target: key.i, name: 'Keyframe', path: 'easing',
    before: PM.Ease?.nameOf?.(key.eo || [0, 0], key.ei || [1, 1]) || 'custom', after: name,
  }));
  return {
    ok: true, message: `${keys.length} keyframe${keys.length === 1 ? '' : 's'} ready`,
    curve, changes, commands: [{ type: 'set_easing', keyframes: keys.map(key => key.i), curve }],
  };
}

function previewEasing(action, state) { return easingResult(action, state); }
function applyEasing(action, state, meta = {}) {
  const result = easingResult(action, state);
  if (!result.ok) return result;
  const applied = PM.Edit.apply(result.commands, {
    label: meta.label || 'Apply easing', origin: meta.origin || 'generated-tool',
  });
  return { ...result, ...applied, changes: result.changes };
}

function staggerTransform(mode = 'apply') {
  return {
    type: 'transform', mode,
    transform: {
      version: 1, label: 'Stagger layers', selector: { scope: 'selection' },
      order: { state: 'order', fallback: 'stack' },
      edits: [{
        path: 'layer.from',
        value: { op: 'max', args: [0, { op: 'add', args: [
          { op: 'if', args: [
            { op: 'equal', args: [{ state: 'anchor' }, 'playhead'] },
            { ref: 'playhead' },
            { aggregate: 'min', path: 'layer.from' },
          ] },
          { op: 'multiply', args: [{ ref: 'index' }, { op: 'frames', args: [{ state: 'offsetFrames' }] }] },
        ] }] },
      }],
    },
  };
}

const DECOMPOSE_TEXT_SCRIPT = `
const source = PM.selectedLayers.find(layer => layer.type === 'text' && !layer.locked);
PM.assert(source, 'Select one unlocked text layer');
const mode = ['characters', 'words', 'lines'].includes(PM.input.mode) ? PM.input.mode : 'characters';
const pieces = source.textLayout?.[mode] || [];
PM.assert(pieces.length, 'The selected text has no visible ' + mode + ' to decompose');
PM.assert(pieces.length <= 220, 'This text contains more than 220 pieces; split it in smaller sections first');
const commands = pieces.map((piece, index) => PM.addLayer({
  id: PM.uid('text-piece'),
  layerType: 'text',
  name: source.name + ' · ' + String(index + 1).padStart(2, '0') + ' · ' + piece.text,
  from: source.from,
  duration: source.duration,
  content: { ...PM.clone(source.content), text: piece.text, align: 'left' },
  properties: {
    'anchor.x': 0, 'anchor.y': 0,
    'position.x': piece.x, 'position.y': piece.y,
    'scale.x': 100, 'scale.y': 100,
    rotation: 0, opacity: 100, skew: 0,
  },
  color: source.color,
  parent: source.id,
  blend: source.blend,
  motionBlur: source.motionBlur,
  index: source.index + index,
  select: false,
}));
if (PM.input.original !== 'keep') commands.push(PM.setLayer(source.id, { visible: false }));
return commands;`;

function decomposeTextAction(mode = 'apply') {
  return {
    type: 'script', mode, code: DECOMPOSE_TEXT_SCRIPT,
    label: mode === 'apply' ? 'Decompose text' : 'Preview text decomposition',
    requiredTypes: ['text'],
  };
}

function panelRecipe(id) {
  if (id === 'easing-flow') return {
    id: 'easing-flow', title: 'Easing Flow', size: 430,
    note: 'Shape a reusable timing curve, then apply it to the selected keyframes as one editable source change.',
    state: { curve: [...DEFAULT_CURVE] },
    controls: [
      { type: 'readout', label: 'Target', source: 'keyframes.summary' },
      { type: 'curve', label: 'Easing curve', stateKey: 'curve', def: [...DEFAULT_CURVE], minY: -1, maxY: 2,
        presets: ['linear', 'easeIn', 'easeOut', 'easeInOut', 'power', 'snap', 'glide', 'backOut'] },
      { type: 'button', label: 'Preview', action: { type: 'easing', mode: 'preview', scope: 'selected-keyframes', curveState: 'curve', defaultCurve: [...DEFAULT_CURVE] } },
      { type: 'button', label: 'Apply easing', primary: true, action: { type: 'easing', mode: 'apply', scope: 'selected-keyframes', curveState: 'curve', defaultCurve: [...DEFAULT_CURVE] } },
      { type: 'button', label: 'Undo last edit', action: { type: 'history', command: 'undo' } },
    ],
  };
  if (id === 'layer-stagger') return {
    id: 'layer-stagger', title: 'Layer Stagger', size: 300,
    note: 'Offset complete selected layers while preserving their duration and editable source.',
    state: { offsetFrames: 2, order: 'stack', anchor: 'earliest' },
    controls: [
      { type: 'readout', label: 'Selection', source: 'selection.summary' },
      { type: 'slider', label: 'Offset', stateKey: 'offsetFrames', def: 2, min: -120, max: 120, step: 1, unit: ' fr' },
      { type: 'select', label: 'Order', stateKey: 'order', def: 'stack', options: [
        { v: 'stack', label: 'Top → Bottom' }, { v: 'reverseStack', label: 'Bottom → Top' },
        { v: 'selection', label: 'Selection order' }, { v: 'start', label: 'Current timing' },
        { v: 'random', label: 'Random' },
      ] },
      { type: 'select', label: 'Anchor', stateKey: 'anchor', def: 'earliest', options: [
        { v: 'earliest', label: 'Earliest layer' }, { v: 'playhead', label: 'Playhead' },
      ] },
      { type: 'button', label: 'Preview', action: staggerTransform('preview') },
      { type: 'button', label: 'Apply Stagger', primary: true, action: staggerTransform('apply') },
      { type: 'button', label: 'Undo last edit', action: { type: 'history', command: 'undo' } },
    ],
  };
  if (id === 'decompose-text') return {
    id: 'decompose-text', title: 'Decompose Text', size: 300,
    note: 'Split the selected text into independently editable, correctly positioned text layers.',
    state: { mode: 'characters', original: 'hide' },
    controls: [
      { type: 'readout', label: 'Selection', source: 'selection.summary' },
      { type: 'select', label: 'Split into', stateKey: 'mode', def: 'characters', options: [
        { v: 'characters', label: 'Characters' }, { v: 'words', label: 'Words' }, { v: 'lines', label: 'Lines' },
      ] },
      { type: 'select', label: 'Original', stateKey: 'original', def: 'hide', options: [
        { v: 'hide', label: 'Hide and preserve' }, { v: 'keep', label: 'Keep visible' },
      ] },
      { type: 'button', label: 'Preview', action: decomposeTextAction('preview') },
      { type: 'button', label: 'Decompose', primary: true, action: decomposeTextAction('apply') },
      { type: 'button', label: 'Undo last edit', action: { type: 'history', command: 'undo' } },
    ],
  };
  return null;
}

function catalog() {
  return {
    version: 1,
    generatedTools: ['easing-flow', 'layer-stagger', 'decompose-text'],
    scripting: PM.Script?.catalog?.() || { language: 'sandboxed-javascript', status: 'loads after capabilities' },
    selectors: [...SCOPES],
    orders: [...ORDERS],
    paths: ['layer.*', 'properties.*', 'content.*'],
    expressions: [...OPS],
    visualControls: ['curve'],
    sourceActions: ['easing', 'transform', 'script'],
    guarantees: ['locked layers skipped', 'dry-run preview', 'validated source commands', 'one-step undo'],
  };
}

PM.Capabilities = {
  sanitizeTransform, sanitizeControlAction, sanitizeCurve, resolveTargets, compile, preview, apply,
  previewEasing, applyEasing, selectedKeyframes, panelRecipe, selectionSummary, keyframeSummary, catalog,
  test: { sanitizeExpression, evaluate, orderedTargets, getPath },
};
})();
