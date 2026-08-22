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
    commands: { fitView: { id: 'fitView', label: 'Fit view' } },
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

test('circle selection rejects open scribbles and accepts a closed useful region', () => {
  const math = spatialModel().math;
  const circle = Array.from({ length: 25 }, (_, i) => {
    const a = (i / 24) * Math.PI * 2;
    return { x: 220 + Math.cos(a) * 90, y: 180 + Math.sin(a) * 60 };
  });
  const open = circle.slice(0, 17);
  assert.equal(math.loopInfo(circle).closed, true);
  assert.equal(math.loopInfo(open).closed, false);
});

test('one click exits while a drag remains a selection gesture', () => {
  const math = spatialModel().math;
  assert.equal(math.isClickGesture([{ x: 100, y: 100 }, { x: 103, y: 102 }]), true);
  assert.equal(math.isClickGesture([{ x: 100, y: 100 }, { x: 125, y: 102 }]), false);
  assert.match(source, /if \(isClickGesture\(S\.points\)\) \{ cancel\(\); return; \}/);
});

test('generated section manifests are bounded and discard unsafe button commands', () => {
  const math = spatialModel().math;
  const plan = math.sanitizePlan({
    operation: 'modify', targetPanelId: 'inspector', placement: 'replace', message: 'Ready',
    section: { id: 'Fresh Controls!', title: 'Fresh Controls', size: 5000, note: 'Useful controls', controls: [
      { type: 'slider', label: 'Opacity', parameter: '', defaultValue: 35, min: 0, max: 100, step: 1, target: '$selection', path: 'properties.opacity' },
      { type: 'button', label: 'Unknown', command: 'runAnything' },
      { type: 'button', label: 'Fit', command: 'fitView' },
    ] },
  }, { targetPanelId: 'viewer' });
  assert.equal(plan.section.id, 'fresh-controls');
  assert.equal(plan.section.size, 700);
  assert.deepEqual([...plan.section.controls.map(c => c.label)], ['Opacity', 'Fit']);
  assert.equal(plan.section.controls[0].target, '$selection');
});

test('full-window effect is a true WebGPU WGSL ripple with a reduced-motion-safe shell', () => {
  assert.match(source, /requestRippleAdapter/);
  assert.match(source, /getContext\('webgpu'/);
  assert.match(source, /@fragment[\s\S]*fn fragmentMain/);
  assert.match(source, /device\.createRenderPipeline/);
  assert.match(source, /copyExternalImageToTexture/);
  assert.match(source, /if \(shakeIntent\(S\.samples\) && !S\.rippleWarmup\) warmRipple\(\)/);
  assert.match(source, /cachedScene \? Promise\.resolve\(cachedScene\)/);
  assert.doesNotMatch(source, /S\.gpuAdapter/, 'no consumed adapter is cached across activations');
  assert.match(source, /adapterPromise \|\| requestRippleAdapter\(\)/);
  assert.match(source, /let displacedUV = clamp\(uv - displacement/);
  assert.match(source, /textureSample\(sceneTexture, sceneSampler/);
  assert.match(source, /let crest = exp\(-pow\(\(distanceFromSource - front\) \* 7\.0/);
  assert.match(source, /S\.origin\.x = live\.clientX; S\.origin\.y = live\.clientY/);
  assert.match(source, /let cursorRipple = sin\(distanceFromSource \* 38\.0/);
  assert.match(source, /let front = uniforms\.time \* 1\.32/);
  assert.match(source, /let propagationFade = exp\(-uniforms\.time \* 0\.38\)/);
  assert.match(source, /let lightIn = smoothstep\(0\.0, 0\.16, uniforms\.time\)/);
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
  assert.match(css, /spatial-wash\{[^}]*rgba\(8,8,12,\.12\)/);
  assert.doesNotMatch(css, /spatial-wash\{[^}]*blur/);
  assert.match(css, /spatial-shade\{[^}]*rgba\(7,7,10,\.38\)/);
  assert.match(source, /S\.shadePath\.setAttribute\('fill-rule', 'evenodd'\)/,
    'the lasso de-emphasizes everything except the circled region');
  assert.match(css, /@media \(prefers-reduced-motion:reduce\)/);
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
  assert.match(source, /S\.hint = h\('div\.spatial-hint', h\('span', 'Circle any part of the interface'\)\)/);
  assert.match(css, /\.spatial-hint\{[^}]*pointer-events:none[^}]*will-change:transform/s);
});

test('circled-region prompt, Send, movable result, cancel, Preview, and Apply stay structured', () => {
  const math = spatialModel().math;
  assert.deepEqual({ ...math.clampFloatingPosition(-40, 900, 420, 220, 1200, 800) }, { x: 12, y: 568 });
  assert.match(source, /S\.context = inspectRegion\(S\.points, S\.region\)/, 'the prompt binds to the lasso context');
  assert.match(source, /sendRequest\(input, status, sendBtn, cancelBtn\)/);
  assert.match(source, /PM\.CodexBridge\.request\(agentPrompt\(request\), responseSchema\(\)\)/);
  assert.match(source, /makeCardMovable\(S\.card, handle\)/);
  assert.match(source, /setPointerCapture/);
  assert.match(source, /\['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'\]/);
  assert.match(source, /Return to layout/);
  assert.match(source, /event\.key === 'Escape'[\s\S]*cancel\(\)/);
  assert.match(source, /S\.plan = plan; showPreview\(\)/);
  assert.match(source, /onclick: applyPlan/);
  assert.match(source, /PM\.WS\.mutate\(workspace =>/,
    'Apply crosses the validated structured workspace transaction boundary');
});
