/*
 * The rebase gates of the customization corpus (owner amendment 24;
 * docs/engine/extensions-compat.md): a user's modified fork of a built-in as
 * the released app shipped it (e2e/fixtures/compat-corpus/shipped) rebased
 * with `api.extensions.rebase`'s local path (update-fork.ts) onto the
 * built-in as the engine rewrote it (src/extensions). The merge carries the
 * built-in's migration off deprecated members (the viewer's two
 * `render.gl.context` call sites, now `render.gl.canvas`) while keeping the
 * user's own changes; overlapping changes are a conflict left to the agent;
 * the update can be undone exactly; and a fork that is never rebased keeps
 * working (the e2e corpus loads it unchanged).
 */
import { cp, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

import { analyzeExtensionSource } from '../../shared/extension-analysis';
import { restoreExtensionChangeSet } from '../codex/change-history';
import { readForkRebaseInfo } from './rebase';
import { updateFork } from './update-fork';

const DESKTOP = path.resolve(__dirname, '../../..');
const SHIPPED = path.join(DESKTOP, 'e2e/fixtures/compat-corpus/shipped');
const BUILTINS = path.join(DESKTOP, 'src/extensions');

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true }))); });

/** A user data folder with `<id>-fork` written as `api.extensions.fork` writes it (fork.ts), from the shipped source. */
async function forkOf(id: string): Promise<{ userData: string; userExtensionsDir: string; builtinExtensionsDir: string; forkId: string; live: string }> {
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-rebase-corpus-'));
  roots.push(userData);
  const userExtensionsDir = path.join(userData, 'extensions');
  const builtinExtensionsDir = path.join(userData, 'builtins');
  const forkId = `${id}-fork`;
  const live = path.join(userExtensionsDir, forkId);
  const skipTests = (source: string) => !/\.test\.ts$/.test(source);
  await cp(path.join(SHIPPED, id), live, { recursive: true });
  await cp(path.join(SHIPPED, id), path.join(live, '.forked-from'), { recursive: true });
  // The built-in as this app ships it now (tests are not shipped).
  await cp(path.join(BUILTINS, id), path.join(builtinExtensionsDir, id), { recursive: true, filter: skipTests });
  const manifest = JSON.parse(await readFile(path.join(SHIPPED, id, 'manifest.json'), 'utf8'));
  await writeFile(path.join(live, 'manifest.json'), `${JSON.stringify({
    ...manifest, id: forkId, name: `${manifest.name} (mine)`, replaces: [id], forkedFrom: `${id}@${manifest.version}`, author: 'user'
  }, null, 2)}\n`);
  return { userData, userExtensionsDir, builtinExtensionsDir, forkId, live };
}

async function edit(file: string, from: string, to: string): Promise<void> {
  const text = await readFile(file, 'utf8');
  expect(text, `${path.basename(file)} has the line to edit`).toContain(from);
  await writeFile(file, text.replace(from, to));
}

const deprecated = async (file: string, relative: string) =>
  analyzeExtensionSource([{ path: relative, text: await readFile(file, 'utf8') }]).map((w) => `${w.member}:${w.line}`);

