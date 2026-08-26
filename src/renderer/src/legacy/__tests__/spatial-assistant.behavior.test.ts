// @ts-nocheck -- faithful behavioral transplant of exported spatial pure/adapter seams.
import assert from 'node:assert/strict';
import { afterEach, it, vi } from 'vitest';

import { makePM } from './make-pm';
import { install as installElectronShim } from '../host/electron-shim';

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function spatialHarness(adapterFactory) {
  vi.stubGlobal('window', {
    navigator: {
      gpu: adapterFactory ? { requestAdapter: adapterFactory } : undefined,
    },
    console,
    setTimeout,
    clearTimeout,
    AbortController,
    atob: value => Buffer.from(value, 'base64').toString('binary'),
    TextDecoder,
    addEventListener() {},
  });

  const PM = makePM(
    'core/easing',
    'core/capabilities',
    'ui/layout',
    'core/workspace',
    'assistant/harness',
    'assistant/spatial',
  );
  PM.TYPE_META = { text: {} };
  Object.assign(PM.PANELS, {
    viewer: { title: 'Composition' },
    timeline: { title: 'Timeline' },
    inspector: { title: 'Inspector' },
    assets: { title: 'Project' },
  });
  PM.commands = { fitView: { id: 'fitView', label: 'Fit view' } };
  PM.firstSel = () => null;
  PM.L = () => null;
  PM.byName = () => null;
  PM.findProp = () => null;
  return { PM, assistant: PM.SpatialAssistant };
}

function spatialModel(adapterFactory) {
  return spatialHarness(adapterFactory).assistant;
}

function sampledShake(step) {
  const turns = [0, 60, 0, 60, 0];
  const out = [];
  for (let t = 0; t <= 480; t += step) {
    const segment = Math.min(3, Math.floor(t / 120));
    const f = (t - segment * 120) / 120;
    out.push({ x: turns[segment] + (turns[segment + 1] - turns[segment]) * f, y: 80, t });
  }
  return out;
}

it('shake activation uses timestamp-normalized inertia and repeated reversals', () => {
  const math = spatialModel().math;
  const deliberate60hz = sampledShake(16);
  const deliberate120hz = sampledShake(8);
  const slow = sampledShake(80).map(point => ({ ...point, t: point.t * 4 }));
  const sweep = Array.from({ length: 14 }, (_, i) => ({ x: i * 24, y: 80, t: i * 24 }));
  const jitter = Array.from({ length: 80 }, (_, i) => ({ x: i % 2 ? 3 : 0, y: 80, t: i * 5 }));
  assert.equal(math.shakeReady(deliberate60hz), true, 'short deliberate shake activates');
  assert.equal(math.shakeReady(deliberate120hz), true, 'sampling rate does not change the outcome');
  assert.equal(math.shakeReady(slow), false, 'slow navigation does not activate');
  assert.equal(math.shakeReady(sweep), false, 'one fast sweep has no repeated reversals');
  assert.equal(math.shakeReady(jitter), false, 'high-frequency tiny jitter lacks useful span');
  assert.ok(math.motionProfile(deliberate60hz).reversals >= 3);
  assert.equal(math.shakeIntent(deliberate60hz.slice(0, 14)), true, 'early intentional inertia prewarms Ripple');
});

it('selected regions become cropped visual attachments with workspace semantics', () => {
  const math = spatialModel().math;
  assert.deepEqual({ ...math.bitmapCropRect(
    { x: 100, y: 50, width: 200, height: 100 }, 2560, 1440, 1280, 720,
  ) }, { sx: 200, sy: 100, sw: 400, sh: 200, width: 400, height: 200 });
  assert.deepEqual({ ...math.bitmapCropRect(
    { x: 0, y: 0, width: 1280, height: 720 }, 2560, 1440, 1280, 720, 1280,
  ) }, { sx: 0, sy: 0, sw: 2560, sh: 1440, width: 1280, height: 720 });
});

