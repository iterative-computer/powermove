const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');

/* ── layout.js harness (resolveDropIndex is pure) ───────── */
function layoutModel() {
  const PM = {
    h: () => ({ style: {}, classList: { add() {}, remove() {} }, appendChild() {}, append() {} }),
    $: () => null,
    $$: () => [],
    icon: () => '',
    clamp: (v, a, b) => Math.max(a, Math.min(b, v)),
    bus: { on() {}, emit() {} },
    PANELS: {},
    panelInst: {},
    toast() {},
  };
  const context = vm.createContext({ window: { PM }, console, innerWidth: 1440, innerHeight: 900, clearInterval });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/ui/layout.js'), 'utf8'), context);
  return PM;
}

test('drop index accounts for removing the dragged panel first', () => {
  const PM = layoutModel();
  const R = PM.Layout.resolveDropIndex;
  /* dock [A, B, C]: dragging A "after B" must land between B and C */
  assert.equal(R('center', 0, 'center', 2), 1, 'downward move shifts by one');
  /* dock [A, B, C]: dragging C "before B" lands at index 1 */
  assert.equal(R('center', 2, 'center', 1), 1, 'upward move keeps its slot');
  /* dropping back onto the original position is a no-op in both half-planes */
  assert.equal(R('center', 1, 'center', 1), null, 'drop on own upper half is a no-op');
  assert.equal(R('center', 1, 'center', 2), null, 'drop on own lower half is a no-op');
  /* cross-dock drops pass the index through untouched */
  assert.equal(R('left', 0, 'right', 3), 3);
  assert.equal(R('right', 4, 'left', 0), 0);
});

test('malformed drop indices resolve to null rather than corrupting the dock', () => {
  const PM = layoutModel();
  const R = PM.Layout.resolveDropIndex;
  assert.equal(R('center', 0, 'center', undefined), null);
  assert.equal(R('center', 0, 'center', NaN), null);
  assert.equal(R('center', 0, 'right', NaN), null, 'cross-dock also rejects garbage');
});

test('dock hit testing makes top, bottom, gaps, and left/right placement generous', () => {
  const PM = layoutModel();
  const hit = PM.Layout.hitTestDockPlacement;
  const rect = (left, top, right, bottom) => ({ left, top, right, bottom, width: right - left, height: bottom - top });
  const docks = [
    { id: 'left', rect: rect(20, 80, 260, 760), panels: [
      { rect: rect(20, 80, 260, 280) }, { rect: rect(20, 292, 260, 520) }, { rect: rect(20, 532, 260, 760) },
    ] },
    { id: 'center', rect: rect(272, 80, 1020, 760), panels: [
      { rect: rect(272, 80, 1020, 440) }, { rect: rect(272, 452, 1020, 760) },
    ] },
    { id: 'right', rect: rect(1032, 80, 1320, 760), panels: [{ rect: rect(1032, 80, 1320, 760) }] },
  ];
  assert.deepEqual({ ...hit(docks, 120, 84) }, { dockId: 'left', index: 0 }, 'top zone inserts above');
  assert.deepEqual({ ...hit(docks, 120, 755) }, { dockId: 'left', index: 3 }, 'bottom zone inserts below');
  assert.deepEqual({ ...hit(docks, 120, 286) }, { dockId: 'left', index: 1 }, 'gap between panels has an exact insertion slot');
  assert.deepEqual({ ...hit(docks, 30, 400) }, { dockId: 'left', index: 1 }, 'left docking remains easy');
  assert.deepEqual({ ...hit(docks, 1318, 400) }, { dockId: 'right', index: 0 }, 'right docking remains easy');
  assert.deepEqual({ ...hit(docks, 266, 400) }, { dockId: 'left', index: 1 }, 'near-edge tolerance keeps a valid target');
  assert.equal(hit(docks, 700, 20), null, 'far outside all docks cancels instead of losing a panel');
});

