const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8');

test('ordinary interface text does not opt into the monospace font', () => {
  const css = read('css/app.css');
  const timeline = read('js/ui/timeline.js');
  const panels = read('js/ui/panels.js');
  const inspector = read('js/ui/inspector.js');
  const exporter = read('js/core/exporter.js');

  for (const selector of [
    '.pv-tc', '.panel > header .sub', '.generated-tool-preview code',
    '.generated-curve-head code', '.lyr .idx', '.asset-count',
    '.asset-list-count', '.asset-copy small', '.codebar', '.pitem .cat',
    '#status', '.spatial-frame-grid figcaption', '.ps-navbtn .count',
    '.ps-title span', '.library-navbtn .count', '.library-count',
    '.agent-attachment-type',
  ]) {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    assert.doesNotMatch(css, new RegExp(`${escaped}\\{[^}]*(?:--f-mono|monospace)`),
      `${selector} should inherit the regular interface font`);
  }

  assert.doesNotMatch(timeline, /fmono|Geist Mono|ui-monospace|Menlo|monospace/,
    'canvas-drawn timeline labels should use the regular UI font');
  for (const source of [panels, inspector, exporter]) {
    assert.doesNotMatch(source, /fontFamily:\s*['"]var\(--f-mono\)['"]/,
      'non-control interface copy should not set monospace inline');
  }
});

test('monospace remains available for precision and editable controls', () => {
  const css = read('css/app.css');
  for (const selector of ['#tl-time', '#tl-edit', '.num', '.color-hex', '.sel', '.code', '.pitem .kb']) {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    assert.match(css, new RegExp(`${escaped}\\{[^}]*--f-mono`),
      `${selector} should retain monospace as a control`);
  }
});