it('generated section manifests are bounded and discard unsafe button commands', () => {
  const math = spatialModel().math;
  const plan = math.sanitizePlan({
    operation: 'modify', targetPanelId: 'inspector', placement: 'replace', message: 'Ready',
    section: { id: 'Fresh Controls!', title: 'Fresh Controls', size: 5000, note: 'Useful controls', controls: [
      { type: 'color', label: 'Start color', parameter: '', defaultValue: '#0A84FF', min: 0, max: 100, step: 1, target: '$composition', path: 'composition.background.startColor' },
      { type: 'slider', label: 'Disconnected', parameter: 'Looks useful', defaultValue: 35, min: 0, max: 100, step: 1, target: '', path: '' },
      { type: 'button', label: 'Unknown', command: 'runAnything' },
      { type: 'button', label: 'Fit', command: 'fitView' },
    ] },
  }, { targetPanelId: 'viewer' });
  assert.equal(plan.section.id, 'fresh-controls');
  assert.equal(plan.section.size, 700);
  assert.deepEqual([...plan.section.controls.map(c => c.label)], ['Start color', 'Fit']);
  assert.equal(plan.section.controls[0].target, '$composition');
  assert.equal(plan.section.controls[0].connection, 'Composition background');
});

it('known generated tools expand into validated selection-aware native panels', () => {
  const plan = spatialModel().math.sanitizePlan({
    kind: 'section', operation: 'create', message: 'Stagger tool ready',
    section: { id: 'timing-tool', title: 'Layer Stagger', size: 300, note: '', tool: 'layer-stagger', controls: [] },
  }, { targetPanelId: 'timeline' });
  assert.equal(plan.section.tool, 'layer-stagger');
  assert.deepEqual([...plan.section.controls.map(control => control.label)], [
    'Selection', 'Offset', 'Order', 'Anchor', 'Preview', 'Apply Stagger', 'Undo last edit',
  ]);
  assert.equal(plan.section.controls.find(control => control.label === 'Offset').stateKey, 'offsetFrames');
  assert.equal(plan.section.controls.find(control => control.label === 'Apply Stagger').action.type, 'transform');
  assert.equal(plan.section.controls.find(control => control.label === 'Apply Stagger').primary, true);
});

it('agents can author bounded visual curves and source-connected easing actions', () => {
  const action = JSON.stringify({
    type: 'easing', mode: 'apply', scope: 'selected-keyframes', curveState: 'curve', defaultCurve: [.62, .05, 0, 1],
  });
  const plan = spatialModel().math.sanitizePlan({
    kind: 'section', operation: 'create', message: 'Flow controls ready',
    section: { id: 'flow-tool', title: 'Flow', size: 420, note: '', controls: [
      { type: 'readout', label: 'Target', source: 'keyframes.summary' },
      { type: 'curve', label: 'Curve', stateKey: 'curve', defaultValue: '[0.42,0,0.58,1]', min: -1, max: 2, options: ['linear', 'easeInOut', 'unknown'] },
      { type: 'button', label: 'Apply', action, primary: true },
      { type: 'button', label: 'Unsafe', action: JSON.stringify({ type: 'easing', mode: 'apply', scope: 'all-project-keyframes', curveState: 'curve' }) },
    ] },
  }, { targetPanelId: 'timeline' });

  assert.deepEqual([...plan.section.controls.map(control => control.label)], ['Target', 'Curve', 'Apply']);
  assert.equal(plan.section.controls[0].connection, 'Live keyframes');
  assert.equal(plan.section.controls[1].type, 'curve');
  assert.deepEqual([...plan.section.controls[1].def], [.42, 0, .58, 1]);
  assert.deepEqual([...plan.section.controls[1].presets], ['linear', 'easeInOut']);
  assert.equal(plan.section.controls[2].action.type, 'easing');
  assert.equal(plan.section.controls[2].connection, 'Source action');
});

it('the reusable easing tool expands into a complete Flow-style native panel', () => {
  const plan = spatialModel().math.sanitizePlan({
    kind: 'section', operation: 'create', message: 'Easing tool ready',
    section: { id: 'flow', title: 'Flow', size: 430, note: '', tool: 'easing-flow', controls: [] },
  }, { targetPanelId: 'timeline' });
  assert.equal(plan.section.tool, 'easing-flow');
  assert.deepEqual([...plan.section.controls.map(control => control.type)], ['readout', 'curve', 'button', 'button', 'button']);
  assert.equal(plan.section.controls.find(control => control.label === 'Apply easing').action.type, 'easing');
});

