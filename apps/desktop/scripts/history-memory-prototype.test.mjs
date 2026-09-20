// Uses Node's built-in runner so these checks do not require the Vite build.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { registerHooks } from 'node:module';
import { existsSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { internHistory, encodeHistory, decodeHistory } from './history-memory-prototype.mjs';

function resolveTypescript(specifier, context, next) {
  if (specifier.startsWith('.') && context.parentURL) {
    const url = new URL(specifier + '.ts', context.parentURL);
    if (existsSync(url)) return next(url.href, context);
  }
  return next(specifier, context);
}
registerHooks({ resolve: resolveTypescript });
const historyURL = new URL('../src/renderer/src/legacy/core/history.ts', import.meta.url);
const { install } = await import(historyURL.href);
const clone = value => JSON.parse(JSON.stringify(value));

function createPM(project) {
  let id = 0;
  const PM = {
    proj: structuredClone(project), sel: { layers: [], keys: [] },
    uid: prefix => `${prefix}-${++id}`,
    replaceProject(next) { PM.proj = next; },
    touch() {}, invalidate() {}, toast() {}, autosave() {},
    bus: { emit() {} },
    store: { get: (_key, fallback) => fallback, set() {} },
  };
  install(PM);
  return PM;
}

function fixture() {
  const PM = createPM({ id: 'fixture', layers: [{ id: 'base', d: { x: 0, keys: [] } }], edits: [], assets: {} });
  const states = [clone(PM.proj)];
  for (let i = 0; i < 60; i++) {
    PM.hist.do(`Edit ${i}`, () => {
      switch (i % 6) {
        case 0: PM.proj.layers.push({ id: `layer-${i}`, d: { x: i, keys: [{ t: 0, v: i }] } }); break;
        case 1: PM.proj.layers.reverse(); break;
        case 2: PM.proj.layers[0].d.keys.push({ t: i, v: i * 2 }); break;
        case 3: PM.proj.assets[`asset-${i}`] = { kind: 'image', storageKey: `media-${i}` }; break;
        case 4: PM.proj.layers.splice(0, 1); break;
        case 5: delete PM.proj.assets[`asset-${i - 2}`]; break;
      }
      PM.proj.edits.push({ id: `edit-${i}`, label: `Edit ${i}`, operations: [{ type: 'edit', i }] });
    });
    states.push(clone(PM.proj));
  }
  for (let i = 0; i < 3; i++) PM.hist.undo();
  return { project: clone(PM.proj), saved: PM.hist.export(), states };
}

function freeze(value, seen = new WeakSet()) {
  if (!value || typeof value !== 'object' || seen.has(value)) return;
  seen.add(value);
  Object.values(value).forEach(child => freeze(child, seen));
  Object.freeze(value);
}

function reopen(project, saved, disk = false) {
  const source = clone(saved);
  internHistory(source);
  const PM = createPM(project);
  assert.equal(PM.hist.import(disk ? decodeHistory(encodeHistory(source)) : source), true);
  // Verify the production importer already shares records, then freeze them.
  const live = PM.hist.export({ copy: false });
  const edits = live.entries.flatMap(entry => [...entry.forward, ...entry.backward])
    .filter(patch => patch.path.length === 1 && patch.path[0] === 'edits').flatMap(patch => patch.value);
  assert.ok(edits.length > new Set(edits).size);
  freeze(live);
  return PM;
}

for (const disk of [false, true]) {
  test(`every Undo/Redo state matches, ${disk ? 'dictionary reload' : 'shared records'}`, () => {
    const { project, saved, states } = fixture();
    const PM = reopen(project, saved, disk);
    assert.deepEqual(PM.hist.export(), saved);
    assert.equal(PM.hist.canRedo(), true);
    for (let index = saved.index; index >= 0; index--) {
      assert.equal(PM.hist.undo(), true);
      assert.deepEqual(PM.proj, states[index]);
    }
    assert.equal(PM.hist.undo(), false);
    for (let index = 0; index < saved.entries.length; index++) {
      assert.equal(PM.hist.redo(), true);
      assert.deepEqual(PM.proj, states[index + 1]);
    }
    assert.equal(PM.hist.redo(), false);
  });
}

test('editing after Undo cannot mutate shared historical records and discards only the old Redo branch', () => {
  const { project, saved } = fixture();
  const PM = reopen(project, saved, true);
  const historical = PM.hist.export({ copy: false });
  const before = JSON.stringify(historical);
  assert.equal(PM.hist.undo(), true);
  const oldProject = clone(PM.proj);
  PM.hist.do('New branch', () => {
    PM.proj.layers[0].d.keys.push({ t: 999, v: 123 });
    PM.proj.edits.push({ id: 'new-branch', operations: [] });
  });
  const branch = clone(PM.proj);
  assert.equal(JSON.stringify(historical), before);
  assert.equal(PM.hist.canRedo(), false);
  assert.equal(PM.hist.list().length, saved.index + 1);
  assert.equal(PM.hist.undo(), true);
  assert.deepEqual(PM.proj, oldProject);
  assert.equal(PM.hist.redo(), true);
  assert.deepEqual(PM.proj, branch);
  assert.equal(JSON.stringify(historical), before);
});

test('scoped edits and failed-transaction rollback preserve frozen shared history', () => {
  const { project, saved } = fixture();
  const PM = reopen(project, saved);
  const before = clone(PM.proj);
  const historical = PM.hist.export({ copy: false });
  const serialized = JSON.stringify(historical);
  PM.hist.beginScoped('Move');
  PM.hist.track([['layers', 0, 'd', 'x']]);
  PM.proj.layers[0].d.x = 500;
  PM.hist.commit();
  PM.hist.undo();
  assert.deepEqual(PM.proj, before);
  assert.throws(() => PM.hist.do('Fail', () => {
    PM.proj.layers[0].d.keys.push({ t: 1000, v: 1000 });
    throw new Error('deliberate rollback');
  }), /deliberate rollback/);
  assert.deepEqual(PM.proj, before);
  assert.equal(JSON.stringify(historical), serialized);
});

test('empty arrays, removals, nested patches, and record zero survive the codec', () => {
  const saved = { version: 1, index: -1, entries: [{ label: 'edge cases',
    forward: [
      { path: ['layers'], exists: true, value: [] },
      { path: ['edits'], exists: true, value: [{ id: 'first' }, { id: 'first' }] },
      { path: ['assets', 'removed'], exists: false },
      { path: ['layers', 0, 'd'], exists: true, value: { nullable: null, enabled: false, number: 0, text: '' } },
    ], backward: [],
  }] };
  const before = clone(saved);
  internHistory(saved);
  const decoded = decodeHistory(encodeHistory(saved));
  assert.deepEqual(decoded, before);
  const records = decoded.entries[0].forward[1].value;
  assert.equal(records[0], records[1]);
});

test('a fresh Node process restores disk history, cursor, pending Redo, and all project states', () => {
  const data = fixture();
  internHistory(data.saved);
  const dir = mkdtempSync(join(tmpdir(), 'powermove-history-regression-'));
  try {
    const path = join(dir, 'restart.json');
    writeFileSync(path, JSON.stringify({ project: data.project, history: encodeHistory(data.saved), states: data.states }));
    const code = `
      import assert from 'node:assert/strict';
      import { existsSync, readFileSync } from 'node:fs';
      import { registerHooks } from 'node:module';
      registerHooks({ resolve: ${resolveTypescript.toString()} });
      const { install } = await import(${JSON.stringify(historyURL.href)});
      const { decodeHistory, internHistory } = await import(${JSON.stringify(new URL('./history-memory-prototype.mjs', import.meta.url).href)});
      const createPM = ${createPM.toString()};
      const data = JSON.parse(readFileSync(process.argv[1], 'utf8'));
      const saved = decodeHistory(data.history);
      const PM = createPM(data.project);
      assert.equal(PM.hist.import(saved), true);
      assert.equal(PM.hist.export().index, 56);
      assert.equal(PM.hist.canRedo(), true);
      for (let i = saved.index; i >= 0; i--) {
        assert.equal(PM.hist.undo(), true); assert.deepEqual(PM.proj, data.states[i]);
      }
      assert.equal(PM.hist.undo(), false);
      for (let i = 0; i < saved.entries.length; i++) {
        assert.equal(PM.hist.redo(), true); assert.deepEqual(PM.proj, data.states[i + 1]);
      }
      assert.equal(PM.hist.redo(), false);
      console.log('restart replay passed');
    `;
    const child = spawnSync(process.execPath, ['--input-type=module', '-e', code, path], { encoding: 'utf8', timeout: 30_000 });
    assert.equal(child.status, 0, child.stderr || String(child.error));
    assert.match(child.stdout, /restart replay passed/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
