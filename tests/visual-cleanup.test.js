const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');
const workspace = fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8');

test('timeline ruler does not draw orange composition-marker diamonds', () => {
  const ruler = timeline.slice(timeline.indexOf('function drawRuler'), timeline.indexOf('function fmtRuler'));
  assert.doesNotMatch(ruler, /markers/);
  assert.doesNotMatch(ruler, /lineTo\(x \+ 5/);
  assert.match(timeline, /PM\.proj\.markers\.map\(m => m\.t\)/, 'marker timing data remains editable');
});

test('Design workspace uses the timeline instead of a redundant Layers panel', () => {
  const design = workspace.slice(workspace.indexOf("id: 'design'"), workspace.indexOf("id: 'animate'"));
  assert.doesNotMatch(design, /p\('layers'/);
  assert.match(design, /p\('assets', \{ size: 150 \}\), p\('chat', \{ flex: true \}\)/);
});

test('Gradient is a clean shared preset instead of app-only saved state', () => {
  const gradient = workspace.slice(workspace.indexOf("id: 'gradient'"), workspace.indexOf("id: 'animate'"));
  assert.match(gradient, /name: 'Gradient', builtin: true/);
  assert.match(gradient, /dock\('center', \[p\('viewer', \{ flex: true \}\), p\('timeline', \{ size: 300 \}\)\]\)/);
  assert.doesNotMatch(gradient, /p\('layers'/);
  assert.match(gradient, /label: 'Gradient End'.*def: '#34144F'/);
  assert.match(gradient, /label: 'Gradient Midpoint'.*def: 50/);
});
