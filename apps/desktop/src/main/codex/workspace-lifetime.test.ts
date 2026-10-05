import { lutimes, mkdir, mkdtemp, readdir, rm, symlink, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { forgetProjectWorkspace, sweepOrphanWorkspaces } from './workspace-lifetime';

const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.UTC(2026, 9, 4);

let userData: string;
const workspaces = () => path.join(userData, 'Agent Workspaces');
const names = async (directory: string) => (await readdir(directory).catch(() => [])).sort();

/** A workspace with one artifact, every entry last changed `ageDays` ago. */
async function workspace(id: string, ageDays: number): Promise<void> {
  const seconds = (NOW - ageDays * DAY) / 1000;
  const run = path.join(workspaces(), id, 'artifacts', 'run');
  await mkdir(run, { recursive: true });
  await writeFile(path.join(run, 'logo.svg'), '<svg/>');
  const state = path.join(userData, 'Agent State', id);
  await mkdir(state, { recursive: true });
  await writeFile(path.join(state, 'session-v2-project.txt'), 'thread');
  for (const entry of [path.join(run, 'logo.svg'), run, path.dirname(run), path.join(workspaces(), id)]) {
    await utimes(entry, seconds, seconds);
  }
}

beforeEach(async () => { userData = await mkdtemp(path.join(tmpdir(), 'pm-workspace-lifetime-')); });
afterEach(async () => { await rm(userData, { recursive: true, force: true }); });

describe('forgetProjectWorkspace', () => {
  it('removes the project workspace and its agent sessions only', async () => {
    await workspace('Pgone', 0);
    await workspace('Pkept', 0);
    await forgetProjectWorkspace(userData, 'Pgone');
    expect(await names(workspaces())).toEqual(['Pkept']);
    expect(await names(path.join(userData, 'Agent State'))).toEqual(['Pkept']);
  });

  it('never removes the app agent workspace', async () => {
    await workspace('powermove-global', 0);
    await forgetProjectWorkspace(userData, 'powermove-global');
    expect(await names(workspaces())).toEqual(['powermove-global']);
  });

  it('removes a link planted in the workspace without following it', async () => {
    const outside = path.join(userData, 'outside');
    await mkdir(outside);
    await writeFile(path.join(outside, 'keep.txt'), 'keep');
    await workspace('Pgone', 0);
    await symlink(outside, path.join(workspaces(), 'Pgone', 'artifacts', 'link'));
    await forgetProjectWorkspace(userData, 'Pgone');
    expect(await names(workspaces())).toEqual([]);
    expect(await names(outside)).toEqual(['keep.txt']);
  });
});

describe('sweepOrphanWorkspaces', () => {
  it('removes idle workspaces whose project is gone and keeps everything else', async () => {
    await workspace('Plive', 90);
    await workspace('Porphan', 30);
    await workspace('Precent', 2);
    await workspace('Pbusy', 30);
    await workspace('powermove-global', 90);
    const removed = await sweepOrphanWorkspaces(userData, ['Plive'], new Set(['Pbusy']), NOW);
    expect(removed).toEqual(['Porphan']);
    expect(await names(workspaces())).toEqual(['Pbusy', 'Plive', 'Precent', 'powermove-global']);
    expect(await names(path.join(userData, 'Agent State'))).not.toContain('Porphan');
  });

  it('treats a recent file deep in an old workspace as activity', async () => {
    await workspace('Porphan', 30);
    const fresh = path.join(workspaces(), 'Porphan', 'artifacts', 'run', 'new.png');
    await writeFile(fresh, 'x');
    await utimes(fresh, (NOW - DAY) / 1000, (NOW - DAY) / 1000);
    expect(await sweepOrphanWorkspaces(userData, ['Plive'], new Set(), NOW)).toEqual([]);
  });

  it('finishes removals a crash interrupted', async () => {
    await mkdir(path.join(workspaces(), '.removing-abc', 'artifacts'), { recursive: true });
    await workspace('Plive', 0);
    await sweepOrphanWorkspaces(userData, ['Plive'], new Set(), NOW);
    expect(await names(workspaces())).toEqual(['Plive']);
  });

  it('ignores a top-level link', async () => {
    const outside = path.join(userData, 'outside');
    await mkdir(outside);
    await mkdir(workspaces(), { recursive: true });
    const link = path.join(workspaces(), 'Plinked');
    await symlink(outside, link);
    await lutimes(link, 1_000_000_000, 1_000_000_000);
    await sweepOrphanWorkspaces(userData, ['Plive'], new Set(), NOW);
    expect(await names(workspaces())).toEqual(['Plinked']);
  });
});
