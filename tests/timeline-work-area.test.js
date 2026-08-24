const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const root = path.join(__dirname, '..');
const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');

function workAreaMath() {
  const start = timeline.indexOf('const WorkArea = PM.TimelineWorkArea');
  const end = timeline.indexOf('\n\nPM.registerPanel', start);
  assert.ok(start >= 0 && end > start, 'timeline exposes its work-area math');
  const source = timeline.slice(start, end).replace('const WorkArea = PM.TimelineWorkArea', 'var WorkArea = PM.TimelineWorkArea');
  const context = {
    PM: {},
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
  };
  vm.runInNewContext(source, context);
  return context.PM.TimelineWorkArea;
}

test('dragging the out marker past the composition end extends both ranges', () => {
  const math = workAreaMath();
  const patch = math.resize([0, 10], 1, 14.5, 10, 1 / 30);
  assert.equal(patch.duration, 14.5);
  assert.deepEqual([...patch.workArea], [0, 14.5]);
});

test('work-area markers cannot cross and remain at least one frame apart', () => {
  const math = workAreaMath();
  const left = math.resize([2, 8], 0, 20, 10, 1 / 30);
  const right = math.resize([2, 8], 1, 0, 10, 1 / 30);
  assert.ok(Math.abs(left.workArea[0] - (8 - 1 / 30)) < 1e-9);
  assert.ok(Math.abs(right.workArea[1] - (2 + 1 / 30)) < 1e-9);
});

test('moving the center bar preserves its length and stops at composition edges', () => {
  const math = workAreaMath();
  assert.deepEqual([...math.move([2, 6], 3, 10, 1 / 30).workArea], [5, 9]);
  assert.deepEqual([...math.move([2, 6], 20, 10, 1 / 30).workArea], [6, 10]);
  assert.deepEqual([...math.move([2, 6], -20, 10, 1 / 30).workArea], [0, 4]);
});

test('work-area styling shades the outside and leaves the ruler free of a connecting line', () => {
  const tracks = timeline.slice(timeline.indexOf('function drawTracksBg'), timeline.indexOf('\nfunction niceStep'));
  assert.match(tracks, /fillRect\(T\.gut, T\.ruler, Math\.max\(0, x0 - T\.gut\), H - T\.ruler\)/,
    'the range before the in marker uses the shaded color');
  assert.match(tracks, /fillRect\(x1, T\.ruler, Math\.max\(0, W - x1\), H - T\.ruler\)/,
    'the range after the out marker uses the shaded color');
  assert.doesNotMatch(tracks, /fillRect\(t2x\(wa\[0\]\), T\.ruler/,
    'the active in-to-out range is no longer shaded');

  const ruler = timeline.slice(timeline.indexOf('function drawRuler'), timeline.indexOf('\nfunction drawWorkBracket'));
  assert.doesNotMatch(ruler, /fillRect\(x0, WORK_BAR\.top/,
    'no visual bar connects the work-area handles');
  assert.match(ruler, /drawWorkBracket\(c, x0, 0\); drawWorkBracket\(c, x1, 1\)/,
    'the in and out handles remain visible and editable');
});

test('timeline wires the complete AE-style work-area interaction', () => {
  assert.match(timeline, /workHit\.kind === 'handle' \? 'ew-resize' : 'grab'/);
  assert.match(timeline, /if \(hit\?\.kind === 'bar'\) return workAreaMove\(e\)/);
  assert.match(timeline, /if \(workHit\?\.kind === 'bar'\)[\s\S]*workArea: \[0, PM\.proj\.dur\]/,
    'double-clicking the bar restores the full composition');
  const start = timeline.indexOf('function workAreaMove');
  const end = timeline.indexOf('\nfunction ', start + 1);
  assert.match(timeline.slice(start, end), /cancel: \(\) => PM\.Edit\.cancel\(\)/,
    'an interrupted center-bar move rolls back its edit transaction');
});
