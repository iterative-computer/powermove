const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = fs.readFileSync(path.join(root, 'js/ui/viewer.js'), 'utf8');

function viewerModel() {
  const PM = {
    h() {}, clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    registerPanel() {}, bus: { on() {} }, GL: {},
  };
  const window = { PM };
  vm.runInContext(source, vm.createContext({
    window, console, addEventListener() {}, requestAnimationFrame() {}, ResizeObserver: class {},
    devicePixelRatio: 1, innerWidth: 1200, innerHeight: 800,
  }));
  return PM;
}

test('viewport snapping chooses the closest edge or center within the screen-space threshold', () => {
  const V = viewerModel().Viewer;
  const snap = V.snapAxis([93, 103, 113], [{ value: 0 }, { value: 100 }, { value: 200 }], 8);
  assert.deepEqual({ ...snap }, { delta: -3, value: 100, distance: 3 });
  assert.equal(V.snapAxis([30, 40, 50], [100], 8), null, 'distant marks remain unsnapped');
});

test('alignment snapping resolves horizontal and vertical guides independently', () => {
  const V = viewerModel().Viewer;
  const snap = V.alignmentSnap(
    { x0: 193, cx: 203, x1: 213, y0: 42, cy: 52, y1: 62 },
    { x: [0, 200, 400], y: [0, 50, 300] }, 6,
  );
  assert.equal(snap.dx, -3);
  assert.equal(snap.dy, -2);
  assert.equal(snap.x.value, 200);
  assert.equal(snap.y.value, 50);

  const constrained = V.alignmentSnap(
    { x0: 193, cx: 203, x1: 213, y0: 42, cy: 52, y1: 62 },
    { x: [200], y: [50] }, 6, { x: true, y: false },
  );
  assert.equal(constrained.dx, -3);
  assert.equal(constrained.dy, 0, 'Shift-constrained motion cannot drift on the locked axis');
});

test('rotated layer bounds are measured in composition space before snapping', () => {
  const PM = viewerModel();
  PM.GL.bounds = () => ({ x0: -10, y0: -20, x1: 10, y1: 20 });
  PM.worldMatrix = () => [0, 1, -1, 0, 100, 200];
  const bounds = PM.Viewer.worldBounds({}, 0);
  assert.deepEqual({ ...bounds }, { x0: 80, x1: 120, y0: 190, y1: 210, cx: 100, cy: 200 });
});

test('composition drag wiring shares the magnet setting, paints guides, and cancels safely', () => {
  assert.match(source, /if \(PM\.snap\) snap = alignmentSnap/);
  assert.match(source, /8 \/ Math\.max\(\.02, V\.shown\)/, 'tolerance remains constant in screen pixels at every zoom');
  assert.match(source, /drawSnapGuides\(c, S, p\)/);
  assert.match(source, /hasSelectedAncestor\(L, selectedIds\)/, 'children moving with a selected parent are not snap targets');
  assert.match(source, /soloOn && !L\.solo/, 'layers hidden by Solo cannot attract the moving selection');
  assert.match(source, /cancel: \(\) => \{ clearGuides\(\); PM\.Edit\.cancel\(\); PM\.Inspector\.refresh\(\); \}/);
});
