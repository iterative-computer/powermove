const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = fs.readFileSync(path.join(root, 'js/assistant/spatial.js'), 'utf8');
const css = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');

function spatialModel(adapterFactory) {
  const PM = {
    h() {}, uid: () => 'id',
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    round: (value, places = 0) => Number(Number(value).toFixed(places)),
    commands: { fitView: { id: 'fitView', label: 'Fit view' } },
    PANELS: { viewer: { title: 'Composition' }, timeline: { title: 'Timeline' }, inspector: { title: 'Inspector' }, assets: { title: 'Project' } },
    registerPanel(id, definition) { this.PANELS[id] = definition; },
    firstSel: () => null, L: () => null, byName: () => null, findProp: () => null,
    AgentHarness: {
      sceneSchema: () => ({ type: 'object' }),
      sanitizeProposal: value => ({ commands: value?.commands || [] }),
      promptContext: () => '',
      describeCommand: command => command.type,
    },
  };
  const findPanel = (workspace, id) => {
    for (const dock of workspace.layout.docks) {
      const spec = dock.panels.find(panel => panel.id === id);
      if (spec) return { dock, spec };
    }
    return null;
  };
  const ensureDock = (workspace, id) => {
    let dock = workspace.layout.docks.find(item => item.id === id);
    if (!dock) { dock = { id, panels: [] }; workspace.layout.docks.push(dock); }
    return dock;
  };
  PM.Layout = {
    findPanel,
    hasPanel: (workspace, id) => !!findPanel(workspace, id),
    addPanel(workspace, id, dockId) {
      workspace.layout.docks.forEach(dock => { dock.panels = dock.panels.filter(panel => panel.id !== id); });
      ensureDock(workspace, dockId).panels.push({ id });
    },
    hidePanel(workspace, id) {
      const found = findPanel(workspace, id); if (!found || id === 'viewer') return false;
      workspace.hiddenPanels ||= [];
      workspace.hiddenPanels.push({ id, dockId: found.dock.id, index: found.dock.panels.indexOf(found.spec), spec: { ...found.spec } });
      found.dock.panels = found.dock.panels.filter(panel => panel !== found.spec); return true;
    },
    restorePanel(workspace, id) {
      const hidden = (workspace.hiddenPanels || []).find(item => item.id === id); if (!hidden) return false;
      ensureDock(workspace, hidden.dockId).panels.splice(hidden.index, 0, hidden.spec);
      workspace.hiddenPanels = workspace.hiddenPanels.filter(item => item !== hidden); return true;
    },
    movePanel(workspace, id, dockId) {
      const found = findPanel(workspace, id); if (!found || found.dock.id === dockId) return false;
      found.dock.panels = found.dock.panels.filter(panel => panel !== found.spec);
      ensureDock(workspace, dockId).panels.push(found.spec); return true;
    },
  };
  const context = vm.createContext({
    window: { PM }, console, Map, Set, Uint8Array, TextDecoder,
    navigator: { gpu: adapterFactory ? { requestAdapter: adapterFactory } : undefined },
    setTimeout, clearTimeout, atob: value => Buffer.from(value, 'base64').toString('binary'),
  });
  vm.runInContext(source, context);
  return PM.SpatialAssistant;
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

test('shake activation uses timestamp-normalized inertia and repeated reversals', () => {
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

test('drag selection resolves a clean rectangle in either direction', () => {
  const math = spatialModel().math;
  assert.deepEqual({ ...math.selectionRect({ x: 310, y: 260 }, { x: 120, y: 80 }) },
    { x: 120, y: 80, width: 190, height: 180 });
  assert.deepEqual({ ...math.selectionRect({ x: 40, y: 50 }, { x: 40, y: 50 }) },
    { x: 40, y: 50, width: 0, height: 0 });
  assert.match(source, /updateOutline\(selectionRect\(S\.points\[0\], p\)\)/,
    'the rectangular marquee follows the pointer without drawing a freehand loop');
  assert.doesNotMatch(source, /loopInfo|Close the loop|pathData\(S\.points\)/);
});

test('selected regions become cropped visual attachments with workspace semantics', () => {
  const math = spatialModel().math;
  assert.deepEqual({ ...math.bitmapCropRect(
    { x: 100, y: 50, width: 200, height: 100 }, 2560, 1440, 1280, 720,
  ) }, { sx: 200, sy: 100, sw: 400, sh: 200, width: 400, height: 200 });
  assert.deepEqual({ ...math.bitmapCropRect(
    { x: 0, y: 0, width: 1280, height: 720 }, 2560, 1440, 1280, 720, 1280,
  ) }, { sx: 0, sy: 0, sw: 2560, sh: 1440, width: 1280, height: 720 });
  assert.match(source, /S\.sceneFrame = snapshotScene\(sceneBitmap\)/,
    'the clean pre-overlay window capture is retained before WebGPU consumes it');
  assert.match(source, /open: openAgentPanel/,
    'toolbar activation opens the first-class Agent panel while shake still owns spatial capture');
  assert.match(source, /S\.regionImage = regionCapture\?\.dataUrl \|\| null/);
  assert.match(source, /\[\.\.\.userImages, \.\.\.\(S\.regionImage \? \[S\.regionImage\] : \[\]\), \.\.\.observation\.images\]/,
    'user attachments and the selected editor crop are sent before rendered review frames');
  assert.match(source, /the next attached image is an exact screenshot of the selected editor region/);
  assert.match(source, /SEMANTIC WORKSPACE MAP[\s\S]*workspaceSemanticContext\(workspace\)/);
  assert.match(source, /SELECTED REGION SEMANTICS[\s\S]*JSON\.stringify\(S\.context\)/);
  assert.match(source, /role: section \? 'generated editable section' : 'native editor panel'/);
});

test('one click exits while a drag remains a selection gesture', () => {
  const math = spatialModel().math;
  assert.equal(math.isClickGesture([{ x: 100, y: 100 }, { x: 103, y: 102 }]), true);
  assert.equal(math.isClickGesture([{ x: 100, y: 100 }, { x: 125, y: 102 }]), false);
  assert.equal(math.overlayPointerAction('arming', 0, false), 'cancel', 'clicking off the initial prompt dismisses it immediately');
  assert.equal(math.overlayPointerAction('selecting', 0, false), 'select', 'the selection phase still begins a drag');
  assert.equal(math.overlayPointerAction('composing', 0, false), 'cancel', 'clicking off a selected-area prompt dismisses the overlay');
  assert.equal(math.overlayPointerAction('composing', 0, true), 'ignore', 'clicking in the prompt remains interactive');
  assert.equal(math.overlayPointerAction('composing', 2, false), 'ignore', 'secondary clicks are left alone');
  assert.match(source, /S\.root\.addEventListener\('pointerdown', onOverlayPointerDown\)/);
  assert.match(source, /if \(isClickGesture\(S\.points\)\) \{ cancel\(\); return; \}/);
});

test('generated section manifests are bounded and discard unsafe button commands', () => {
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

test('composition width and height become working generated controls instead of refusals', () => {
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

test('valid preview chrome edits remain reviewable and mutate source only on Apply', () => {
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
  assert.match(source, /PM\.WS\.mutate\(workspace => \{ changed = applyChromeEdit/);
  assert.match(source, /Apply interface edit/);
  assert.match(source, /Never alter rendered composition shapes or export geometry/);
});

test('Timeline surface reversal is a validated workspace chrome edit', () => {
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

test('panel plans can safely control the complete panel lifecycle', () => {
  const math = spatialModel().math;
  const plan = math.sanitizePlan({
    kind: 'panels', operation: 'modify', message: 'Panel layout ready',
    panelEdit: JSON.stringify({ actions: [
      { type: 'add', panelId: 'inspector', dockId: 'right', position: 0 },
      { type: 'move', panelId: 'timeline', dockId: 'right', position: 1 },
      { type: 'resize', panelId: 'timeline', size: 420 },
      { type: 'hide', panelId: 'assets' },
      { type: 'hide', panelId: 'viewer' },
      { type: 'popout', panelId: 'timeline' },
      { type: 'popout', panelId: 'inspector' },
    ] }),
  }, { targetPanelId: '' });
  assert.equal(plan.kind, 'panels');
  assert.deepEqual([...plan.panelEdit.actions.map(action => action.type)], ['add', 'move', 'resize', 'hide', 'popout']);
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
  assert.deepEqual([...result.runtime.map(action => action.type)], ['popout']);
  assert.match(source, /Undo panel changes/);
  assert.match(source, /agentAutoApplyPanels/);
});

test('whole-workspace proposals stay reachable and keep only source-connected generated sections', () => {
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
  assert.match(source, /Create workspace/);
  assert.match(source, /PM\.WS\.create\(/);
});

test('full-window effect is a true WebGPU WGSL ripple with a reduced-motion-safe shell', () => {
  assert.match(source, /requestRippleAdapter/);
  assert.match(source, /getContext\('webgpu'/);
  assert.match(source, /@fragment[\s\S]*fn fragmentMain/);
  assert.match(source, /device\.createRenderPipeline/);
  assert.match(source, /copyExternalImageToTexture/);
  assert.match(source, /if \(shakeIntent\(S\.samples\) && !S\.rippleWarmup\) warmRipple\(\)/);
  assert.match(source, /capture: PM\.WindowCapture\.request\(\)/,
    'shake intent starts a fresh clean screenshot for the current selection');
  assert.match(source, /const sceneRequest = warmup\?\.capture[\s\S]*warmup\.capture\.then\(fresh =>/);
  assert.doesNotMatch(source, /S\.gpuAdapter/, 'no consumed adapter is cached across activations');
  assert.match(source, /adapterPromise \|\| requestRippleAdapter\(\)/);
  assert.match(source, /let displacedUV = clamp\(uv - displacement/);
  assert.match(source, /textureSample\(sceneTexture, sceneSampler/);
  assert.match(source, /let crest = exp\(-pow\(\(distanceFromSource - front\) \* 4\.4/);
  assert.match(source, /distanceFromSource \* 34\.0 - uniforms\.time \* 12\.0/,
    'shake feedback uses substantially wider-spaced bands across the editor');
  assert.match(source, /wakeMask = smoothstep\(front \+ 0\.48, front - 0\.2/);
  assert.match(source, /S\.origin\.x = live\.clientX; S\.origin\.y = live\.clientY/);
  assert.match(source, /let cursorRipple = sin\(distanceFromSource \* 38\.0/);
  assert.match(source, /let propagationFade = exp\(-uniforms\.time \* 0\.38\)/);
  assert.match(source, /let entrance = smoothstep\(0\.0, 0\.11, uniforms\.time\)/,
    'the ripple eases into view over a tenth of a second');
  assert.match(source, /let front = uniforms\.time \* 1\.32/,
    'the wavefront still advances from time zero without a gesture delay');
  assert.doesNotMatch(source, /setTimeout\([^)]*startRipple|uniforms\.time\s*-\s*0\.11/,
    'the entrance ramp never pauses ripple playback');
  assert.match(source, /displacementStrength[\s\S]*uniforms\.intensity \* entrance/,
    'displacement and light fade in together instead of popping');
  assert.match(source, /trackedOrigin\.x \+= \(origin\.x - trackedOrigin\.x\) \* follow/);
  assert.match(source, /fadeProgress \* fadeProgress \* \(3 - 2 \* fadeProgress\)/);
  assert.match(source, /const fadeStartsAt = 0\.48/);
  assert.match(source, /const settlesAt = 1\.45/);
  assert.match(source, /elapsed < settlesAt/);
  assert.match(source, /webgpu-settled/);
  assert.match(source, /format = 'rgba16float'/);
  assert.match(source, /toneMapping: \{ mode: 'extended' \}/);
  assert.match(source, /let hdrBloomNear = exp\(-pow\(\(distanceFromSource - front\) \* 1\.15/);
  assert.match(source, /let hdrBloomFar = exp\(-pow\(\(distanceFromSource - front\) \* 0\.52/);
  assert.match(source, /hdrBloomNear \* 0\.18 \+ hdrBloomFar \* 0\.045/);
  assert.match(source, /sceneColor \+ ringColor \* ringAlpha \+ hdrColor \* hdrCrest/);
  assert.match(source, /dynamicRange = hdr \? 'hdr' : 'sdr'/);
  assert.doesNotMatch(source, /getContext\('webgl2'/);
  assert.match(css, /#spatial-assistant\{position:fixed;inset:0/);
  assert.match(css, /#spatial-assistant canvas\{[^}]*pointer-events:none/,
    'expanded shake feedback cannot intercept the editor pointer');
  assert.match(css, /spatial-wash\{[^}]*rgba\(8,8,12,\.12\)/);
  assert.doesNotMatch(css, /spatial-wash\{[^}]*blur/);
  assert.match(css, /spatial-shade\{[^}]*rgba\(7,7,10,\.38\)/);
  assert.match(source, /S\.shadePath\.setAttribute\('fill-rule', 'evenodd'\)/,
    'the rectangular selection de-emphasizes everything except the selected region');
  assert.match(css, /@media \(prefers-reduced-motion:reduce\)/);
  assert.match(source, /window\.matchMedia\?\.\('\(prefers-reduced-motion: reduce\)'\)\.matches[\s\S]*renderer = 'reduced-motion'/,
    'reduced motion bypasses the WebGPU animation while leaving the assistant usable');
  assert.match(css, /\.spatial-input-row:focus-within\{[^}]*var\(--accent-dim\)/,
    'the prompt responds with a restrained token-based focus cue');
});

test('Ripple adapter acquisition is fresh on every lifecycle request', async () => {
  let calls = 0;
  const assistant = spatialModel(() => Promise.resolve({ generation: ++calls }));
  const first = await assistant.lifecycle.requestAdapter();
  const second = await assistant.lifecycle.requestAdapter();
  assert.equal(calls, 2);
  assert.notEqual(first.generation, second.generation);
  assert.match(source, /device\.lost\.then/);
  assert.match(source, /context\?\.unconfigure\?\.\(\)/);
  assert.match(source, /canvas\.style\.opacity = '\.24'/, 'fallback remains visibly meaningful');
});

test('instruction pill follows the cursor without intercepting input', () => {
  const math = spatialModel().math;
  assert.deepEqual({ ...math.hintPosition(100, 80, 250, 38, 800, 600) }, { x: 118, y: 98 });
  assert.deepEqual({ ...math.hintPosition(790, 590, 250, 38, 800, 600) }, { x: 538, y: 534 });
  assert.match(source, /S\.hint = h\('div\.spatial-hint', h\('span', 'Type a prompt or drag to select an area'\)\)/);
  assert.match(css, /\.spatial-hint\{[^}]*pointer-events:none[^}]*will-change:transform/s);
});

test('selected-region prompt hands off into the ongoing Agent workspace panel', () => {
  const math = spatialModel().math;
  assert.deepEqual({ ...math.clampFloatingPosition(-40, 900, 420, 220, 1200, 800) }, { x: 12, y: 568 });
  assert.match(source, /S\.context = inspectRegion\(S\.points, S\.region\)/, 'the prompt binds to the rectangular selection context');
  assert.match(source, /`Selected \$\{S\.context\.targetTitle\}`/,
    'Selected Timeline is displayed inside the prompt surface');
  assert.match(css, /\.spatial-context-label\{[^}]*font-family:var\(--f-ui\)/,
    'selection context uses the normal interface typeface');
  assert.match(source, /Powermove agent · full composition/);
  assert.match(source, /document\.body\.appendChild\(S\.root\);[\s\S]*showComposer\(\)/,
    'the floating prompt appears immediately after a shake; drag selection is optional context');
  assert.match(source, /sendRequest\(input\)/);
  assert.match(source, /PM\.CodexBridge\.request\([\s\S]*agentPrompt\(request, observation\), responseSchema\(\), attachedImages,[\s\S]*model: S\.model/,
    'the agent receives the selected-region image followed by rendered composition frames');
  assert.match(source, /promoteToConversation\(\); renderConversation\(\)/,
    'Send immediately moves the compact spatial prompt into the panel conversation');
  assert.match(source, /function promoteToConversation\(\)[\s\S]*dismissOverlay\(true\)[\s\S]*openAgentPanel\(\)/);
  assert.match(source, /registerPanel\('agent'/);
  assert.match(source, /PM\.Layout\.restorePanel/);
  assert.match(source, /PM\.Layout\.addPanel/);
  assert.match(source, /Describe what you want changed/);
  assert.match(source, /CONVERSATION SO FAR/);
  assert.match(source, /setPointerCapture/);
  assert.match(source, /\['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'\]/);
  assert.doesNotMatch(source, /Return to layout/, 'native pop-out behavior remains owned by the shared panel system');
  assert.match(source, /event\.key === 'Escape'[\s\S]*cancel\(\)/);
  assert.match(source, /S\.plan = plan;[\s\S]*plan\.kind === 'panels' && S\.autoApplyPanels[\s\S]*applyPlan\(\)[\s\S]*showPreview\(\)/);
  assert.match(source, /onclick: applyPlan/);
  assert.match(source, /Apply scene edit/);
  assert.match(source, /Undo change/);
  assert.match(source, /Keep change/);
  assert.match(source, /PM\.WS\.mutate\(workspace =>/,
    'Apply crosses the validated structured workspace transaction boundary');
  assert.match(css, /\.agent-composer\{[^}]*background:var\(--bg-float\)/);
  assert.match(css, /\.spatial-input-row>\.spatial-action\.pri\{[^}]*display:grid;place-items:center/,
    'the shake-prompt send button centers its icon independently of text baselines');
  assert.match(css, /\.spatial-send svg\[data-icon="return"\],\.agent-send svg\[data-icon="return"\]\{transform:translate\(-\.35px,\.45px\)\}/,
    'both Return-key icons share the same optical centering correction');
});

test('Ripple activation is limited to the active project editor', () => {
  assert.match(source, /if \(!isEditorPointer\(event\)\) \{ S\.samples = \[\]; return; \}/);
  assert.match(source, /window\.opener == null/);
  assert.match(source, /ProjectsScreen && PM\.ProjectsScreen\.isOpen/);
  assert.match(source, /LibraryUI && PM\.LibraryUI\.isOpen/);
  assert.match(source, /document\.querySelector\('#scrim\.on,\.modal'\)/);
  assert.match(source, /target\?\.closest\?\.\('#body'\)/);
  assert.match(source, /PM\.registerPanel\('agent'/, 'the resulting conversation uses the movable workspace panel');
});
