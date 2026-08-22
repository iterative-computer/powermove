const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');
const workspace = fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8');
const tokens = fs.readFileSync(path.join(root, 'css/tokens.css'), 'utf8');
const appCss = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');
const app = fs.readFileSync(path.join(root, 'js/app.js'), 'utf8');

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

test('Design workspace uses the timeline and has no retired Generative panel', () => {
  const design = workspace.slice(workspace.indexOf("id: 'design'"), workspace.indexOf("id: 'gradient'"));
  assert.doesNotMatch(design, /p\('layers'/);
  /* no assistant or retired Generative surface is docked */
  assert.doesNotMatch(design, /p\('chat'/);
  assert.doesNotMatch(design, /p\('library'/);
  assert.match(design, /p\('assets', \{ size: 190 \}\), p\('fxbrowser', \{ flex: true \}\)/);
});

test('Gradient is a clean shared preset instead of app-only saved state', () => {
  const gradient = workspace.slice(workspace.indexOf("id: 'gradient'"), workspace.indexOf("id: 'animate'"));
  assert.match(gradient, /name: 'Gradient', builtin: true/);
  assert.match(gradient, /dock\('center', \[p\('viewer', \{ flex: true \}\), p\('timeline', \{ size: 300 \}\)\]\)/);
  assert.doesNotMatch(gradient, /p\('layers'/);
  assert.match(gradient, /label: 'Gradient End'.*def: '#34144F'/);
  assert.match(gradient, /label: 'Gradient Midpoint'.*def: 50/);
});

test('non-editable chrome cannot be selected while text editors remain selectable', () => {
  assert.match(appCss, /body \*\{-webkit-user-select:none;user-select:none\}/);
  assert.match(appCss, /textarea,[\s\S]*\[contenteditable\][\s\S]*-webkit-user-select:text;user-select:text/);
});

test('the Composition surface loses only its drop shadow and keeps its edge geometry', () => {
  const stage = appCss.match(/#stage-inner\{([^}]*)\}/)?.[1] || '';
  assert.match(stage, /border-radius:var\(--r-md\)/);
  assert.match(stage, /overflow:hidden/);
  assert.match(stage, /box-shadow:none/);
  assert.match(stage, /outline:1px solid/, 'the preview edge remains legible');
  assert.match(appCss, /#library-screen\{[^}]*box-shadow:var\(--shadow-float\)/s, 'unrelated elevation remains intact');
});

test('one Library control replaces duplicate global commands without hiding unique actions', () => {
  const titlebar = app.slice(app.indexOf('right.append('), app.indexOf("PM.bus.on('workspaces', paintTabs)"));
  assert.match(titlebar, /Library · Sections and Workspaces/);
  assert.doesNotMatch(titlebar, /button\('plus', 'New layer'/);
  assert.doesNotMatch(titlebar, /button\('wand', 'New shader layer'/);
  assert.doesNotMatch(titlebar, /button\('export', 'Export'/);
  assert.doesNotMatch(titlebar, /button\('gear', 'Workspace definition'/);
  assert.match(fs.readFileSync(path.join(root, 'js/ui/library.js'), 'utf8'), /Edit validated definition….*PM\.WS\.editJSON/s,
    'validated workspace definition editing remains reachable from Library');
  assert.match(fs.readFileSync(path.join(root, 'js/ui/toolbar.js'), 'utf8'), /PM\.cmd\('newShader'\)/,
    'shader creation remains reachable from the canonical tool strip');
  assert.match(fs.readFileSync(path.join(root, 'js/ui/viewer.js'), 'utf8'), /PM\.Export\.dialog\(\)/,
    'export remains reachable from the composition surface');
  assert.doesNotMatch(fs.readFileSync(path.join(root, 'js/ui/toolbar.js'), 'utf8'), /title: 'Snapping \(S\)'/,
    'the global snapping duplicate is removed while the timeline owns its control');
});