it('advanced generated tool actions accept bounded transforms and reject executable transform expressions', () => {
  const action = JSON.stringify({
    type: 'transform', mode: 'apply', transform: {
      label: 'Offset layers', selector: { scope: 'selection' }, order: 'selection',
      edits: [{ path: 'layer.from', value: { op: 'add', args: [{ ref: 'current' }, { op: 'frames', args: [{ state: 'offset' }] }] } }],
    },
  });
  const plan = spatialModel().math.sanitizePlan({
    kind: 'section', operation: 'create', message: 'Timing tool ready',
    section: { id: 'offset-tool', title: 'Offset', size: 240, note: '', controls: [
      { type: 'slider', label: 'Offset', stateKey: 'offset', defaultValue: 2, min: -60, max: 60, step: 1 },
      { type: 'button', label: 'Apply', action, primary: true },
      { type: 'button', label: 'Unsafe', action: JSON.stringify({ type: 'transform', mode: 'apply', transform: {
        selector: { scope: 'selection' }, edits: [{ path: 'layer.from', value: { op: 'javascript', args: ['alert(1)'] } }],
      } }) },
    ] },
  }, { targetPanelId: 'timeline' });
  assert.deepEqual([...plan.section.controls.map(control => control.label)], ['Offset', 'Apply']);
  assert.equal(plan.section.controls[0].connection, 'Tool setting');
  assert.equal(plan.section.controls[1].connection, 'Source action');
});

it('generated sections accept bounded sandbox scripts for procedural source tools', () => {
  const action = JSON.stringify({
    type: 'script', mode: 'apply', label: 'Build copies', requiredTypes: ['text'],
    code: "const source=PM.selectedLayers[0]; PM.assert(source, 'Select text'); return [PM.setContent(source.id,{text:'Safe'})];",
  });
  const plan = spatialModel().math.sanitizePlan({
    kind: 'section', operation: 'create', message: 'Procedural tool ready',
    section: { id: 'procedural', title: 'Procedural', size: 240, note: '', controls: [
      { type: 'button', label: 'Apply', action, primary: true },
    ] },
  }, { targetPanelId: 'timeline' });
  assert.equal(plan.section.controls.length, 1);
  assert.equal(plan.section.controls[0].action.type, 'script');
  assert.equal(plan.section.controls[0].action.mode, 'apply');
  assert.deepEqual([...plan.section.controls[0].action.requiredTypes], ['text']);
});

it('composition width and height become working generated controls instead of refusals', () => {
  const plan = spatialModel().math.sanitizePlan({
    kind: 'section', operation: 'create', message: 'Size controls ready',
    section: { id: 'composition-size', title: 'Composition size', size: 220, note: '', controls: [
      { type: 'slider', label: 'Width', target: '$composition', path: 'composition.width', defaultValue: 1920, min: 16, max: 16384, step: 1 },
      { type: 'slider', label: 'Height', target: '$composition', path: 'composition.height', defaultValue: 1080, min: 16, max: 16384, step: 1 },
    ] },
  }, { targetPanelId: 'viewer' });
  assert.deepEqual([...plan.section.controls.map(control => control.path)], ['composition.width', 'composition.height']);
  assert.deepEqual([...plan.section.controls.map(control => control.connection)], ['Composition', 'Composition']);
});

it('valid preview chrome edits remain reviewable and mutate source only on Apply', () => {
  const math = spatialModel().math;
  const workspace = { chrome: { previewCornerRadius: 'rounded' } };
  const plan = math.sanitizePlan({
    kind: 'chrome', operation: 'modify', targetPanelId: 'viewer', dockId: 'center', placement: 'replace',
    message: 'Use square preview corners', chromeEdit: { target: 'preview.cornerRadius', value: 'square' },
    section: { id: '', title: '', size: 220, note: '', controls: [] },
  }, { targetPanelId: 'viewer' });
  assert.equal(plan.kind, 'chrome');
  assert.equal(workspace.chrome.previewCornerRadius, 'rounded', 'proposal and Preview do not mutate source');
  assert.equal(math.applyChromeEdit(workspace, plan.chromeEdit), true);
  assert.equal(workspace.chrome.previewCornerRadius, 'square', 'Apply changes the structured workspace source');
  assert.equal(math.applyChromeEdit(workspace, { target: 'project.shape', value: 'square' }), false,
    'unapproved project/render targets are rejected');
});

