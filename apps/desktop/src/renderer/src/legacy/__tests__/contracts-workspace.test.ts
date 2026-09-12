// @ts-nocheck -- faithful behavioral transplant of the frozen workspace-validator oracle.
import assert from 'node:assert/strict';
import { it } from 'vitest';

import { makePM } from './make-pm';

function workspaceModel() {
  const PM = makePM('core/workspace');
  PM.proj = { id: 'contract-project' };
  return PM.WS;
}

function manifest(overrides = {}) {
  return {
    id: 'contract',
    name: 'Contract',
    layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] },
    ...overrides,
  };
}

/* tests/library-workspaces.test.js already freezes workspace version/scope,
   the basic missing-viewer repair, Timeline migrations, and trash recovery.
   These tests cover only the remaining validator boundaries. */

it('normalization adds one flex dock only when none exists and preserves multiple authored flex docks', () => {
  const WS = workspaceModel();
  const zeroFlex = WS.normalize(manifest({
    hiddenPanels: [{ id: 'viewer', dockId: 'right', spec: { id: 'viewer' } }],
    layout: { docks: [
      { id: 'left', panels: [{ id: 'assets', size: 180 }] },
      { id: 'main', panels: [{ id: 'timeline', size: 240 }] },
    ] },
  }));

  const docks = zeroFlex.layout.docks;
  const viewerDock = docks.find(dock => dock.panels.some(panel => panel.id === 'viewer'));
  assert.ok(viewerDock, 'viewer is always visible');
  assert.equal(viewerDock.flex, true, 'the viewer dock consumes unused width');
  assert.equal(docks.filter(dock => dock.flex).length, 1, 'zero-flex input gains one fluid dock');
  assert.equal(zeroFlex.hiddenPanels.some(panel => panel.id === 'viewer'), false);
  for (const dock of docks) {
    assert.equal(dock.panels.some(panel => panel.flex), true, `${dock.id} has a flex panel`);
  }

  const threeFlex = WS.normalize(manifest({
    layout: { docks: [
      { id: 'left', flex: true, panels: [{ id: 'assets' }] },
      { id: 'center', flex: true, panels: [{ id: 'viewer' }] },
      { id: 'right', flex: true, panels: [{ id: 'inspector' }] },
    ] },
  }));
  // Phase 3a decides whether the contract should enforce a single flex dock.
  assert.equal(threeFlex.layout.docks.filter(dock => dock.flex).length, 3,
    'normalization does not clear extra authored flex docks');
});

it('retired and duplicate panel ids are removed from visible docks', () => {
  const WS = workspaceModel();
  const workspace = WS.normalize(manifest({
    layout: { docks: [
      { id: 'left', panels: ['chat', 'layers', 'library', 'assets', 'assets'] },
      { id: 'center', panels: ['viewer', 'assets', 'timeline'] },
    ] },
  }));
  const ids = workspace.layout.docks.flatMap(dock => dock.panels.map(panel => panel.id));

  assert.equal(ids.includes('chat'), false);
  assert.equal(ids.includes('layers'), false);
  assert.equal(ids.includes('library'), false);
  assert.equal(ids.filter(id => id === 'assets').length, 1);
});

it('workspace numeric dimensions are clamped to their contract bounds', () => {
  const WS = workspaceModel();
  const low = WS.normalize(manifest({
    chrome: { timeline: { rowHeight: -100 } },
    custom: [{ id: 'small-controls', size: -100 }],
    layout: { docks: [{ id: 'center', size: -100, panels: [
      { id: 'viewer', size: -100 }, { id: 'small-controls' },
    ] }] },
  }));
  const high = WS.normalize(manifest({
    chrome: { timeline: { rowHeight: 10_000 } },
    custom: [{ id: 'large-controls', size: 10_000 }],
    layout: { docks: [{ id: 'center', size: 10_000, panels: [
      { id: 'viewer', size: 10_000 }, { id: 'large-controls' },
    ] }] },
  }));

  assert.equal(low.layout.docks[0].panels[0].size, 56);
  assert.equal(high.layout.docks[0].panels[0].size, 1600);
  assert.equal(low.layout.docks[0].size, 200);
  assert.equal(high.layout.docks[0].size, 760);
  assert.equal(low.custom[0].size, 72);
  assert.equal(high.custom[0].size, 1200);
  assert.equal(low.chrome.timeline.rowHeight, 22);
  assert.equal(high.chrome.timeline.rowHeight, 48);
});

it('custom tool state keeps at most 64 primitive-valued keys', () => {
  const WS = workspaceModel();
  const state = Object.fromEntries(Array.from({ length: 70 }, (_, index) => [
    `key${index}`,
    [null, true, index, `value-${index}`][index % 4],
  ]));
  state.nested = { rejected: true };
  state.list = ['rejected'];
  state.bad$key = 'rejected';

  const workspace = WS.normalize(manifest({
    custom: [{ id: 'tool', state }],
  }));
  const clean = workspace.custom[0].state;

  assert.equal(Object.keys(clean).length, 64);
  assert.equal(clean.key0, null);
  assert.equal('key69' in clean, false);
  assert.equal('nested' in clean, false);
  assert.equal('list' in clean, false);
  assert.equal('bad$key' in clean, false);
  assert.equal(Object.values(clean).every(value => value === null || ['string', 'number', 'boolean'].includes(typeof value)), true);
});

it('controls without a valid type infer select, toggle, color, text, and slider', () => {
  const WS = workspaceModel();
  const workspace = WS.normalize(manifest({
    custom: [{ id: 'inferred-controls', controls: [
      { label: 'Mode', options: ['A', 'B'] },
      { label: 'Enabled', default: true },
      { label: 'Tint', value: '#abcdef' },
      { label: 'Caption', def: 'Hello' },
      { label: 'Amount', value: 25, min: 0, max: 100 },
    ] }],
  }));

  assert.deepEqual([...workspace.custom[0].controls.map(control => control.type)], [
    'select', 'toggle', 'color', 'text', 'slider',
  ]);
});