test('missing side docks get full-height edge targets and are created in stable order', () => {
  const PM = layoutModel();
  const rect = (left, top, right, bottom) => ({ left, top, right, bottom, width: right - left, height: bottom - top });
  const targets = PM.Layout.buildDockDropTargets([
    { id: 'center', rect: rect(0, 44, 1400, 860), panels: [] },
  ], rect(0, 44, 1400, 860));
  assert.deepEqual({ ...PM.Layout.hitTestDockPlacement(targets, 18, 400) }, { dockId: 'left', index: 0 });
  assert.deepEqual({ ...PM.Layout.hitTestDockPlacement(targets, 1382, 400) }, { dockId: 'right', index: 0 });

  const workspace = { layout: { docks: [{ id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }, { id: 'assets', size: 240 }] }] }, hiddenPanels: [] };
  assert.equal(PM.Layout.movePanel(workspace, 'assets', 'left'), true);
  assert.deepEqual([...workspace.layout.docks.map(dock => dock.id)], ['left', 'center']);
  assert.equal(workspace.layout.docks[0].panels[0].size, 240, 'panel height metadata survives side docking');
  assert.equal(PM.Layout.movePanel(workspace, 'assets', 'right'), true);
  assert.deepEqual([...workspace.layout.docks.map(dock => dock.id)], ['left', 'center', 'right']);
  assert.equal(workspace.layout.docks[2].panels[0].size, 240);
});

test('left and right docking never leave an empty reserved track', () => {
  const PM = layoutModel();
  const workspace = { layout: { docks: [{ id: 'center', flex: true, panels: [
    { id: 'viewer', flex: true }, { id: 'assets', size: 180 },
  ] }] }, hiddenPanels: [] };
  PM.Layout.movePanel(workspace, 'assets', 'left');
  assert.deepEqual([...PM.Layout.visibleDockPlan(workspace, () => false).map(item => item.dock.id)], ['left', 'center']);
  const left = workspace.layout.docks.find(dock => dock.id === 'left');
  assert.equal(left.panels[0].flex, true, 'the side panel fills its column instead of leaving blank space below');
  assert.equal(left.panels[0].size, 180, 'its prior vertical height remains available for later stacking');

  PM.Layout.movePanel(workspace, 'assets', 'right');
  assert.deepEqual([...PM.Layout.visibleDockPlan(workspace, () => false).map(item => item.dock.id)], ['center', 'right'],
    'the now-empty left dock produces no DOM track or splitter');
  const right = workspace.layout.docks.find(dock => dock.id === 'right');
  assert.equal(right.panels[0].flex, true);
  assert.equal(right.panels[0].size, 180);
});

test('context-menu vertical movement reorders without changing panel metadata', () => {
  const PM = layoutModel();
  const workspace = { layout: { docks: [{ id: 'center', panels: [
    { id: 'viewer', flex: true }, { id: 'assets', size: 190 }, { id: 'inspector', size: 260 },
  ] }] } };
  assert.equal(PM.Layout.movePanelBy(workspace, 'inspector', -1), true);
  assert.deepEqual([...workspace.layout.docks[0].panels.map(p => p.id)], ['viewer', 'inspector', 'assets']);
  assert.equal(workspace.layout.docks[0].panels[1].size, 260);
  assert.equal(PM.Layout.movePanelBy(workspace, 'viewer', -1), false, 'edge commands are safe no-ops');
});

test('vertical panel resize geometry clamps both panels and is sampling-rate independent', () => {
  const PM = layoutModel();
  const clamp = PM.Layout.clampPanelHeight;
  assert.equal(clamp(220, 40, 1, 600, 88, 100, 8), 260);
  assert.equal(clamp(220, -500, 1, 600, 88, 100, 8), 88);
  assert.equal(clamp(220, 800, 1, 600, 88, 100, 8), 492);
  assert.equal(clamp(300, 40, -1, 600, 88, 88, 8), 260, 'lower fixed panel uses the inverse direction');
});

test('panel hiding is recoverable, preserves its slot, and handles the last panel in a dock', () => {
  const PM = layoutModel();
  const workspace = { layout: { docks: [
    { id: 'left', size: 250, panels: [{ id: 'assets', size: 180 }] },
    { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
  ] }, hiddenPanels: [] };
  assert.equal(PM.Layout.hidePanel(workspace, 'assets'), true);
  assert.equal(workspace.layout.docks[0].panels.length, 0, 'the empty dock remains a valid recoverable slot');
  assert.equal(workspace.hiddenPanels[0].dockId, 'left');
  assert.equal(workspace.hiddenPanels[0].spec.size, 180, 'panel-specific layout metadata is preserved');
  assert.equal(PM.Layout.restorePanel(workspace, 'assets'), true);
  assert.deepEqual({ ...workspace.layout.docks[0].panels[0] }, { id: 'assets', size: 180, flex: true });
  assert.equal(workspace.hiddenPanels.length, 0);
});

test('Composition cannot be hidden and invalid restore requests are safe no-ops', () => {
  const PM = layoutModel();
  const workspace = { layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] }, hiddenPanels: [] };
  assert.equal(PM.Layout.hidePanel(workspace, 'viewer'), false);
  assert.equal(PM.Layout.hidePanel(workspace, 'missing'), false);
  assert.equal(PM.Layout.restorePanel(workspace, 'missing'), false);
  assert.equal(workspace.layout.docks[0].panels[0].id, 'viewer');
});

