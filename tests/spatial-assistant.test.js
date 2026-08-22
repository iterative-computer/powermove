const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = fs.readFileSync(path.join(root, 'js/assistant/spatial.js'), 'utf8');

function spatialModel() {
  const PM = {
    h() {}, uid: () => 'id',
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    commands: { fitView: { id: 'fitView', label: 'Fit view' } },
  };
  const context = vm.createContext({
    window: { PM }, console, Map, Set, Uint8Array, TextDecoder,
    setTimeout, clearTimeout, atob: value => Buffer.from(value, 'base64').toString('binary'),
  });
  vm.runInContext(source, context);
  return PM.SpatialAssistant.math;
}

test('shake activation requires fast repeated reversals instead of ordinary travel', () => {
  const math = spatialModel();
  const shake = Array.from({ length: 10 }, (_, i) => ({ x: i % 2 ? 150 : 0, y: 80, t: i * 60 }));
  const easierShake = Array.from({ length: 8 }, (_, i) => ({ x: i % 2 ? 85 : 0, y: 80, t: i * 72 }));
  const tinyJitter = Array.from({ length: 12 }, (_, i) => ({ x: i % 2 ? 10 : 0, y: 80, t: i * 55 }));
  const travel = Array.from({ length: 10 }, (_, i) => ({ x: i * 40, y: 80, t: i * 60 }));
  assert.equal(math.shakeReady(shake), true);
  assert.equal(math.shakeReady(easierShake), true);
  assert.equal(math.shakeReady(tinyJitter), false);
  assert.equal(math.shakeReady(travel), false);
  assert.equal(math.shakeIntent(easierShake.slice(0, 5)), true, 'early deliberate motion prewarms the ripple');
  assert.equal(math.shakeIntent(tinyJitter), false, 'tiny jitter never starts expensive preparation');
});

test('circle selection rejects open scribbles and accepts a closed useful region', () => {
  const math = spatialModel();
  const circle = Array.from({ length: 25 }, (_, i) => {
    const a = (i / 24) * Math.PI * 2;
    return { x: 220 + Math.cos(a) * 90, y: 180 + Math.sin(a) * 60 };
  });
  const open = circle.slice(0, 17);
  assert.equal(math.loopInfo(circle).closed, true);
  assert.equal(math.loopInfo(open).closed, false);
});

test('one click exits while a drag remains a selection gesture', () => {
  const math = spatialModel();
  assert.equal(math.isClickGesture([{ x: 100, y: 100 }, { x: 103, y: 102 }]), true);
  assert.equal(math.isClickGesture([{ x: 100, y: 100 }, { x: 125, y: 102 }]), false);
  assert.match(source, /if \(isClickGesture\(S\.points\)\) \{ cancel\(\); return; \}/);
});

test('instruction pill follows the cursor with a stable offset and viewport clamping', () => {
  const math = spatialModel();
  assert.deepEqual({ ...math.hintPosition({ x: 100, y: 80 }, { width: 800, height: 600 }, { width: 196, height: 38 }) }, { x: 116, y: 98 });
  assert.deepEqual({ ...math.hintPosition({ x: 790, y: 590 }, { width: 800, height: 600 }, { width: 196, height: 38 }) }, { x: 592, y: 550 });
  assert.match(source, /S\.origin\.x = live\.clientX; S\.origin\.y = live\.clientY;\s*queueHint\(live\.clientX, live\.clientY\)/);
  assert.match(source, /S\.hint = h\('div\.spatial-hint', h\('span', 'Circle any part of the interface'\)\)/);
  assert.doesNotMatch(source, /S\.hint = h\('div\.spatial-hint', h\('i'\)/);
});

test('device loss clears stale WebGPU pixels and prepares a fresh adapter', () => {
  assert.match(source, /context\?\.unconfigure\?\.\(\)/);
  assert.match(source, /canvas\.width = 1; canvas\.height = 1;[\s\S]*canvas\.dataset\.renderer = 'css-fallback'/);
  assert.match(source, /const reason = info\?\.reason \|\| 'unknown'/);
  assert.match(source, /reason !== 'destroyed'\) S\.gpuAdapter = navigator\.gpu\?\.requestAdapter\?\.\(\)/);
  assert.match(source, /addEventListener\?\.\('uncapturederror'/);
  assert.match(source, /const adapter = await[\s\S]*S\.gpuAdapter = navigator\.gpu\.requestAdapter\(\);\s*device = await adapter\.requestDevice\(\)/,
    'each claimed adapter is rotated before a device is requested');
  assert.doesNotMatch(source, /powerPreference: 'high-performance'/);
});

test('generated section manifests are bounded and discard unsafe button commands', () => {
  const math = spatialModel();
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
  const css = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');
  assert.match(source, /navigator\.gpu\.requestAdapter/);
  assert.match(source, /getContext\('webgpu'/);
  assert.match(source, /@fragment[\s\S]*fn fragmentMain/);
  assert.match(source, /device\.createRenderPipeline/);
  assert.match(source, /copyExternalImageToTexture/);
  assert.match(source, /if \(shakeIntent\(S\.samples\) && !S\.rippleWarmup\) warmRipple\(\)/);
  assert.match(source, /S\.gpuAdapter = navigator\.gpu\?\.requestAdapter[\s\S]*refreshSceneCache\(\)/);
  assert.match(source, /cachedScene \? Promise\.resolve\(cachedScene\)/);
  assert.match(source, /S\.gpuAdapter = navigator\.gpu\?\.requestAdapter/);
  assert.match(source, /adapterPromise \|\| navigator\.gpu\.requestAdapter/);
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
  assert.match(css, /spatial-wash\{[^}]*rgba\(8,8,12,\.012\)/);
  assert.doesNotMatch(css, /spatial-wash\{[^}]*blur/);
  assert.match(css, /@media \(prefers-reduced-motion:reduce\)/);
});