it('Timeline surface reversal is a validated workspace chrome edit', () => {
  const math = spatialModel().math;
  const workspace = { chrome: { timelineSurfaceOrder: 'normal' } };
  const plan = math.sanitizePlan({
    kind: 'chrome', operation: 'modify', targetPanelId: 'timeline', dockId: 'center', placement: 'replace',
    message: 'Reverse the Timeline surfaces', steps: ['Reverse the Timeline surfaces'],
    chromeEdit: { target: 'timeline.surfaceOrder', value: 'reversed' },
    section: { id: '', title: '', size: 220, note: '', controls: [] },
  }, { targetPanelId: 'timeline' });
  assert.deepEqual({ ...plan.chromeEdit }, { target: 'timeline.surfaceOrder', value: 'reversed' });
  assert.equal(math.applyChromeEdit(workspace, plan.chromeEdit), true);
  assert.equal(workspace.chrome.timelineSurfaceOrder, 'reversed');
});

it('Timeline redesigns use a bounded interface manifest without touching project source', () => {
  const plan = spatialModel().math.sanitizePlan({
    kind: 'interface', operation: 'modify', message: 'Compact Timeline ready',
    interfaceEdit: JSON.stringify({ target: 'timeline', patch: {
      rowHeight: 18, gutterWidth: 420, clipRadius: 4, showLayerNumbers: false,
    } }),
    section: { id: '', title: '', size: 220, note: '', controls: [] },
  }, { targetPanelId: 'timeline' });
  assert.equal(plan.kind, 'interface');
  assert.deepEqual({ ...plan.interfaceEdit.patch }, {
    rowHeight: 22, gutterWidth: 360, clipRadius: 4, showLayerNumbers: false,
  });
});

it('panel plans can safely control the complete panel lifecycle', () => {
  const math = spatialModel().math;
  const plan = math.sanitizePlan({
    kind: 'panels', operation: 'modify', message: 'Panel layout ready',
    panelEdit: JSON.stringify({ actions: [
      { type: 'add', panelId: 'inspector', dockId: 'right', position: 0 },
      { type: 'move', panelId: 'timeline', dockId: 'right', position: 1 },
      { type: 'resize', panelId: 'timeline', size: 420 },
      { type: 'hide', panelId: 'assets' },
      { type: 'hide', panelId: 'viewer' },
      { type: 'float', panelId: 'timeline' },
      { type: 'attach', panelId: 'inspector' },
    ] }),
  }, { targetPanelId: '' });
  assert.equal(plan.kind, 'panels');
  assert.deepEqual([...plan.panelEdit.actions.map(action => action.type)], ['add', 'move', 'resize', 'hide']);
  const workspace = {
    layout: { docks: [
      { id: 'left', panels: [{ id: 'assets' }] },
      { id: 'center', panels: [{ id: 'viewer' }, { id: 'timeline' }] },
    ] }, hiddenPanels: [],
  };
  const result = math.applyPanelEdit(workspace, plan.panelEdit);
  assert.deepEqual(workspace.layout.docks.find(dock => dock.id === 'right').panels.map(panel => panel.id), ['inspector', 'timeline']);
  assert.equal(workspace.layout.docks.find(dock => dock.id === 'right').panels[1].size, 420);
  assert.equal(workspace.hiddenPanels[0].id, 'assets');
  assert.deepEqual([...result.runtime.map(action => action.type)], []);
});

it('whole-workspace proposals stay reachable and keep only source-connected generated sections', () => {
  const plan = spatialModel().math.sanitizePlan({
    kind: 'workspace', operation: 'create', message: 'Workspace ready',
    workspaceEdit: JSON.stringify({
      name: 'Gradient Focus', density: 'compact', accent: '#0A84FF',
      docks: [
        { id: 'left', size: 300, panels: [{ id: 'gradient-tools' }, { id: 'made-up-panel' }] },
        { id: 'right', size: 320, panels: [{ id: 'inspector' }] },
      ],
      sections: [{
        id: 'gradient-tools', title: 'Gradient tools', size: 240, note: 'Edits the actual background', controls: [
          { type: 'color', label: 'Start color', target: '$composition', path: 'composition.background.startColor', defaultValue: '#112233', min: 0, max: 0, step: 0, options: [], parameter: '', command: '' },
          { type: 'slider', label: 'Disconnected', target: '', path: '', defaultValue: 50, min: 0, max: 100, step: 1, options: [], parameter: 'Fake', command: '' },
        ],
      }],
    }),
    chromeEdit: { target: 'preview.cornerRadius', value: 'square' },
    section: { id: '', title: '', size: 220, note: '', controls: [] },
    sceneEdit: { label: '', summary: '', commands: [], reviewTimes: [] },
  }, { targetPanelId: 'viewer' });
  assert.equal(plan.kind, 'workspace');
  assert.equal(plan.workspaceEdit.name, 'Gradient Focus');
  assert.equal(plan.workspaceEdit.sections[0].controls.length, 1);
  assert.ok(plan.workspaceEdit.docks.some(dock => dock.panels.some(panel => panel.id === 'viewer')), 'Composition remains reachable');
  assert.equal(plan.workspaceEdit.docks.some(dock => dock.panels.some(panel => panel.id === 'made-up-panel')), false);
});

