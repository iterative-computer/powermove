const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const rasterSource = fs.readFileSync(path.join(root, 'js/gl/raster.js'), 'utf8');
const appSource = fs.readFileSync(path.join(root, 'js/app.js'), 'utf8');

function deferred() {
  let resolve;
  const promise = new Promise(yes => { resolve = yes; });
  return { promise, resolve };
}

function flush() {
  return new Promise(resolve => setImmediate(resolve));
}

test('a slow audio import is discarded if its original project is no longer active', async () => {
  const preparation = deferred();
  const disposed = [];
  const projectA = { id: 'A', assets: {}, layers: [] };
  const projectB = { id: 'B', assets: {}, layers: [] };
  const PM = {
    proj: projectA,
    clamp(value, min, max) { return Math.max(min, Math.min(max, value)); },
    uid() { return 'asset-new'; },
    Audio: {
      accepts() { return true; },
      prepareAsset() { return preparation.promise; },
      disposeAsset(asset) { disposed.push(asset); },
      rebalanceCache() {},
    },
    MediaStore: { put: async () => true },
    MediaImport: {
      fingerprint: async () => 'fingerprint',
      storageKeyFor: value => 'media:' + value,
      match: () => ({ canonicalId: null, aliases: [] }),
      coalesce: () => 0,
      mapBounded: async (items, limit, worker) => Promise.all(items.map(worker)),
    },
    bus: { emit() {} },
    touch() {},
  };
  const document = { createElement() { return { getContext() { return {}; } }; } };
  const context = vm.createContext({
    window: { PM, navigator: { hardwareConcurrency: 4 } },
    document,
    console,
    URL: { createObjectURL() { return 'blob:test'; }, revokeObjectURL() {} },
    Image: class {},
    Promise,
    Map,
    setTimeout,
    clearTimeout,
  });
  vm.runInContext(rasterSource, context, { filename: 'js/gl/raster.js' });

  const importing = PM.assets.add({ name: 'slow.mp3', type: 'audio/mpeg', size: 10 });
  await flush();
  PM.proj = projectB;
  PM.assets.clear();
  const prepared = { id: 'asset-new', name: 'slow.mp3', kind: 'audio' };
  preparation.resolve(prepared);

  await assert.rejects(importing, /switched projects/);
  assert.deepEqual(projectA.assets, {});
  assert.deepEqual(projectB.assets, {});
  assert.equal(PM.assets.map.size, 0);
  assert.deepEqual(disposed, [prepared]);
});

test('the public import queue captures the project selected when the request begins', () => {
  assert.match(appSource, /PM\.importFiles = \(files, \{ project = PM\.proj \} = \{\}\)/);
  assert.match(appSource, /if \(PM\.proj !== project\)/);
  assert.match(appSource, /PM\.importFiles\(files, \{ project: targetProject \}\)/);
});
