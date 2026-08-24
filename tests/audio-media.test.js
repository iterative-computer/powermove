const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const mediaSource = fs.readFileSync(path.join(root, 'js/core/media.js'), 'utf8');
const shortcutsSource = fs.readFileSync(path.join(root, 'js/ui/shortcuts.js'), 'utf8');

function fakeIndexedDB() {
  const records = new Map();
  const storeFor = tx => ({
    put(record) { records.set(record.id, record); return request(record.id, tx); },
    get(id) { return request(records.get(id), tx); },
    delete(id) { records.delete(id); return request(undefined, tx); },
  });
  function request(result, tx) {
    const req = { result };
    setImmediate(() => {
      if (req.onsuccess) req.onsuccess();
      setImmediate(() => { if (tx.oncomplete) tx.oncomplete(); });
    });
    return req;
  }
  const db = {
    objectStoreNames: { contains: () => true },
    createObjectStore() {},
    transaction() {
      const tx = {};
      tx.objectStore = () => storeFor(tx);
      return tx;
    },
  };
  return {
    open() {
      const req = { result: db };
      setImmediate(() => { if (req.onsuccess) req.onsuccess(); });
      return req;
    },
  };
}

function loadMedia(PM = {}, indexedDB = fakeIndexedDB()) {
  const window = { PM, indexedDB };
  vm.runInContext(mediaSource, vm.createContext({ window, Blob, Date, Promise, setImmediate, setTimeout, clearTimeout }));
  return PM;
}

test('generic imported media blobs survive a MediaStore round trip', async () => {
  const PM = loadMedia();
  const original = new Blob(['exact media bytes'], { type: 'video/mp4' });

  assert.equal(await PM.MediaStore.put('media-1', original), true);
  const restored = await PM.MediaStore.get('media-1');

  assert.ok(restored instanceof Blob);
  assert.equal(restored.type, 'video/mp4');
  assert.equal(await restored.text(), 'exact media bytes');
});

test('video trim timing preserves source time at playback speed', () => {
  const PM = loadMedia({}, null);
  const video = { type: 'video', from: 2, d: { trim: 1, speed: 2 } };

  assert.equal(PM.MediaTiming.trimAtStart(video, 5), 7);
  assert.equal(PM.MediaTiming.earliestStart(video), 1.5);
});

function shortcutHarness(layer, asset) {
  let added;
  const PM = loadMedia({
    proj: { w: 1920, h: 1080, fps: 30, dur: 8, assets: asset ? { [asset.id]: asset } : {}, layers: layer ? [layer] : [] },
    time: layer ? 5 : .9,
    h() { return {}; },
    snapF(v) { return v; },
    Edit: {
      apply(command) {
        added = command;
        return { ok: true, data: { results: [{ data: { id: 'new-layer' } }] } };
      },
    },
    L() { return { id: 'new-layer' }; },
    hist: { do(label, fn) { return fn(); } },
    selLayers() { return layer ? [layer] : []; },
    cloneLayer(value) { return JSON.parse(JSON.stringify(value)); },
    uid() { return 'copy'; },
    selectLayers() {},
    bus: { emit() {} },
  }, null);
  const context = vm.createContext({ window: { PM }, addEventListener() {}, document: {}, navigator: {}, console });
  vm.runInContext(shortcutsSource, context);
  return { PM, added: () => added };
}

test('the audio layer command keeps complete source duration and canonical editable content', () => {
  const asset = { id: 'asset-1', name: 'Diamonds.mp3', kind: 'audio', dur: 29.58, w: 0, h: 0 };
  const { PM, added } = shortcutHarness(null, asset);

  PM.cmd('addFromAsset', asset.id);

  assert.equal(added().layerType, 'audio');
  assert.equal(added().duration, 29.58);
  assert.equal(added().from, .9);
  assert.deepEqual(JSON.parse(JSON.stringify(added().content)), {
    asset: 'asset-1', trim: 0, gain: 1, fadeIn: 0, fadeOut: 0,
  });
});

test('splitting an audio layer preserves source continuity without playback-engine internals', () => {
  const layer = { id: 'audio-1', type: 'audio', from: 2, dur: 10, d: { asset: 'asset-1', trim: 1 } };
  const { PM } = shortcutHarness(layer, null);

  PM.cmd('split');

  assert.equal(layer.dur, 3);
  const right = PM.proj.layers[0];
  assert.equal(right.from, 5);
  assert.equal(right.dur, 7);
  assert.equal(right.d.trim, 4);
  assert.equal(layer.d.trim, 1);
  assert.notEqual(right.d, layer.d, 'split layers keep independently editable source settings');
});