it('Ripple adapter acquisition is fresh on every lifecycle request', async () => {
  let calls = 0;
  const assistant = spatialModel(() => Promise.resolve({ generation: ++calls }));
  const first = await assistant.lifecycle.requestAdapter();
  const second = await assistant.lifecycle.requestAdapter();
  assert.equal(calls, 2);
  assert.notEqual(first.generation, second.generation);
});

it('normalizes only valid typed extension changes', () => {
  const { PM, assistant } = spatialHarness();
  PM.proj = { id: 'project-1' };
  const summary = 's'.repeat(200);
  const result = assistant.math.normalizeAutonomousResult({ summary: 'Done' }, [
      { id: 'new-mod', action: 'created', summary },
      { id: 'updated-mod', action: 'updated' },
      { id: 'removed-mod', action: 'removed' },
      { id: 'x', action: 'created' },
      { id: 'Uppercase-mod', action: 'updated' },
      { id: 'bad-action', action: 'changed' },
      { id: 'long-summary', action: 'created', summary: 's'.repeat(201) },
      { id: 'wrong-summary', action: 'created', summary: 12 },
  ]);

  assert.deepEqual(result.extensions, [
    { id: 'new-mod', action: 'created', summary },
    { id: 'updated-mod', action: 'updated' },
    { id: 'removed-mod', action: 'removed' },
  ]);
});

it('caps normalized extension changes at 32 entries', () => {
  const { PM, assistant } = spatialHarness();
  PM.proj = { id: 'project-1' };
  const extensions = Array.from({ length: 40 }, (_, index) => ({ id: `mod-${index}`, action: 'updated' }));
  assert.equal(assistant.math.normalizeAutonomousResult({}, extensions).extensions.length, 32);
});

it('reloads created and updated mods while leaving removals to the watcher', async () => {
  const { PM, assistant } = spatialHarness();
  const reload = vi.fn(async () => {});
  PM.Kernel = { loader: { reload, records: () => [
    { id: 'new-mod', manifest: { name: 'New Mod' }, health: { state: 'ok' } },
    { id: 'updated-mod', manifest: { name: 'Updated Mod' }, health: { state: 'ok' } },
  ] } };

  const turns = await assistant.lifecycle.applyExtensionChanges([
    { id: 'new-mod', action: 'created' },
    { id: 'updated-mod', action: 'updated' },
    { id: 'removed-mod', action: 'removed' },
  ]);

  assert.deepEqual(reload.mock.calls, [['new-mod'], ['updated-mod']]);
  assert.deepEqual(turns.map(turn => turn.text), [
    'Added mod New Mod',
    'Updated mod Updated Mod',
    'Removed mod removed-mod',
  ]);
});

it('reloads typed extension changes during the autonomous request flow', async () => {
  const { PM } = spatialHarness();
  const reload = vi.fn(async () => {});
  PM.proj = { id: 'project-1', name: 'Test Project', revision: 0, layers: [] };
  PM.hist = { mark: vi.fn(() => 1), squash: vi.fn() };
  PM.AgentHarness = {
    observe: vi.fn(async () => ({ state: {}, times: [], images: [] })),
    cleanCommand: vi.fn((command) => command),
  };
  PM.Kernel = { loader: {
    reload,
    records: () => [{ id: 'new-mod', manifest: { name: 'New Mod' }, health: { state: 'ok' } }],
  } };
  PM.CodexBridge.request = vi.fn(async () => ({
    text: JSON.stringify({ summary: 'Built the mod', commands: [], artifacts: [], externalActions: [], notes: [] }),
    extensions: [{ id: 'new-mod', action: 'created' }],
  }));

  PM.AgentUI.submit('Build a mod');
  await vi.waitFor(() => assert.notEqual(PM.AgentUI.state.phase, 'running'), { timeout: 1_500 });
  assert.equal(PM.AgentUI.state.phase, 'result', JSON.stringify(PM.AgentUI.state.conversation));

  assert.deepEqual(reload.mock.calls, [['new-mod']]);
  assert.ok(PM.AgentUI.state.conversation.some(turn => turn.text === 'Added mod New Mod'));
});