describe('rebasing a modified fork onto the engine-rewritten built-in', () => {
  it('viewer: the built-in moved to the engine, so a rebase is offered', async () => {
    const setup = await forkOf('viewer');
    const shipped = JSON.parse(await readFile(path.join(SHIPPED, 'viewer/manifest.json'), 'utf8'));
    const current = JSON.parse(await readFile(path.join(BUILTINS, 'viewer/manifest.json'), 'utf8'));
    expect(await readForkRebaseInfo(setup)).toEqual({ forkId: 'viewer-fork', forkedFrom: 'viewer', base: shipped.version, current: current.version });
    expect(current.version).not.toBe(shipped.version);
  });

  it('viewer, no conflict: the deprecated calls are migrated, the user\'s change is kept, and undo restores the fork exactly', async () => {
    const setup = await forkOf('viewer');
    const viewer = path.join(setup.live, 'viewer.ts');
    // The user's own change, away from the lines the built-in changed.
    await edit(viewer, 'const ZOOM_LAYOUT_VERSION = 3;', 'const ZOOM_LAYOUT_VERSION = 3; // mine: keep the zoom model');
    const before = await readFile(viewer, 'utf8');
    const beforeManifest = await readFile(path.join(setup.live, 'manifest.json'), 'utf8');
    expect(await deprecated(viewer, 'viewer.ts')).toEqual(expect.arrayContaining([expect.stringMatching(/^render\.gl\.context:/)]));

    const result = await updateFork(setup);
    expect(result).toMatchObject({ kind: 'updated' });
    if (result.kind !== 'updated') throw new Error('expected a local update');
    const merged = await readFile(viewer, 'utf8');
    // The built-in's migration came along: no deprecated compatibility member left.
    expect(await deprecated(viewer, 'viewer.ts')).toEqual([]);
    expect(merged).toContain('const renderingCanvas = api.render.gl.canvas;');
    expect(merged).toContain('if (!api.render.gl.canvas) api.render.gl.init(gl);');
    // And the user's change stayed.
    expect(merged).toContain('const ZOOM_LAYOUT_VERSION = 3; // mine: keep the zoom model');
    const manifest = JSON.parse(await readFile(path.join(setup.live, 'manifest.json'), 'utf8'));
    const current = JSON.parse(await readFile(path.join(BUILTINS, 'viewer/manifest.json'), 'utf8'));
    expect(manifest).toMatchObject({ id: 'viewer-fork', name: 'Viewer (mine)', replaces: ['viewer'], forkedFrom: `viewer@${current.version}`, author: 'user' });
    // The fork's base is now the built-in it was rebased onto.
    expect(await readFile(path.join(setup.live, '.forked-from/viewer.ts'), 'utf8')).toBe(await readFile(path.join(BUILTINS, 'viewer/viewer.ts'), 'utf8'));

    // Undo: the exact fork the user had (it keeps working without a rebase).
    await restoreExtensionChangeSet({ liveDirectory: setup.userExtensionsDir, historyRoot: path.join(setup.userData, 'Agent Change History', result.projectId), changeSetId: result.changeSetId });
    expect(await readFile(viewer, 'utf8')).toBe(before);
    expect(await readFile(path.join(setup.live, 'manifest.json'), 'utf8')).toBe(beforeManifest);
  }, 120_000);

  it('viewer, conflict: the user changed the line the built-in migrated; the live fork is left untouched for the agent', async () => {
    const setup = await forkOf('viewer');
    const viewer = path.join(setup.live, 'viewer.ts');
    await edit(viewer, 'const renderingCanvas = api.render.gl.context?.canvas;', 'const renderingCanvas = api.render.gl.context?.canvas ?? document.querySelector(\'#gl\'); // mine');
    const before = await readFile(viewer, 'utf8');
    expect(await updateFork(setup)).toEqual({ kind: 'needs-agent', reason: 'conflict', conflicts: ['viewer.ts'] });
    expect(await readFile(viewer, 'utf8')).toBe(before);
    expect(await readFile(path.join(setup.live, '.forked-from/viewer.ts'), 'utf8')).toBe(await readFile(path.join(SHIPPED, 'viewer/viewer.ts'), 'utf8'));
  }, 120_000);

  it('keymap-default, no conflict: the built-in\'s change merges with the user\'s binding', async () => {
    const setup = await forkOf('keymap-default');
    const keymap = path.join(setup.live, 'keymap.ts');
    const text = await readFile(keymap, 'utf8');
    // The user's own binding, appended to the shipped map.
    const marker = text.lastIndexOf('];');
    expect(marker).toBeGreaterThan(0);
    const chord = ['Ctrl', 'Alt', 'Shift', 'F12'].join('+');
    const binding = `{ ${'key'}: ${JSON.stringify(chord)}, command: 'view.fit' }, // mine`;
    await writeFile(keymap, `${text.slice(0, marker)}  ${binding}\n${text.slice(marker)}`);
    const result = await updateFork(setup);
    expect(result).toMatchObject({ kind: 'updated' });
    const merged = await readFile(keymap, 'utf8');
    expect(merged).toContain(binding);
    // Every line the built-in has now is in the merge.
    const builtin = await readFile(path.join(BUILTINS, 'keymap-default/keymap.ts'), 'utf8');
    for (const line of builtin.split('\n').filter((l) => l.includes('command:'))) expect(merged).toContain(line);
  }, 120_000);
});
