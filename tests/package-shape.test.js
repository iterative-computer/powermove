const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');

function files(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const target = path.join(dir, entry.name);
    return entry.isDirectory() ? files(target) : [target];
  });
}

test('all browser scripts parse', () => {
  for (const file of files(path.join(root, 'js')).filter((name) => name.endsWith('.js'))) {
    assert.doesNotThrow(() => new vm.Script(fs.readFileSync(file, 'utf8'), { filename: file }));
  }
});

test('spatial assistant uses the native Codex bridge without writable shell access', () => {
  const spatial = fs.readFileSync(path.join(root, 'js/assistant/spatial.js'), 'utf8');
  const native = fs.readFileSync(path.join(root, 'native/main.swift'), 'utf8');
  assert.match(spatial, /messageHandlers\?\.pmCodex/);
  assert.match(spatial, /responseSchema\(\)/);
  assert.match(native, /name: "pmCodex"/);
  assert.match(native, /name: "pmCaptureWindow"/);
  assert.match(native, /takeSnapshot/);
  assert.match(native, /wantsExtendedDynamicRangeContent = true/);
  assert.match(native, /"--sandbox",\s*"read-only"/);
});