it('offers Fix it when a reloaded mod is unhealthy', async () => {
  const { PM, assistant } = spatialHarness();
  PM.Kernel = { loader: {
    reload: vi.fn(async () => {}),
    records: () => [{
      id: 'broken-mod', manifest: { name: 'Broken Mod' },
      health: { state: 'activation-error', error: 'Unexpected token\nindex.ts:12' },
    }],
  } };

  const turns = await assistant.lifecycle.applyExtensionChanges([{ id: 'broken-mod', action: 'updated' }]);
  assert.deepEqual(turns, [
    { role: 'assistant', text: 'Updated mod Broken Mod' },
    { role: 'assistant', text: "Broken Mod didn't load: Unexpected token", fixExtensionId: 'broken-mod' },
  ]);
});

it('builds and automatically submits an extension Fix-it prompt in project mode', async () => {
  const { PM, assistant } = spatialHarness();
  const files = [{ path: 'index.ts', text: 'throw new Error()' }];
  const readSource = vi.fn(async () => files);
  const fixPrompt = vi.fn(async () => 'Fix broken-mod without changing its public id.');
  (window as any).powermove = { extensions: { readSource }, codex: { fixPrompt } };
  PM.Kernel = { loader: { records: () => [{
    id: 'broken-mod', manifest: { name: 'Broken Mod' },
    health: { state: 'build-error', error: 'Build failed\nstack' },
  }] } };
  PM.AgentUI = { setDraft: vi.fn(), submit: vi.fn(), update: vi.fn() };

  await assistant.requestFix('broken-mod');

  assert.deepEqual(readSource.mock.calls, [[{ id: 'broken-mod' }]]);
  assert.deepEqual(fixPrompt.mock.calls, [[{ id: 'broken-mod', error: 'Build failed\nstack', files }]]);
  assert.equal(PM.store.get('agentAccessMode', ''), 'project');
  assert.deepEqual(PM.AgentUI.setDraft.mock.calls, [['Fix broken-mod without changing its public id.', true]]);
  assert.deepEqual(PM.AgentUI.submit.mock.calls, [['Fix broken-mod without changing its public id.']]);
});

it('warns and no-ops when the Fix-it bridge is absent', async () => {
  const { assistant } = spatialHarness();
  const warn = vi.spyOn(window.console, 'warn').mockImplementation(() => {});
  await assistant.requestFix('missing-mod');
  assert.equal(warn.mock.calls.length, 1);
  assert.match(warn.mock.calls[0][0], /Fix it is unavailable/);
});

it('forwards typed extension changes through the Electron shim payload', async () => {
  const extensions = [{ id: 'new-mod', action: 'created', summary: 'Adds a panel' }];
  const resolve = vi.fn();
  const nativeBridge = {
    codex: {
      run: vi.fn(async () => ({ ok: true, text: '{"summary":"Done"}', access: 'project', extensions })),
      requestComputerConsent: vi.fn(),
      cancel: vi.fn(async () => {}),
    },
    store: { snapshotSync: () => ({}), set: vi.fn(), delete: vi.fn(), onError: vi.fn() },
    log: vi.fn(),
    onMenuCommand: vi.fn(),
  };
  vi.stubGlobal('window', {
    powermove: nativeBridge,
    TextEncoder,
    atob: value => Buffer.from(value, 'base64').toString('binary'),
    btoa: value => Buffer.from(value, 'binary').toString('base64'),
    structuredClone,
    console,
    setTimeout,
    addEventListener() {},
    document: {
      readyState: 'loading',
      addEventListener() {},
      documentElement: { classList: { add() {} } },
    },
  });
  const PM = { CodexBridge: { resolve, progress: vi.fn() }, toast: vi.fn() };
  installElectronShim(PM as any);

  (window as any).webkit.messageHandlers.pmCodex.postMessage({
    id: 'run-1', mode: 'autonomous', access: 'project', prompt: 'Build a mod', projectId: 'project-1',
  });
  await vi.waitFor(() => assert.equal(resolve.mock.calls.length, 1));
  assert.deepEqual(resolve.mock.calls[0][1].extensions, extensions);
});
