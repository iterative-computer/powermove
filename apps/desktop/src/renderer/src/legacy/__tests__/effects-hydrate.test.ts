// @ts-nocheck -- legacy PM is intentionally a dynamic registry.
/*
 * Hydration must not destroy data it cannot render. An effect whose type is not
 * registered (its extension is off, or not loaded yet) survives boot as a
 * `missing` placeholder, keyframes and all, instead of being filtered away.
 */
import assert from 'node:assert/strict';
import { afterEach, it, vi } from 'vitest';

import { EFFECTS } from '../../../../extensions/effects-basic/effects';
import { install as installApp } from '../app';
import { makePM } from './make-pm';

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');

afterEach(() => {
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
  else delete globalThis.window;
});

const noop = () => {};
const element = () => ({
  dataset: {}, style: {}, classList: { add: noop, remove: noop },
  append: noop, addEventListener: noop, remove: noop, focus: noop, select: noop, click: noop
});

function registerEffects(PM) {
  for (const definition of EFFECTS) PM.Kernel.registerEffect('effects-basic', definition);
}

function bootWith(fx, { registerBefore = false } = {}) {
  const PM = makePM(
    'core/easing', 'core/model', 'core/selection', 'core/anim',
    'core/history', 'core/editing', 'gl/shaders'
  );
  if (registerBefore) registerEffects(PM);
  PM.proj = PM.mkProject({ name: 'Effects', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.Kernel.services.register('shaderHooks', { syncShaderUniforms: noop });
  const layer = PM.mkLayer('shape', { name: 'Signal', from: 0, dur: 5 });
  layer.id = 'signal';
  layer.fx = fx;
  PM.addLayer(layer, 0);
  const raw = JSON.parse(JSON.stringify(PM.proj));

  Object.defineProperty(globalThis, 'window', {
    value: {
      document: { documentElement: element(), body: element(), createElement: element, getElementById: element },
      matchMedia: () => ({ matches: false, addEventListener: noop }),
      addEventListener: noop, removeEventListener: noop,
      requestAnimationFrame: () => 1, setTimeout: () => 1, clearTimeout: noop, setInterval: () => 1,
      Blob
    },
    configurable: true
  });
  Object.assign(PM, {
    bootVersion: 0,
    Projects: {
      pickBoot: () => raw, list: () => [{ id: raw.id, name: raw.name }], get: () => raw,
      put: noop, remove: noop, getState: () => null, putState: noop, openProjects: () => [],
      tabs: () => [raw.id], rename: noop
    },
    WS: { init: noop, restoreSnapshot: noop, snapshot: () => ({}), editing: false },
    setTime(value) { PM.time = value; },
    Audio: { normalizeLayer: noop },
    rasterClear: noop,
    assets: { restoreProject: async () => ({ stale: true, missing: [] }), clear: noop },
    Export: { snapshot: () => '' },
    h: () => element(), $: () => element(), icon: element, PANELS: {}, perf: {}
  });
  PM.Kernel.services.register('timeline', { pps: 90, scrollT: 0, scrollY: 0, graph: false, frameView: noop });
  PM.Kernel.services.register('viewer', { layout: noop });
  PM.Kernel.services.register('inspector', { refresh: noop });
  installApp(PM);
  return PM;
}

it('hydrate keeps an unknown effect as a missing placeholder with its parameters', () => {
  const PM = bootWith([
    { id: 'fx1', type: 'removed-extension-effect', on: true, p: { amount: { v: 7, kf: [], expr: null } } }
  ]);

  const [effect] = PM.proj.layers[0].fx;
  assert.equal(effect.type, 'removed-extension-effect');
  assert.equal(effect.missing, true);
  assert.equal(effect.p.amount.v, 7);
});

it('hydrate leaves a known effect unmarked', () => {
  const PM = bootWith([{ id: 'fx1', type: 'blur', on: true, p: {} }], { registerBefore: true });

  const [effect] = PM.proj.layers[0].fx;
  assert.equal(effect.type, 'blur');
  assert.equal(effect.missing, undefined);
});

it('hydrate clears a stale missing flag once the type is registered again', () => {
  const PM = bootWith([{ id: 'fx1', type: 'blur', on: true, missing: true, p: {} }], { registerBefore: true });

  assert.equal(PM.proj.layers[0].fx[0].missing, undefined);
});

it('recovers a blur placeholder when effects-basic registers after hydration', () => {
  const PM = bootWith([{ id: 'fx1', type: 'blur', on: true, p: { amount: { v: 17, kf: [], expr: null } } }]);
  const invalidate = vi.fn();
  PM.invalidate = invalidate;

  assert.equal(PM.proj.layers[0].fx[0].missing, true);
  registerEffects(PM);

  assert.equal(PM.proj.layers[0].fx[0].missing, undefined);
  assert.equal(PM.proj.layers[0].fx[0].p.amount.v, 17);
  assert.equal(invalidate.mock.calls.length > 0, true);
});

it('hydrate still drops entries that are not effect objects', () => {
  const PM = bootWith([null, 42, { id: 'fx1', type: 'blur', on: true, p: {} }], { registerBefore: true });

  assert.equal(PM.proj.layers[0].fx.length, 1);
});

it('migrates unversioned saved effects to revision one without replacing their controls or animation', () => {
  const PM = bootWith([{ id: 'legacy-invert', type: 'invert', on: true,
    p: { amount: { v: 37, kf: [], expr: 'value + 1' } } }], { registerBefore: true });
  const effect = PM.proj.layers[0].fx[0];
  assert.equal(effect.version, 1);
  assert.equal(effect.missing, undefined);
  assert.equal(effect.p.amount.v, 37);
  assert.equal(effect.p.amount.expr, 'value + 1');
  assert.deepEqual(Object.keys(effect.p), ['amount']);
  assert.deepEqual(PM.effectDef(effect).params.map(param => param.k), ['amount']);
});

it('keeps a newer unavailable revision missing with its editable channels intact', () => {
  const PM = bootWith([{ id: 'future-invert', type: 'invert', version: 99, on: true,
    p: { futureChannel: { v: 17, kf: [], expr: null } } }], { registerBefore: true });
  const effect = PM.proj.layers[0].fx[0];
  assert.equal(effect.version, 99); assert.equal(effect.missing, true);
  assert.equal(effect.p.futureChannel.v, 17);
  registerEffects(PM);
  assert.equal(effect.missing, true); assert.equal(effect.p.futureChannel.v, 17);
});


it('retains future layer-input revisions and initializes known inputs outside channels', () => {
  const inputs = { map: { layerId: 'deleted-layer', stage: 'future-stage', future: 7 } };
  const PM = bootWith([{ id: 'future-map', type: 'setMatte', version: 999, on: true, inputs, p: {} },
    { id: 'known-map', type: 'displacementMap', version: 1, on: true, p: {} }], { registerBefore: true });
  const [future, known] = PM.proj.layers[0].fx;
  assert.equal(future.missing, true); assert.deepEqual(future.inputs, inputs);
  assert.deepEqual(known.inputs, { map: { layerId: null, stage: 'source' } });
  assert.equal(known.p.map, undefined);
});
