const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = fs.readFileSync(path.join(root, 'js/core/media.js'), 'utf8');

function loadMedia(PM = {}, indexedDB = null) {
  const window = { PM, indexedDB, crypto: crypto.webcrypto };
  vm.runInContext(source, vm.createContext({
    window, Blob, Date, Promise, TextEncoder, Uint8Array, ArrayBuffer,
    setTimeout, clearTimeout, setImmediate,
  }));
  return PM;
}

test('media fingerprints are content based and ignore file names', async () => {
  const PM = loadMedia();
  const first = new Blob(['same media bytes'], { type: 'audio/mpeg' });
  const renamed = new Blob(['same media bytes'], { type: 'application/octet-stream' });
  first.name = 'first.mp3';
  renamed.name = 'renamed.mp3';

  assert.equal(await PM.MediaImport.fingerprint(first), await PM.MediaImport.fingerprint(renamed));
});

test('fingerprinting large media reads only bounded samples', async () => {
  const PM = loadMedia();
  const reads = [];
  const huge = {
    size: 8 * 1024 * 1024 * 1024,
    type: 'video/mp4',
    slice(start, end) {
      reads.push([start, end]);
      return { arrayBuffer: async () => new Uint8Array(end - start).buffer };
    },
  };

  const fingerprint = await PM.MediaImport.fingerprint(huge);

  assert.match(fingerprint, /^v2:/);
  assert.ok(reads.length <= 3, `expected at most three samples, got ${reads.length}`);
  assert.ok(reads.reduce((sum, [start, end]) => sum + end - start, 0) <= 192 * 1024);
});

test('bounded media work preserves order without exceeding its concurrency limit', async () => {
  const PM = loadMedia();
  let active = 0;
  let peak = 0;
  const output = await PM.MediaImport.mapBounded([1, 2, 3, 4, 5], 2, async value => {
    active++;
    peak = Math.max(peak, active);
    await new Promise(resolve => setTimeout(resolve, value % 2 ? 4 : 1));
    active--;
    return value * 10;
  });

  assert.deepEqual(Array.from(output), [10, 20, 30, 40, 50]);
  assert.equal(peak, 2);
});

test('legacy duplicate imports relink layers to the one live asset', () => {
  const PM = loadMedia();
  const project = {
    assets: {
      missing: { id: 'missing', name: 'song.mp3', kind: 'audio', dur: 29.58 },
      live: { id: 'live', name: 'song.mp3', kind: 'audio', dur: 29.58, size: 451000 },
    },
    layers: [{ id: 'audio-layer', type: 'audio', d: { asset: 'missing' } }],
    comps: {},
  };
  const liveAssets = new Map([['live', { id: 'live' }]]);
  const identity = { name: 'song.mp3', kind: 'audio', dur: 29.58, size: 451000, fingerprint: 'v2:451000:abc' };

  const plan = PM.MediaImport.match(project, liveAssets, identity);
  assert.equal(plan.canonicalId, 'live');
  assert.deepEqual(Array.from(plan.aliases), ['missing']);

  const changed = PM.MediaImport.coalesce(project, plan.canonicalId, plan.aliases);
  assert.equal(changed, 1);
  assert.equal(project.layers[0].d.asset, 'live');
  assert.equal(project.assets.missing, undefined);
});

test('removing project media also removes every layer that uses it', () => {
  const PM = loadMedia();
  const project = {
    assets: {
      used: { id: 'used', name: 'clip.mov', kind: 'video' },
      kept: { id: 'kept', name: 'still.png', kind: 'image' },
    },
    layers: [
      { id: 'parent', type: 'video', d: { asset: 'used' } },
      { id: 'child', parent: 'parent', type: 'text', d: {} },
      { id: 'kept-layer', type: 'image', d: { asset: 'kept' } },
    ],
    comps: {
      nested: { layers: [{ id: 'nested-use', type: 'video', d: { asset: 'used' } }] },
    },
  };

  assert.equal(PM.MediaImport.referenceCount(project, 'used'), 2);
  const result = PM.MediaImport.removeAsset(project, 'used');

  assert.equal(result.removedLayers, 2);
  assert.deepEqual(Array.from(result.removedLayerIds).sort(), ['nested-use', 'parent']);
  assert.equal(project.assets.used, undefined);
  assert.ok(project.assets.kept);
  assert.deepEqual(Array.from(project.layers.map(layer => layer.id)), ['child', 'kept-layer']);
  assert.equal(project.layers[0].parent, null, 'children of removed media layers are safely unparented');
  assert.deepEqual(Array.from(project.comps.nested.layers), []);
});
