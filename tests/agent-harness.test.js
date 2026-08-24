const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = name => fs.readFileSync(path.join(root, name), 'utf8');

function harnessEditor() {
  let id = 0;
  const PM = {
    version: 1,
    uid: prefix => `${prefix}-${++id}`,
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    round: (value, places = 0) => Number(Number(value).toFixed(places)),
    snapF: (time, fps) => Math.round(time * fps) / fps,
    Ease: {
      handles: () => ({ eo: [.33, 0], ei: [.67, 1] }),
      bezier: () => value => value, spring: value => value, nameOf: () => 'power',
    },
    SHADER_TEMPLATE: 'void main(){}',
    bus: { emit() {} }, invalidate() {}, toast() {},
    store: { get: (_key, fallback) => fallback, set() {} },
    FX: {}, GL: { dropProgram() {} },
  };
  const context = vm.createContext({
    window: { PM }, console, Date, JSON, Object, Set, Map, Promise,
    requestAnimationFrame: resolve => resolve(),
  });
  for (const file of ['js/core/model.js', 'js/core/anim.js']) vm.runInContext(source(file), context, { filename: file });
  PM.proj = PM.mkProject({ name: 'Harness test', w: 1920, h: 1080, fps: 30, dur: 6 });
  PM.time = 1;
  PM.syncShaderUniforms = () => {};
  PM.mkEffect = () => null;
  vm.runInContext(source('js/core/history.js'), context, { filename: 'js/core/history.js' });
  vm.runInContext(source('js/core/editing.js'), context, { filename: 'js/core/editing.js' });
  PM.Export = { snapshot: time => `data:image/jpeg;base64,frame-${time}` };
  vm.runInContext(source('js/assistant/harness.js'), context, { filename: 'js/assistant/harness.js' });
  return PM;
}

test('harness accepts only bounded typed source edits and preserves hand intent', () => {
  const PM = harnessEditor();
  const layer = PM.mkLayer('text', { name: 'Title' });
  layer.id = 'title'; PM.addLayer(layer, 0); PM.selectLayers(layer.id);
  const raw = {
    label: 'Move title', summary: 'A safe motion adjustment', reviewTimes: [-3, 1, 99],
    commands: [
      JSON.stringify({ type: 'set_property', target: 'title', path: 'position.x', value: 700, preserveHandEdits: false, overrideLock: true }),
      JSON.stringify({ type: 'create_section', section: { id: 'not-scene-source' } }),
      'not json',
    ],
  };
  const plan = PM.AgentHarness.sanitizeProposal(raw);
  assert.equal(plan.commands.length, 1);
  assert.equal(plan.commands[0].type, 'set_property');
  assert.equal(plan.commands[0].preserveHandEdits, true);
  assert.equal('overrideLock' in plan.commands[0], false);
  assert.deepEqual([...plan.reviewTimes], [0, 1, 6]);
  assert.equal(plan.baseRevision, 0);
});

test('harness observes source and real frames, applies one revision, and rolls the whole run back', async () => {
  const PM = harnessEditor();
  const bridgeCalls = [];
  PM.CodexBridge = {
    async request(prompt, schema, images) {
      bridgeCalls.push({ prompt, schema, images });
      return JSON.stringify({ status: 'pass', message: 'Looks correct', critique: '', commands: [], reviewTimes: [0, 3, 6] });
    },
  };
  const proposal = PM.AgentHarness.sanitizeProposal({
    label: 'Add title', summary: 'Add a real editable title', reviewTimes: [0, 3, 6],
    commands: [JSON.stringify({
      type: 'add_layer', id: 'agent-title', layerType: 'text', name: 'Agent title',
      from: 0, duration: 6, content: { text: 'Connected' }, properties: { 'position.x': 960, 'position.y': 540 },
    })],
  });
  const run = await PM.AgentHarness.execute('Add a title', proposal);
  assert.ok(PM.L('agent-title'), 'the proposed source layer is live');
  assert.equal(PM.proj.revision, 1);
  assert.equal(PM.proj.edits[0].origin, 'agent');
  assert.ok(bridgeCalls[0].images.length >= 3, 'rendered frames are attached to visual review');
  assert.match(bridgeCalls[0].prompt, /visual review stage/);
  assert.ok(run.frames.images.length >= 3, 'final approval receives rendered pixels');
  assert.equal(PM.hist.list().length, 1, 'the complete agent run occupies one history entry');
  assert.equal(PM.AgentHarness.rollback(run.checkpoint), true);
  assert.equal(PM.L('agent-title'), null, 'rollback restores the pre-run project');
  assert.equal(PM.hist.redo(), true, 'the standard history can restore an undone agent run');
  assert.ok(PM.L('agent-title'));
});

test('initial edits and visual-review repairs collapse into one reversible agent step', async () => {
  const PM = harnessEditor();
  let review = 0;
  PM.CodexBridge = {
    async request(_prompt, _schema, _images, options) {
      options?.onProgress?.('Checking title placement against the rendered frame');
      if (review++ === 0) return JSON.stringify({
        status: 'repair', message: 'Move it right', critique: 'The title is too far left', reviewTimes: [1],
        commands: [JSON.stringify({ type: 'set_property', target: 'agent-title', path: 'position.x', value: 800 })],
      });
      return JSON.stringify({ status: 'pass', message: 'Looks correct', critique: '', commands: [], reviewTimes: [1] });
    },
  };
  const proposal = PM.AgentHarness.sanitizeProposal({
    label: 'Add and place title', summary: 'Add a title', reviewTimes: [1],
    commands: [JSON.stringify({
      type: 'add_layer', id: 'agent-title', layerType: 'text', name: 'Agent title',
      content: { text: 'Reversible' }, properties: { 'position.x': 300, 'position.y': 300 },
    })],
  });
  await PM.AgentHarness.execute('Add a title', proposal);
  assert.equal(PM.hist.list().length, 1);
  assert.equal(PM.hist.undo(), true);
  assert.equal(PM.L('agent-title'), null, 'one undo reverses the initial edit and its repair');
  assert.equal(PM.hist.redo(), true);
  const title = PM.L('agent-title');
  assert.equal(PM.evP(title, PM.findProp(title, 'position.x'), 1, 'position.x'), 800);
});

test('native Codex bridge attaches temporary rendered frames without writable agent access', () => {
  const native = source('native/main.swift');
  assert.match(native, /images: \[String\] = \[\]/);
  assert.match(native, /arguments\.append\(contentsOf: \["--image", imageURL\.path\]\)/);
  assert.ok(native.indexOf('arguments.append(prompt)') < native.indexOf('arguments.append(contentsOf: ["--image", imageURL.path])'),
    'the prompt must precede variadic --image arguments');
  assert.match(native, /"--sandbox", "read-only"/);
  assert.match(native, /"--json"/);
  assert.match(native, /\["reasoning", "agent_message"\]\.contains\(type\)/);
  assert.match(native, /\["message", "summary", "critique", "status"\]/);
  assert.match(native, /sendCodexProgress/);
  assert.match(native, /data\.count <= 4_000_000/);
});
