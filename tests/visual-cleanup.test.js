const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');
const workspace = fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8');
const tokens = fs.readFileSync(path.join(root, 'css/tokens.css'), 'utf8');
const appCss = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');

test('major section borders have a dedicated perceptual weight in both themes', () => {
  const light = tokens.slice(tokens.indexOf(':root{'), tokens.indexOf(':root[data-density'));
  const dark = tokens.slice(tokens.indexOf(':root[data-theme="dark"]{', tokens.indexOf('/* ── dark theme')));
  assert.match(light, /--section-line:rgba\(15,15,20,\.13\)/);
  assert.match(dark, /--section-line:rgba\(255,255,255,\.08\)/);
  assert.match(appCss, /\.panel\s*\{[^}]*border:1px solid var\(--section-line\)/s);
  assert.match(appCss, /\.panel > header\s*\{[^}]*border-bottom:1px solid var\(--section-line\)/s);
  assert.match(appCss, /#tl-head\s*\{[^}]*border-bottom:1px solid var\(--section-line\)/s);
});

test('timeline ruler renders composition markers as compact, labeled diamonds', () => {
  const ruler = timeline.slice(timeline.indexOf('function drawRuler'), timeline.indexOf('function fmtRuler'));
  /* markers must be visible and clickable — created-but-invisible state is a trap */
  assert.match(ruler, /markers/);
  assert.match(ruler, /closePath\(\)\.fill|c\.fill\(\)/);
  assert.match(ruler, /m\.name/, 'marker names render when there is room');
  assert.doesNotMatch(ruler, /lineTo\(x \+ 5/);
  assert.match(timeline, /PM\.proj\.markers\.map\(m => m\.t\)/, 'marker timing data remains editable');
});

test('Design workspace uses the timeline instead of a redundant Layers panel', () => {
  const design = workspace.slice(workspace.indexOf("id: 'design'"), workspace.indexOf("id: 'gradient'"));
  assert.doesNotMatch(design, /p\('layers'/);
  /* no assistant is docked; the generative library ships in Design */
  assert.doesNotMatch(design, /p\('chat'/);
  assert.match(design, /p\('assets', \{ size: 150 \}\), p\('library', \{ size: 220 \}\), p\('fxbrowser', \{ flex: true \}\)/);
});

test('Gradient is a clean shared preset instead of app-only saved state', () => {
  const gradient = workspace.slice(workspace.indexOf("id: 'gradient'"), workspace.indexOf("id: 'animate'"));
  assert.match(gradient, /name: 'Gradient', builtin: true/);
  assert.match(gradient, /dock\('center', \[p\('viewer', \{ flex: true \}\), p\('timeline', \{ size: 300 \}\)\]\)/);
  assert.doesNotMatch(gradient, /p\('layers'/);
  assert.match(gradient, /label: 'Gradient End'.*def: '#34144F'/);
  assert.match(gradient, /label: 'Gradient Midpoint'.*def: 50/);
});