test('detached-panel lifecycle has one owner and reapplies the source layout on close or redock', () => {
  const PM = layoutModel();
  let applies = 0, closed = 0;
  const classes = new Set(['popped']);
  PM.PANELS.assets = { id: 'assets', title: 'Assets' };
  PM.panelInst.assets = { el: { classList: { remove: value => classes.delete(value) } } };
  PM.Layout.ws = { id: 'workspace' };
  PM.Layout.apply = () => { applies++; };
  PM.Popout.wins.assets = { window: { closed: true }, timer: 0 };
  assert.equal(PM.Popout.reclaim('assets'), true);
  assert.equal(applies, 1);
  assert.equal(classes.has('popped'), false);
  PM.Popout.wins.assets = { window: { closed: false, close: () => { closed++; } }, timer: 0 };
  assert.equal(PM.Popout.dock('assets'), true);
  assert.equal(applies, 2);
  assert.equal(closed, 1);
  assert.equal(PM.Popout.reclaim('assets'), false, 'a later close callback cannot restore twice');
  assert.equal(PM.Popout.open('unknown'), undefined, 'invalid panels do not create windows');
  PM.PANELS.viewer = { id: 'viewer', title: 'Composition' };
  assert.equal(PM.Popout.open('viewer'), undefined, 'unsupported canvas panels remain safely docked');
});

