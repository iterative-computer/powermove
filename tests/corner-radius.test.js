const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const css = fs.readFileSync(path.resolve(__dirname, '../css/app.css'), 'utf8');

test('toolbar is inline with the native titlebar instead of drawing a second shell', () => {
  assert.match(css, /#titlebar\s*\{[^}]*height:44px[^}]*padding:6px 10px 6px 78px/s);
  assert.match(css, /#toolbar-strip\s*\{[^}]*height:32px[^}]*background:var\(--bg-sunken\)[^}]*border:0[^}]*border-radius:10px/s);
  assert.doesNotMatch(css, /--toolbar-control-inset/);
});

test('all representative app chrome shares 60 percent corner smoothing without touching composition output', () => {
  assert.match(css, /:root\{--ui-corner-smoothing:superellipse\(1\.6\)\}/);
  assert.match(css, /:where\(button,input,textarea,select,\.panel,\.drop,\.modal,#toolbar-strip,#library-screen,\.library-card,\.library-thumb,\.spatial-compose,\.spatial-hint,\.spatial-outline,\.panel-ghost-card,\.panel-drop-preview,\.pop-bar,\.pop-mirror\)/);
  assert.match(css, /corner-shape:var\(--ui-corner-smoothing\)/);
  assert.doesNotMatch(css, /:where\([^)]*(?:canvas|svg|\.layer)/,
    'rendered project surfaces are excluded from UI corner smoothing');
  assert.match(css, /#stage-inner\{[^}]*border-radius:0[^}]*corner-shape:round/s,
    'the preview surface is the deliberate square-corner exception');
});
