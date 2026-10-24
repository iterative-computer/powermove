import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

// The inventory script lives at the repository root (docs/rust-engine.md, section 6).
import { collectWriters, isScannedFile, scanFile, serialize, SNAPSHOT } from '../../../scripts/engine-writers.mjs';

type Entry = { line: number; class: string; scope: string; code: string };
const lines = (...source: string[]) => source.join('\n');
const summary = (entries: Entry[]) => entries.map(entry => `${entry.line} ${entry.class}`).sort();

describe('engine writer inventory', () => {
  it('matches the checked-in engine-writers.json', async () => {
    const current = serialize(await collectWriters());
    const snapshot = await readFile(SNAPSHOT, 'utf8');
    // On failure: run `node scripts/engine-writers.mjs --write` from the repository
    // root, then review every new or moved writer and its class before committing.
    expect(current, 'apps/desktop/scripts/engine-writers.json is stale; regenerate it with node scripts/engine-writers.mjs --write').toBe(snapshot);
  }, 60_000);

  it('classifies history entry points, touches, and direct writes', async () => {
    const entries = await scanFile('renderer/src/sample.ts', lines(
      "PM.hist.do('a', () => { PM.proj.name = 'inside history'; });", // 1
      "api.history.do('b', () => {});", // 2
      "PM.Edit.mutate('c', () => { PM.proj.layers.push(layer); });", // 3
      'PM.WS.mutate(workspace => { workspace.x = 1; });', // 4: workspace, not the document
      'PM.touch(); api.anim.touch();', // 5
      'PM.proj = next;', // 6
      'PM.proj.revision++;', // 7
      'PM.proj.assets[id] = meta;', // 8
      'const project = PM.proj; project.w = 3;', // 9
      'for (const layer of PM.proj.layers) layer.parent = null;', // 10
      'PM.proj.layers.forEach(item => { item.x = 1; });', // 11
      'let local = PM.proj; local = other;', // 12: rebinding a local is not a write
      'delete PM.proj.notes;', // 13
      'Object.assign(PM.proj, patch);', // 14
      'const copy = JSON.parse(JSON.stringify(PM.proj)); copy.w = 1;', // 15: a copy
      'const transact = (label, fn) => PM.hist.do(label, fn);', // 16
      "transact('t', () => { PM.proj.comps = {}; });", // 17
      'extension.host.pm.proj.w = 1;', // 18
      'window.PM?.proj?.markers.sort(byTime);', // 19
      "PM.hist.begin('drag'); PM.hist.external('x', undo, redo); PM.replaceProject(next);", // 20
      'shaderHooks(PM)?.syncShaderUniforms(layer);', // 21
      'const meta = PM.proj.assets[id]; meta.folder = target;', // 22
      "PM.touch([['layers', 0]], true); PM.doc.touched([['name']]);", // 23
      "PM.doc.write([['name']], () => { PM.proj.name = 'declared'; });", // 24
    ));
    expect(summary(entries)).toEqual([
      '1 hist.do', '10 project-write', '11 project-write', '13 project-write', '14 project-write', '16 hist.do', '17 hist.do',
      '18 host.pm', '18 project-write', '19 project-write', '2 history.do', '20 history.external', '20 history.transaction',
      '20 project-replace', '21 shader-uniforms', '22 asset-metadata', '3 edit.mutate', '5 anim.touch', '5 touch', '6 project-swap',
      '7 revision', '8 asset-metadata', '9 project-write', '23 touch.paths', '23 doc.touched', '24 doc.write',
    ].sort());
  });

  it('names enclosing scopes', async () => {
    const entries = await scanFile('renderer/src/sample.ts', lines(
      'export function install(PM) {',
      '  PM.autosave = () => { PM.proj.saved = true; };',
      '  class Store { put() { PM.touch(); } }',
      '}',
    ));
    expect(entries.map(entry => entry.scope)).toEqual(['install > PM.autosave', 'install > Store.put']);
  });

  it('scans Svelte scripts and template expressions with their real line numbers', async () => {
    const entries = await scanFile('renderer/src/panels/Sample.svelte', lines(
      '<script lang="ts">', // 1
      '  const { api } = props;', // 2
      '  function mutate(label: string, edit: () => void) { api.edit.mutate(label, () => edit()); }', // 3
      '  const save = () => { PM.proj.notes = value; };', // 4
      '</script>', // 5
      '', // 6
      '<button onclick={() => api.anim.touch()}>Touch</button>', // 7
      "<button onclick={() => mutate('Rename', () => { layer.name = 'x'; })}>Rename</button>", // 8
      '{#each items as item}', // 9
      '  {@const project = PM.proj}', // 10
      '  <span>{(project.w = item.w)}</span>', // 11
      '{/each}', // 12
    ));
    expect(summary(entries)).toEqual(['11 project-write', '3 edit.mutate', '4 project-write', '7 anim.touch', '8 edit.mutate'].sort());
    expect(entries.find(entry => entry.line === 7)?.scope).toMatch(/^\(template\)/);
  });

  it('covers source files only', () => {
    expect(isScannedFile('renderer/src/legacy/app.ts')).toBe(true);
    expect(isScannedFile('extensions/inspector/ChannelRow.svelte')).toBe(true);
    expect(isScannedFile('main/agent-tools/mcp-server.mjs')).toBe(true);
    for (const excluded of [
      'renderer/src/legacy/app.test.ts', 'extensions/timeline/fake-api.test-helper.ts', 'renderer/src/legacy/__tests__/make-pm.ts',
      'renderer/src/kernel/__fixtures__/fake-pm.ts', 'test-setup/waapi.ts', 'renderer/public/onboarding/player.js',
      'shared/patch 2.ts', 'renderer/src/folder 2/file.ts', 'renderer/src/env.d.ts', 'renderer/src/style.css',
    ]) expect(isScannedFile(excluded), excluded).toBe(false);
  });

  it('fails when a named section 6 site no longer matches the code', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'powermove-writers-'));
    try {
      await mkdir(path.join(root, 'renderer'), { recursive: true });
      await writeFile(path.join(root, 'renderer', 'only.ts'), 'PM.touch();\n');
      await expect(collectWriters(root)).rejects.toThrow('Named writer sites no longer match any code');
    } finally { await rm(root, { recursive: true, force: true }); }
  });
});