test('vertical resize is isolated from panel reorder and supports exact cancellation', () => {
  const layout = fs.readFileSync(path.join(root, 'js/ui/layout.js'), 'utf8');
  assert.match(layout, /if \(e\.button !== 0\) return;\s*e\.stopPropagation\(\);/s);
  assert.match(layout, /if \(!changed && Math\.abs\(dy\) < 2\) return/);
  assert.match(layout, /const saved = \{ size: spec\.size, flex: spec\.flex \}/);
  assert.match(layout, /cancel: \(\) => \{[\s\S]*otherSaved[\s\S]*applyPanelSize\(node/s);
  assert.match(layout, /PM\.WS\.save\(\)/, 'committed sizes persist in the workspace manifest');
});

/* ── util.js harness for PM.drag semantics ──────────────── */
function dragModel() {
  const listeners = {};
  const listenerOptions = {};
  const windowObj = {
    addEventListener: (t, fn, options) => {
      (listeners[t] = listeners[t] || []).push(fn);
      (listenerOptions[t] = listenerOptions[t] || []).push(options);
    },
    removeEventListener: (t, fn, options) => {
      const l = listeners[t] || [];
      const i = l.indexOf(fn);
      if (i >= 0) {
        l.splice(i, 1);
        (listenerOptions[t] || []).splice(i, 1);
      }
    },
    style: {},
  };
  const PM = {};
  const context = vm.createContext({
    window: windowObj,
    document: { body: { style: {} } },
    console,
    addEventListener: () => {},
    removeEventListener: () => {},
    requestAnimationFrame: (fn) => 0,
    setTimeout,
  });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/util.js'), 'utf8'), context);
  return { PM: windowObj.PM, listeners, listenerOptions };
}

function fire(listeners, type, ev = { clientX: 0, clientY: 0 }) {
  (listeners[type] || []).forEach(fn => fn(ev));
}

test('PM.drag routes pointerup to up and ignores later events', () => {
  const { PM, listeners } = dragModel();
  let ups = 0, moves = 0;
  const e = { clientX: 5, clientY: 5, preventDefault() {} };
  PM.drag(e, {
    move: () => moves++,
    up: () => ups++,
  });
  fire(listeners, 'pointermove', { clientX: 20, clientY: 20 });
  fire(listeners, 'pointerup', { clientX: 30, clientY: 30 });
  fire(listeners, 'pointermove', { clientX: 40, clientY: 40 });
  fire(listeners, 'pointerup', { clientX: 50, clientY: 50 });
  assert.equal(moves, 1);
  assert.equal(ups, 1, 'drag ends exactly once');
});

test('PM.drag cancel() ends the drag without calling up', () => {
  const { PM, listeners } = dragModel();
  let ups = 0, cancels = 0;
  const e = { clientX: 5, clientY: 5, preventDefault() {} };
  const ctl = PM.drag(e, { up: () => ups++, cancel: () => cancels++ });
  ctl.cancel();
  ctl.cancel(); // idempotent
  fire(listeners, 'pointerup', { clientX: 30, clientY: 30 });
  assert.equal(cancels, 1);
  assert.equal(ups, 0, 'cancelled drags never commit');
});

test('a native pointercancel aborts the drag instead of wedging it', () => {
  const { PM, listeners } = dragModel();
  let ups = 0, cancels = 0;
  const e = { clientX: 5, clientY: 5, preventDefault() {} };
  PM.drag(e, { up: () => ups++, cancel: () => cancels++ });
  fire(listeners, 'pointercancel', {});
  fire(listeners, 'pointerup', { clientX: 30, clientY: 30 });
  assert.equal(cancels, 1);
  assert.equal(ups, 0);
});

test('PM.drag captures the originating pointer for WKWebView canvas gestures', () => {
  const { PM, listeners, listenerOptions } = dragModel();
  const targetListeners = {};
  const calls = [];
  const target = {
    setPointerCapture: id => calls.push(['capture', id]),
    hasPointerCapture: id => (calls.push(['has', id]), true),
    releasePointerCapture: id => calls.push(['release', id]),
    addEventListener: (type, fn) => { targetListeners[type] = fn; },
    removeEventListener: type => { delete targetListeners[type]; },
  };
  let cancels = 0;
  PM.drag({ clientX: 5, clientY: 5, pointerId: 17, currentTarget: target, preventDefault() {} }, {
    cancel: () => cancels++,
  });
  assert.deepEqual(calls[0], ['capture', 17]);
  assert.equal(listenerOptions.pointermove[0], true, 'movement is observed before WebKit can stop bubbling it');
  targetListeners.lostpointercapture();
  assert.equal(cancels, 1);
  assert.deepEqual(calls.at(-1), ['release', 17]);
  assert.equal((listeners.pointermove || []).length, 0, 'capture loss removes global drag listeners');
});

test('every transaction-backed editor drag cancels cleanly when the pointer is interrupted', () => {
  const controls = fs.readFileSync(path.join(root, 'js/ui/controls.js'), 'utf8');
  const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');
  const viewer = fs.readFileSync(path.join(root, 'js/ui/viewer.js'), 'utf8');

  assert.match(controls, /PM\.drag\(e, \{[\s\S]*?cancel: \(\) => cancel\(opt\),[\s\S]*?\}\);/,
    'interrupted inspector scrubs release their source transaction');
  for (const name of ['workAreaDrag', 'slide', 'trim']) {
    const start = timeline.indexOf(`function ${name}`);
    const next = timeline.indexOf('\nfunction ', start + 1);
    const body = timeline.slice(start, next < 0 ? undefined : next);
    assert.match(body, /cancel: \(\) => PM\.Edit\.cancel\(\)/, `${name} releases its source transaction`);
  }
  const reorderStart = timeline.indexOf('function gutterDown');
  const reorderEnd = timeline.indexOf('\nfunction slide', reorderStart);
  assert.match(timeline.slice(reorderStart, reorderEnd), /cancel: \(\) => \{ if \(done\) PM\.Edit\.cancel\(\); \}/,
    'interrupted layer reordering releases a transaction only when one began');
  const transformStart = viewer.indexOf('function startTransform');
  assert.match(viewer.slice(transformStart), /cancel: \(\) => \{ PM\.Edit\.cancel\(\); PM\.Inspector\.refresh\(\); \}/,
    'interrupted canvas transforms release the shared edit transaction');
});

test('panel drag wiring: Esc cancels and drops resolve before mutation', () => {
  const layout = fs.readFileSync(path.join(root, 'js/ui/layout.js'), 'utf8');
  const util = fs.readFileSync(path.join(root, 'js/core/util.js'), 'utf8');
  assert.match(layout, /L\.resolveDropIndex/, 'drop resolution is a named, testable step');
  assert.match(layout, /removePanel\(w, spec\.id\);\s*\n\s*insertPanel\(w, spec, target\.dockId, index\)/s,
    'insertion uses the resolved index, not the raw pre-removal one');
  assert.match(layout, /ev\.key !== 'Escape'/, 'Esc cancels an in-flight drag');
  assert.match(layout, /drag\.cancel\(\)/);
  assert.match(layout, /'Not a drop zone'/, 'ghost reports invalid targets');
  assert.match(layout, /L\.hitTestDockPlacement/, 'drop selection uses the tested geometry seam');
  assert.match(util, /pointercancel/, 'interrupted gestures end drags cleanly');
});
