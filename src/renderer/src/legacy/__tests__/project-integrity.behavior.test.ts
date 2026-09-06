// @ts-nocheck -- faithful behavioral transplant of unique project-integrity oracle cases.
import assert from 'node:assert/strict';
import { it } from 'vitest';

import { makePM } from './make-pm';

function animModel() {
  const PM = makePM('core/easing', 'core/anim');
  PM.L = id => PM.proj.layers.find(layer => layer.id === id) || null;
  PM.curComp = () => PM.proj;
  return PM;
}

it('parenting cycles no longer freeze transforms after a depth cap', () => {
  const PM = animModel();
  const layer = (id, parent) => ({
    id, parent,
    p: { position: { x: 10, y: 0 } },
  });
  const L = (id, parent) => ({
    id, parent, from: 0,
    p: {
      'anchor.x': { v: 0, kf: [] }, 'anchor.y': { v: 0, kf: [] },
      'position.x': { v: 100, kf: [] }, 'position.y': { v: 0, kf: [] },
      'scale.x': { v: 100, kf: [] }, 'scale.y': { v: 100, kf: [] },
      rotation: { v: 0, kf: [] }, skew: { v: 0, kf: [] },
      opacity: { v: 100, kf: [] },
    },
  });
  const a = L('La', null), b = L('Lb', 'La'), c = L('Lc', 'Lb');
  PM.proj = { layers: [a, b, c], params: {}, fps: 30 };
  // a → c closes the loop
  a.parent = 'Lc';
  const m = PM.worldMatrix(a, 0);
  assert.ok(m.every(Number.isFinite), 'matrix stays finite in a cycle');
  assert.equal(PM.worldOpacity(a, 0), 1);
});

function projectModel() {
  return makePM('core/easing', 'core/model', 'core/selection', 'core/anim');
}

function baseProject(PM) {
  const p = PM.mkProject({ name: 'T', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.proj = p;
  return p;
}

it('precompose retains editable parenting across the comp boundary', () => {
  const PM = projectModel();
  const p = baseProject(PM);
  const inner = PM.mkLayer('shape', { name: 'Inner' }, p);
  const outer = PM.mkLayer('null', { name: 'Outer' }, p);
  inner.parent = outer.id;          // parent outside the future group
  p.layers.push(outer, inner);
  const L = PM.precompose([inner.id], 'Group');
  const sub = p.comps[L.d.comp];
  assert.ok(sub.layers.find(l => l.id === sub.layers[0].parent && l.type === 'null'), 'inbound parent preserved as a rig');
  assert.equal(outer.parent, null);
});


