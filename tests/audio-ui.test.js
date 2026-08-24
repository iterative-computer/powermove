const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const read = file => fs.readFileSync(path.join(root, file), 'utf8');
const inspector = read('js/ui/inspector.js');
const timeline = read('js/ui/timeline.js');

function section(source, start, end) {
  const from = source.indexOf(start);
  const to = source.indexOf(end, from + start.length);
  assert.notEqual(from, -1, `missing start marker: ${start}`);
  assert.notEqual(to, -1, `missing end marker: ${end}`);
  return source.slice(from, to);
}

test('audio inspector exposes only canonical audio content through source commands', () => {
  const audio = section(inspector, 'function audioContent', 'function numRow');
  assert.match(audio, /type: 'set_content'/);
  assert.match(audio, /const commandOnly = \(\) => \{\};/);
  for (const field of ['asset', 'trim', 'gain', 'fadeIn', 'fadeOut']) {
    assert.match(audio, new RegExp(`command\\('${field}'`), `${field} uses set_content`);
  }
  assert.doesNotMatch(audio, /\bd(?:\.[A-Za-z_$][\w$]*|\[[^\]]+\])\s*=(?!=)/, 'audio controls have no direct content mutation path');
});

test('audio inspector omits visual sections but keeps timing and visibility', () => {
  const render = section(inspector, 'function render()', '/* ── rows');
  assert.match(render, /if \(L\.type !== 'audio'\) \{[\s\S]*transform\(wrap, L\);[\s\S]*effects\(wrap, L\);[\s\S]*masksSection\(wrap, L\);/);

  const header = section(inspector, 'header(hdr)', 'I.refresh');
  assert.match(header, /if \(!L \|\| L\.type !== 'audio'\) actions\.push\(/, 'audio selection hides Add effect');

  const layer = section(inspector, "if (L.type === 'audio') {\n    wrap.appendChild(PM.row('Visible'", "  wrap.appendChild(PM.row('Blend mode'");
  assert.match(layer, /layerEdit\('visible'/);
  assert.match(layer, /layerEdit\('from'/);
  assert.match(layer, /layerEdit\('duration'/);
  assert.doesNotMatch(layer, /Blend mode|Motion blur|Parent|Add effect/);
});

test('audio timeline delegates real waveform drawing and has no synthetic texture', () => {
  const waveform = section(timeline, 'function drawAudioClipWaveform', 'function drawClip');
  assert.match(waveform, /PM\.Audio\.drawWaveform\(c, L, \{ x, y, width, height, clipLeft \}\)/);
  assert.match(waveform, /if \(right > clipLeft\)/, 'unavailable waveform renderer gets a neutral fallback');
  assert.match(timeline, /if \(L\.type === 'audio'\) drawAudioClipWaveform/);
  assert.doesNotMatch(timeline, /waveform-ish|Math\.sin/);
});
