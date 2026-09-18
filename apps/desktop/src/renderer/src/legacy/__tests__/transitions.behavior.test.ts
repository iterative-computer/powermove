// @ts-nocheck -- legacy PM is intentionally a dynamic registry.
import assert from 'node:assert/strict';
import { afterEach, it } from 'vitest';

import activateTransitionsBasic from '../../../../extensions/transitions-basic';
import { BUILTIN_TRANSITIONS } from '../../../../extensions/transitions-basic/transitions';
import { install as installApp } from '../app';
import { install as installTransitions } from '../gl/transitions';
import { makePM } from './make-pm';

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');

afterEach(() => {
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
  else delete globalThis.window;
});

function editor() {
  const PM = makePM(
    'core/easing', 'core/model', 'core/selection', 'core/anim',
    'core/history', 'core/editing', 'gl/shaders',
  );
  installTransitions(PM);
  activateTransitionsBasic({
    transitions: {
      register: definition => PM.Kernel.registerTransition('transitions-basic', definition),
    },
  });
  PM.proj = PM.mkProject({ name: 'Transitions', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 1;
  PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
  const layer = PM.mkLayer('text', { name: 'Title', from: 0, dur: 5 });
  layer.id = 'title';
  PM.addLayer(layer, 0);
  PM.selectLayers(layer.id);
  return { PM, layer };
}

function apply(PM, command, origin = 'inspector') {
  return PM.Edit.apply(command, { label: 'Transition', origin });
}

it('registers the four built-in definitions in the fallback registry', () => {
  const { PM } = editor();
  assert.deepEqual(BUILTIN_TRANSITIONS.map(definition => definition.id), ['crossfade', 'wipe', 'slide', 'zoom']);
  assert.deepEqual(Object.keys(PM.TRANSITIONS), ['crossfade', 'wipe', 'slide', 'zoom']);
});

it('provides a standalone GLSL transition preamble and named uniforms', () => {
  const { PM } = editor();
  assert.match(PM.TRANSITION_PRE, /^#version 300 es/);
  assert.match(PM.TRANSITION_PRE, /uniform sampler2D u_from, u_to;/);
  assert.match(PM.TRANSITION_PRE, /float fbm\(vec2 p\)/);
  assert.match(PM.transitionDef('wipe').frag, /uniform float u_angle;/);
  assert.match(PM.transitionDef('wipe').frag, /uniform float u_softness;/);
});

it('prefers a kernel transition and its legacy converter', () => {
  const { PM } = editor();
  const kernelDefinition = { id: 'custom', label: 'Custom', params: [], frag: 'o=vec4(1.);' };
  let converted = null;
  PM.Kernel = {
    transitions: { get: type => type === 'custom' ? kernelDefinition : undefined },
    glsl: { toLegacyTransition(definition, preamble) {
      converted = { definition, preamble };
      return { label: 'Converted', group: 'Test', params: [], frag: 'converted' };
    } },
  };
  assert.equal(PM.transitionDef('custom').frag, 'converted');
  assert.equal(converted.definition, kernelDefinition);
  assert.equal(converted.preamble, PM.TRANSITION_PRE);
});

it('locally converts kernel definitions when no GLSL helper is installed', () => {
  const { PM } = editor();
  PM.Kernel = { transitions: { get: () => ({
    id: 'tint', label: 'Tint', params: [{ k: 'color', label: 'Color', def: '#112233', type: 'color' }],
    frag: 'o=vec4(u_color,1.);',
  }) } };
  const definition = PM.transitionDef('tint');
  assert.match(definition.frag, /uniform vec3 u_color;/);
  assert.match(definition.frag, /void main\(\)/);
});

it('passes raw kernel shaders through without wrapping them', () => {
  const { PM } = editor();
  const shader = '#version 300 es\nvoid main(){}';
  PM.Kernel = { transitions: { get: () => ({ id: 'raw', label: 'Raw', params: [], frag: shader, rawShader: true }) } };
  assert.equal(PM.transitionDef('raw').frag, shader);
});

it('mkTransition creates independent channels with parameter defaults', () => {
  const { PM } = editor();
  const first = PM.mkTransition('wipe');
  const second = PM.mkTransition('wipe');
  assert.equal(first.dur, 0.5);
  assert.deepEqual(Object.keys(first.p), ['angle', 'softness']);
  assert.deepEqual(first.p.angle, { v: 0, kf: [], expr: null });
  first.p.angle.v = 90;
  assert.equal(second.p.angle.v, 0);
});

it('new layers default both transition edges to null and clone them deeply', () => {
  const { PM, layer } = editor();
  assert.equal(layer.transitionIn, null);
  assert.equal(layer.transitionOut, null);
  layer.transitionIn = PM.mkTransition('wipe');
  const clone = PM.cloneLayer(layer);
  clone.transitionIn.p.angle.v = 45;
  assert.equal(layer.transitionIn.p.angle.v, 0);
});

it('set_transition applies defaults, duration, and parameter values', () => {
  const { PM, layer } = editor();
  const result = apply(PM, { type: 'set_transition', layer: layer.id, edge: 'in', transition: {
    type: 'wipe', dur: 1.25, p: { angle: 90, softness: 12 },
  } });
  assert.equal(result.ok, true, result.message);
  assert.equal(layer.transitionIn.type, 'wipe');
  assert.equal(layer.transitionIn.dur, 1.25);
  assert.deepEqual([layer.transitionIn.p.angle.v, layer.transitionIn.p.softness.v], [90, 12]);
});

it('set_transition clears either edge with null', () => {
  const { PM, layer } = editor();
  layer.transitionOut = PM.mkTransition('crossfade');
  const result = apply(PM, { type: 'set_transition', layer: layer.id, edge: 'out', transition: null });
  assert.equal(result.ok, true, result.message);
  assert.equal(layer.transitionOut, null);
});

it('set_transition is one undoable and redoable edit', () => {
  const { PM, layer } = editor();
  assert.equal(apply(PM, { type: 'set_transition', layer: layer.id, edge: 'in', transition: { type: 'zoom' } }).ok, true);
  assert.equal(PM.hist.undo(), true);
  assert.equal(PM.L(layer.id).transitionIn, null);
  assert.equal(PM.hist.redo(), true);
  assert.equal(PM.L(layer.id).transitionIn.type, 'zoom');
});

it('set_transition automatically obeys layer locking', () => {
  const { PM, layer } = editor();
  layer.lock = true;
  const result = apply(PM, { type: 'set_transition', layer: layer.id, edge: 'in', transition: { type: 'crossfade' } });
  assert.equal(result.ok, false);
  assert.match(result.message, /locked/i);
});

it('set_transition rejects an invalid edge and unknown type', () => {
  const { PM, layer } = editor();
  assert.equal(apply(PM, { type: 'set_transition', layer: layer.id, edge: 'middle', transition: { type: 'wipe' } }).ok, false);
  const unknown = apply(PM, { type: 'set_transition', layer: layer.id, edge: 'in', transition: { type: 'missing' } });
  assert.equal(unknown.ok, false);
  assert.match(unknown.message, /Unknown transition/);
});

it('set_transition validates duration bounds and parameter keys and values', () => {
  const { PM, layer } = editor();
  const command = transition => ({ type: 'set_transition', layer: layer.id, edge: 'in', transition });
  assert.equal(apply(PM, command({ type: 'wipe', dur: 0.019 })).ok, false);
  assert.equal(apply(PM, command({ type: 'wipe', dur: 601 })).ok, false);
  assert.equal(apply(PM, command({ type: 'wipe', p: { nope: 1 } })).ok, false);
  assert.equal(apply(PM, command({ type: 'wipe', p: { angle: Number.NaN } })).ok, false);
});

it('set_transition validates color and toggle parameter types', () => {
  const { PM, layer } = editor();
  PM.Kernel = { transitions: { get: type => type === 'typed' ? {
    id: 'typed', label: 'Typed', frag: 'o=texture(u_to,v_st);', params: [
      { k: 'tint', label: 'Tint', def: '#FFFFFF', type: 'color' },
      { k: 'reverse', label: 'Reverse', def: false, type: 'toggle' },
    ],
  } : undefined } };
  const command = p => ({ type: 'set_transition', layer: layer.id, edge: 'out', transition: { type: 'typed', p } });
  assert.equal(apply(PM, command({ tint: 'red' })).ok, false);
  assert.equal(apply(PM, command({ reverse: 1 })).ok, false);
  assert.equal(apply(PM, command({ tint: '#AABBCC', reverse: true })).ok, true);
});

it('transition params are editable sources in the catalog and layer description', () => {
  const { PM, layer } = editor();
  assert.equal(apply(PM, { type: 'set_transition', layer: layer.id, edge: 'in', transition: { type: 'wipe' } }).ok, true);
  const controls = PM.Edit.sourceCatalog().layers.find(item => item.id === layer.id).controls;
  assert.equal(controls.some(control => control.path === 'transitionIn.p.angle'), true);
  assert.equal(PM.Edit.describeLayer(layer.id).properties.some(property => property.path === 'transitionIn.p.softness'), true);
  assert.equal(apply(PM, { type: 'set_property', target: layer.id, path: 'transitionIn.p.angle', value: 33, mode: 'static' }).ok, true);
  assert.equal(layer.transitionIn.p.angle.v, 33);
});

it('serialize and deserialize retain transition channel data', () => {
  const { PM, layer } = editor();
  layer.transitionOut = PM.mkTransition('zoom');
  layer.transitionOut.dur = 0.8;
  layer.transitionOut.p.amount.v = 60;
  const project = PM.deserialize(PM.serialize());
  assert.equal(project.layers[0].transitionOut.dur, 0.8);
  assert.equal(project.layers[0].transitionOut.p.amount.v, 60);
});

it('hydrate preserves unknown transitions as disabled missing placeholders', () => {
  const { PM, layer } = editor();
  layer.transitionIn = { type: 'removed-extension-transition', dur: 2, p: { amount: { v: 7, kf: [], expr: null } } };
  const raw = JSON.parse(JSON.stringify(PM.proj));
  const noop = () => {};
  const element = () => ({ dataset: {}, style: {}, classList: { add: noop, remove: noop }, append: noop, addEventListener: noop, remove: noop, focus: noop, select: noop, click: noop });
  const listeners = new Map();
  const fakeWindow = {
    document: { documentElement: element(), body: element(), createElement: element, getElementById: element },
    matchMedia: () => ({ matches: false, addEventListener: noop }),
    addEventListener: (name, callback) => listeners.set(name, callback), removeEventListener: noop,
    requestAnimationFrame: () => 1, setTimeout: () => 1, clearTimeout: noop, setInterval: () => 1,
    Blob,
  };
  Object.defineProperty(globalThis, 'window', { value: fakeWindow, configurable: true });
  Object.assign(PM, {
    bootVersion: 0,
    Projects: {
      pickBoot: () => raw, list: () => [{ id: raw.id, name: raw.name }], get: () => raw,
      put: noop, remove: noop, getState: () => null, putState: noop, openProjects: () => [raw.id], rename: noop,
    },
    WS: { init: noop, restoreSnapshot: noop, snapshot: () => ({}), editing: false },
    setTime(value) { PM.time = value; },
    Audio: { normalizeLayer: noop },
    rasterClear: noop,
    assets: { restoreProject: async () => ({ stale: true, missing: [] }), clear: noop },
    Export: { snapshot: () => '' },
    h: () => element(), $: () => element(), icon: element, PANELS: {}, perf: {},
  });
  PM.Kernel.services.register('timeline', { pps: 90, scrollT: 0, scrollY: 0, graph: false, frameView: noop });
  PM.Kernel.services.register('viewer', { layout: noop });
  PM.Kernel.services.register('inspector', { refresh: noop });
  installApp(PM);
  assert.equal(PM.proj.layers[0].transitionIn.type, 'removed-extension-transition');
  assert.equal(PM.proj.layers[0].transitionIn.missing, true);
  assert.equal(PM.proj.layers[0].transitionIn.p.amount.v, 7);
});

it('keeps keyframes on transition params when the same type is re-applied', () => {
  const { PM, layer } = editor();
  const first = apply(PM, { type: 'set_transition', layer: layer.id, edge: 'in', transition: { type: 'wipe', dur: 0.5 } });
  assert.equal(first.ok, true);
  PM.setKeyOn(layer.transitionIn.p.angle, 0, 0);
  PM.setKeyOn(layer.transitionIn.p.angle, 1, 90);
  assert.equal(layer.transitionIn.p.angle.kf.length, 2);
  assert.equal(apply(PM, { type: 'set_transition', layer: layer.id, edge: 'in', transition: { type: 'wipe', dur: 0.8 } }).ok, true);
  assert.equal(layer.transitionIn.dur, 0.8);
  assert.equal(layer.transitionIn.p.angle.kf.length, 2, 'keyframes survive a duration edit');
  assert.equal(apply(PM, { type: 'set_transition', layer: layer.id, edge: 'in', transition: { type: 'crossfade' } }).ok, true);
  assert.equal(layer.transitionIn.p.angle, undefined, 'changing type rebuilds channels');
});
