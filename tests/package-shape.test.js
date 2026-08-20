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

test('clean interface includes the native ChatGPT subscription bridge', () => {
  const provider = fs.readFileSync(path.join(root, 'js/agent/providers.js'), 'utf8');
  const native = fs.readFileSync(path.join(root, 'native/main.swift'), 'utf8');
  assert.match(provider, /ChatGPT subscription/);
  assert.match(provider, /messageHandlers\?\.pmCodex/);
  assert.match(native, /name: "pmCodex"/);
  assert.match(native, /"--sandbox",\s*"read-only"/);
});
