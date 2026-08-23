const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = fs.readFileSync(path.join(root, 'js/core/scripting.js'), 'utf8');

function runtime() {
  let id = 0;
  const allowed = new Set(['set_layer', 'set_content', 'add_layer', 'delete_layers', 'set_expression', 'replace_keyframes']);
  const PM = {
    uid: prefix => `${prefix}-${++id}`,
    AgentHarness: {
      cleanCommand(command) { return command && allowed.has(command.type) ? JSON.parse(JSON.stringify(command)) : null; },
      describeCommand(command) { return `${command.type}:${command.target || command.name || ''}`; },
    },
    selLayers: () => [],
  };
  const context = vm.createContext({ window: { PM }, console, JSON, Object, Set, Map, Promise });
  vm.runInContext(source, context, { filename: 'scripting.js' });
  return PM;
}

const state = {
  composition: { revision: 4 }, selection: { layers: ['text-1'] },
  layers: [
    { id: 'text-1', name: 'Title', type: 'text', locked: false },
    { id: 'locked-1', name: 'Locked', type: 'text', locked: true },
  ],
};

test('sandbox results must be bounded typed commands and cannot touch locked layers', () => {
  const PM = runtime();
  const commands = PM.Script.test.normalizeCommands([
    { type: 'add_layer', layerType: 'text', name: 'A' },
    { type: 'set_layer', target: 'text-1', patch: { visible: false } },
  ], state);
  assert.equal(commands.length, 2);
  assert.throws(() => PM.Script.test.normalizeCommands([
    { type: 'set_layer', target: 'locked-1', patch: { visible: false } },
  ], state), /locked layers/);
  assert.throws(() => PM.Script.test.normalizeCommands([{ type: 'run_shell', command: 'whoami' }], state), /unsupported/);
  assert.throws(() => PM.Script.test.normalizeCommands([
    { type: 'set_expression', target: 'text-1', path: 'opacity', expression: 'window.location' },
  ], state), /cannot install executable project expressions/);
  assert.throws(() => PM.Script.test.normalizeCommands([
    { type: 'replace_keyframes', target: 'text-1', path: 'opacity', keyframes: [], expression: 'fetch("https:\/\/example.com")' },
  ], state), /cannot install executable project expressions/);
});

test('the isolated frame is opaque, network-blocked, worker-timed, and data-only', () => {
  const PM = runtime();
  assert.doesNotThrow(() => new vm.Script(PM.Script.test.workerSource), 'the generated Worker bootstrap parses');
  const frame = PM.Script.test.frameDocument();
  assert.match(frame, /default-src 'none'/);
  assert.match(frame, /connect-src 'none'/);
  assert.match(frame, /worker-src blob:/);
  assert.match(frame, /worker\.terminate\(\)/);
  assert.match(source, /setAttribute\('sandbox', 'allow-scripts'\)/);
  assert.doesNotMatch(source, /allow-same-origin/);
  assert.match(source, /PM\.Edit\.apply\(prepared\.commands/);
});

test('script source and result limits are explicit and the SDK has no mutation escape hatch', () => {
  const PM = runtime();
  assert.equal(PM.Script.sourceCode('return [];'), 'return [];');
  assert.equal(PM.Script.sourceCode('x'.repeat(PM.Script.LIMITS.source + 1)), '');
  const catalog = PM.Script.catalog();
  assert.equal(catalog.timeoutMs, 900);
  assert.ok(catalog.sdk.includes('addLayer'));
  assert.equal(catalog.sdk.includes('document'), false);
  assert.ok(catalog.guarantees.includes('atomic apply'));
  assert.ok(catalog.guarantees.includes('one-step undo'));
});
