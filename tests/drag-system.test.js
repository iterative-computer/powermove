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
  };
  const context = vm.createContext({ window: { PM }, console, innerWidth: 1440, innerHeight: 900 });
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

/* ── util.js harness for PM.drag semantics ──────────────── */
function dragModel() {
  const listeners = {};
  const windowObj = {
    addEventListener: (t, fn) => { (listeners[t] = listeners[t] || []).push(fn); },
    removeEventListener: (t, fn) => {
      const l = listeners[t] || [];
      const i = l.indexOf(fn);
      if (i >= 0) l.splice(i, 1);
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
  return { PM: windowObj.PM, listeners };
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

test('panel drag wiring: Esc cancels and drops resolve before mutation', () => {
  const layout = fs.readFileSync(path.join(root, 'js/ui/layout.js'), 'utf8');
  const util = fs.readFileSync(path.join(root, 'js/core/util.js'), 'utf8');
  assert.match(layout, /L\.resolveDropIndex/, 'drop resolution is a named, testable step');
  assert.match(layout, /removePanel\(w, spec\.id\);\s*\n\s*insertPanel\(w, spec, target\.dockId, index\)/s,
    'insertion uses the resolved index, not the raw pre-removal one');
  assert.match(layout, /ev\.key !== 'Escape'/, 'Esc cancels an in-flight drag');
  assert.match(layout, /drag\.cancel\(\)/);
  assert.match(layout, /'Not a drop zone'/, 'ghost reports invalid targets');
  assert.match(util, /pointercancel/, 'interrupted gestures end drags cleanly');
});
