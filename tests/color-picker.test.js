const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
function colors() {
  const PM = { clamp: (value, min, max) => Math.max(min, Math.min(max, value)) };
  const context = { window: { PM }, document: {}, console };
  vm.runInNewContext(fs.readFileSync(path.join(root, 'js/ui/controls.js'), 'utf8'), context);
  return PM.Color;
}

test('custom picker round-trips hex, RGB, and HSB channels', () => {
  const color = colors();
  for (const hex of ['#FF6B1A', '#3366CC', '#FFFFFF', '#09090A']) {
    const rgb = color.hexToRgb(hex);
    assert.equal(color.rgbToHex(rgb), hex);
    const roundTrip = color.rgbToHex(color.hsvToRgb(color.rgbToHsv(rgb)));
    const actual = color.hexToRgb(roundTrip);
    assert.ok(Math.max(...['r', 'g', 'b'].map(key => Math.abs(actual[key] - rgb[key]))) <= 2, `${hex} survives HSB rounding`);
  }
  assert.equal(color.normalizeHex('#f60'), '#FF6600');
  assert.equal(color.normalizeHex('not-a-color'), null);
});

test('custom picker clamps channel edits to their valid ranges', () => {
  const color = colors();
  assert.equal(color.rgbToHex({ r: 999, g: -10, b: 127.6 }), '#FF0080');
  assert.equal(JSON.stringify(color.hsvToRgb({ h: 0, s: 100, v: 100 })), JSON.stringify({ r: 255, g: 0, b: 0 }));
  assert.equal(JSON.stringify(color.hsvToRgb({ h: 120, s: 100, v: 100 })), JSON.stringify({ r: 0, g: 255, b: 0 }));
});
