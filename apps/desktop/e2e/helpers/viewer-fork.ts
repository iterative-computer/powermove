/*
 * User forks of the built-in viewer for the appendix C consumer matrix.
 *
 * A user who forked or overrode the viewer keeps that copy across an update,
 * so every consumer surface is checked with the current viewer and with
 * forks of viewers users can actually have: the viewers shipped in released
 * desktop builds, from before the engine (apiVersion 1, no engine
 * presentation of their own; they draw through `render.gl.init` and the
 * engine binds to their canvas). The engine-era viewer stands for a fork
 * made after this update (it presents by itself).
 *
 * Refs are commit ids, so the fork does not move with any branch.
 */
import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import type { Page } from 'playwright';
import { expect, repoRoot } from './app';

const exec = promisify(execFile);

export type ViewerVariant = {
  /** Shown in test titles. */
  name: string;
  /** Commit whose `apps/desktop/src/extensions/viewer/` the fork copies; null is the current viewer. */
  ref: string | null;
};

/** The current viewer, then forks of the viewers users can have. */
export const VIEWER_VARIANTS: readonly ViewerVariant[] = [
  { name: 'current viewer', ref: null },
  // Desktop v1.1.0 (tag `v1.1.0`): viewer 1.0.8, apiVersion 1.
  { name: 'fork of shipped viewer 1.0.8', ref: 'ded961354' },
  // Desktop v1.2.0 (tag `v1.2.0`): viewer 1.0.18, apiVersion 1.
  { name: 'fork of shipped viewer 1.0.18', ref: '07ed5fa17' },
  // An engine-era viewer, as a fork made after the update would be.
  { name: 'fork of an engine-era viewer', ref: '8d772dac8' },
];

/** The forks alone (for rows whose current-viewer case lives elsewhere). */
export const VIEWER_FORKS = VIEWER_VARIANTS.filter(variant => variant.ref);

export const VIEWER_FORK_ID = 'consumer-viewer-fork';

/** Copies the viewer at `ref` into the user's extensions as a fork that
 *  replaces the built-in viewer, the way a user's fork is stored. */
export async function installViewerFork(userData: string, ref: string, id = VIEWER_FORK_ID): Promise<void> {
  const prefix = 'apps/desktop/src/extensions/viewer/';
  const gitRoot = path.resolve(repoRoot, '../..');
  const { stdout } = await exec('git', ['ls-tree', '-r', '--name-only', ref, '--', prefix], { cwd: gitRoot });
  const root = path.join(userData, 'extensions', id);
  // Publish the whole fork at once; the loader must never see a manifest
  // while its source tree is only partly copied.
  const staging = await mkdtemp(path.join(userData, 'consumer-viewer-'));
  try {
    for (const file of stdout.trim().split('\n').filter(file => file && !file.endsWith('.test.ts'))) {
      const { stdout: source } = await exec('git', ['show', `${ref}:${file}`], { cwd: gitRoot, maxBuffer: 8 * 1024 * 1024 });
      const destination = path.join(staging, file.slice(prefix.length));
      await mkdir(path.dirname(destination), { recursive: true });
      await writeFile(destination, source);
    }
    const { stdout: manifest } = await exec('git', ['show', `${ref}:${prefix}manifest.json`], { cwd: gitRoot });
    const parsed = JSON.parse(manifest);
    await writeFile(path.join(staging, 'manifest.json'), JSON.stringify({ ...parsed, id, replaces: ['viewer'], forkedFrom: `viewer@${parsed.version}` }));
    await mkdir(path.dirname(root), { recursive: true });
    await rename(staging, root);
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

/** Installs the variant's fork (if any) and waits until it is the active viewer. */
export async function useViewer(page: Page, userData: string, variant: ViewerVariant): Promise<void> {
  if (!variant.ref) return;
  await installViewerFork(userData, variant.ref);
  // Active, or the loader's reason it is not (a fork that fails to activate
  // is a user's viewer this build breaks, reported as such).
  const state = () => page.evaluate((id) => {
    const loader = (window as any).PM.Kernel.loader;
    if (loader.activeIds().includes(id)) return 'active';
    const health = loader.records().find((record: any) => record.id === id)?.health;
    return health?.error ? `${health.state}: ${health.error}` : 'pending';
  }, VIEWER_FORK_ID);
  await expect.poll(state, { timeout: 30_000 }).not.toBe('pending');
  expect(await state(), `${variant.name} activates`).toBe('active');
  // The fork's panel replaced the built-in one in the dock.
  await expect.poll(() => page.evaluate(() => !!(window as any).PM.panelInst?.viewer?.el?.querySelector('#stage')), { timeout: 30_000 }).toBe(true);
}
