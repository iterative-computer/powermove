const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const read = file => fs.readFileSync(path.join(root, file), 'utf8');

test('the shipped Geist families are real app assets and are copied into macOS builds', () => {
  for (const file of ['assets/fonts/Geist-Variable.ttf', 'assets/fonts/GeistMono-Variable.ttf', 'assets/fonts/OFL.txt']) {
    assert.ok(fs.statSync(path.join(root, file)).size > 1000, `${file} is present`);
  }
  const tokens = read('css/tokens.css');
  assert.match(tokens, /@font-face\{\s*font-family:"Geist"[\s\S]*Geist-Variable\.ttf/);
  assert.match(tokens, /font-weight:100 900/, 'the bundled face keeps variable weights functional');
  assert.match(read('scripts/build-macos-app.sh'), /cp -R "\$ROOT\/assets\/fonts\/\." "\$WEB\/assets\/fonts\/"/);
});

test('the native app publishes its actual installed font families to the searchable picker', () => {
  const native = read('native/main.swift');
  const controls = read('js/ui/controls.js');
  const inspector = read('js/ui/inspector.js');
  assert.match(native, /NSFontManager\.shared\.availableFontFamilies/);
  assert.match(native, /PM\.Fonts\.setSystemFamilies/);
  assert.match(controls, /PM\.fontField/);
  assert.match(controls, /placeholder: 'Search fonts'/);
  assert.match(inspector, /PM\.fontField\(get\('font'\)/);
});

test('text selection bounds hug measured glyphs instead of the padded render texture', () => {
  const context2d = {
    font: '', letterSpacing: '', textBaseline: '', textAlign: '', fillStyle: '',
    scale() {}, fillText() {},
    measureText(text) {
      const width = Math.max(1, text.length * 48);
      const left = this.textAlign === 'center' ? width / 2 : this.textAlign === 'right' ? width : 0;
      return {
        width,
        actualBoundingBoxLeft: left,
        actualBoundingBoxRight: width - left,
        actualBoundingBoxAscent: 78,
        actualBoundingBoxDescent: 18,
      };
    },
  };
  const document = {
    createElement(tag) {
      assert.equal(tag, 'canvas');
      return { width: 0, height: 0, getContext: () => context2d };
    },
  };
  const PM = { clamp: (value, min, max) => Math.max(min, Math.min(max, value)) };
  const sandbox = { window: { PM }, document, URL, Image: function Image() {}, setTimeout, console };
  vm.createContext(sandbox);
  vm.runInContext(read('js/gl/raster.js'), sandbox, { filename: 'raster.js' });

  const raster = PM.raster({
    type: 'text',
    d: { text: 'Powermove', font: 'Geist', weight: 650, size: 100, tracking: 0, leading: 1, color: '#fff', align: 'center', italic: false },
  });
  assert.ok(raster.h > 250, 'the padded render texture remains large enough for effects');
  assert.ok(raster.selection.h < raster.h * .5, 'interaction geometry excludes texture safety padding');
  assert.ok(raster.selection.w < raster.w, 'horizontal safety padding is excluded too');
  assert.ok(raster.selection.w > 350, 'the measured lettering remains fully selectable');
  assert.ok(Math.abs(raster.selection.x0 + raster.selection.x1) < 1,
    'centered text keeps its selection handles centered on the layer anchor');

  const left = PM.raster({
    type: 'text',
    d: { text: 'Powermove', font: 'Geist', weight: 650, size: 100, tracking: 0, leading: 1, color: '#fff', align: 'left', italic: false },
  }).selection;
  const right = PM.raster({
    type: 'text',
    d: { text: 'Powermove', font: 'Geist', weight: 650, size: 100, tracking: 0, leading: 1, color: '#fff', align: 'right', italic: false },
  }).selection;
  assert.ok(left.x0 < 0 && left.x0 > -12 && left.x1 > 350, 'left-aligned bounds begin at the layer anchor');
  assert.ok(right.x1 > 0 && right.x1 < 12 && right.x0 < -350, 'right-aligned bounds end at the layer anchor');

  const layout = PM.textLayout({
    text: 'AB\nC', font: 'Geist', weight: 650, size: 100,
    tracking: 0, leading: 1.1, color: '#fff', align: 'center', italic: false,
  });
  assert.deepEqual([...layout.characters.map(piece => piece.text)], ['A', 'B', 'C']);
  assert.ok(Math.abs(layout.characters[0].x + 48) <= 1 && Math.abs(layout.characters[1].x) <= 1,
    'the first centered line is decomposed around the same renderer anchor');
  assert.ok(Math.abs(layout.characters[2].x + 24) <= 1 && Math.abs(layout.characters[2].y - 110) < .001,
    'later lines retain their centered horizontal offset and leading');
  assert.deepEqual([...layout.lines.map(piece => piece.text)], ['AB', 'C']);

  const compositor = read('js/gl/compositor.js');
  assert.match(compositor, /if \(r\.selection\) return \{ \.\.\.r\.selection, ax: r\.anchorX, ay: r\.anchorY \}/,
    'canvas hit testing and transform handles consume the tight bounds');
});
