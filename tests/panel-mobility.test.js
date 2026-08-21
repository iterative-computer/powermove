const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const layout = fs.readFileSync(path.join(root, 'js/ui/layout.js'), 'utf8');
const css = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');
const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');

test('headless preview and timeline panels expose a dedicated move handle', () => {
  assert.match(layout, /button\.panel-move-handle/);
  assert.match(layout, /Move \$\{def\.title\} panel/);
  assert.match(layout, /moveHandle\.addEventListener\('pointerdown', beginMove\)/);
  assert.match(css, /\.panel-move-handle\s*\{/);
  assert.match(css, /cursor:grab/);
  assert.match(css, /\.panel-move-handle\.inline/);
  assert.match(css, /\.panel-move-handle\.inline\{[^}]*width:16px[^}]*border:0[^}]*background:transparent[^}]*box-shadow:none/s);
  assert.match(layout, /slot\.insertBefore\(moveHandle, slot\.firstChild\)/);
});

test('persisted canvas panels resolve their current dock before moving again', () => {
  assert.match(layout, /findPanel\(L\.ws, spec\.id\)/);
  assert.match(layout, /inst\.dock = dock/);
  assert.match(layout, /startPanelDrag\(e, current\.spec, current\.dock, el\)/);
});

test('panel dragging uses one compact destination label without workspace lines', () => {
  assert.match(layout, /span\.panel-ghost-destination/);
  assert.match(layout, /document\.body\.classList\.add\('panel-dragging'\)/);
  assert.match(layout, /document\.body\.classList\.remove\('panel-dragging'\)/);
  assert.doesNotMatch(layout, /classList\.add\(before \? 'drop-before' : 'drop-after'\)/);
  assert.doesNotMatch(layout, /dockEl\.classList\.add\('drop-into'\)/);
  assert.doesNotMatch(css, /\.dock\.drop-into\s*\{/);
  assert.doesNotMatch(css, /\.panel\.drop-(?:before|after)\s*\{/);
  assert.match(css, /\.panel-dragging \.splitter::after\{background:transparent\}/);
  assert.match(css, /\.panel-ghost\s*\{[^}]*height:32px/s);
});

test('timeline section resize never exposes an opaque cleared canvas', () => {
  assert.doesNotMatch(timeline, /getContext\('2d',\s*\{\s*alpha:\s*false\s*\}\)/);
  assert.match(timeline, /const changed = T\.cv\.width !== width \|\| T\.cv\.height !== height/);
  assert.match(timeline, /if \(changed\) draw\(\)/);
});
