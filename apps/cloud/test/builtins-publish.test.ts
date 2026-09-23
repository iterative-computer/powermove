import { expect, test } from 'bun:test';
import { readdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { walkDir } from '@powermove/registry/node';
import { publishBuiltins, type BuiltinPlan } from '../../desktop/scripts/lib/publish-builtins';
import { createApp } from '../src/app';
import { extensions, releases, repos } from '../src/db/schema';
import { withData } from './db';
import { makeEnv } from './env';
import { publisher } from './publish-fixture';

const root = resolve(import.meta.dir, '../../desktop/src/extensions');
const expectedCategories: Record<string, BuiltinPlan['category']> = {
  'effects-basic': 'effects', inspector: 'panels', 'keymap-default': 'commands',
  'layers-3d': 'layers', mods: 'panels', 'theme-default': 'themes', timeline: 'panels',
  toolbar: 'panels', 'transitions-basic': 'transitions', viewer: 'panels'
};

test('publishes ten built-ins, skips unchanged versions, and requires a bump for changed trees', () => withData(async data => {
  const env = makeEnv(data);
  const actor = await publisher(data, env, 'powermove');
  const app = createApp({ data: () => data });
  const dirs = (await readdir(root, { withFileTypes: true }))
    .filter(entry => entry.isDirectory() && !entry.name.startsWith('.'))
    .map(entry => resolve(root, entry.name)).sort();
  const fetcher = async (input: string | URL | Request, init?: RequestInit) => app.request(new URL(String(input)).pathname, init, env);
  const options = { fetch: fetcher, dirs, token: actor.token, origin: 'https://cloud.test', appVersion: '1.0.1' };

  const first = await publishBuiltins(options);
  expect(first).toHaveLength(10);
  expect(first.every(plan => plan.status === 'published')).toBe(true);
  expect(Object.fromEntries(first.map(plan => [plan.coordinate.split('/')[1], plan.category]))).toEqual(expectedCategories);
  const repoRows = await data.db.select().from(repos);
  expect(repoRows).toHaveLength(10);
  expect(repoRows.every(repo => repo.ownerId === actor.publisher.id)).toBe(true);
  const listings = await data.db.select().from(extensions);
  expect(listings).toHaveLength(10);
  for (const repo of repoRows) {
    const listing = listings.find(item => item.repoId === repo.id)!;
    expect(listing.handle).toBe('powermove');
    expect(listing.category).toBe(expectedCategories[repo.slug]);
    expect(listing.licence).toBe('AGPL-3.0-or-later');
  }

  const second = await publishBuiltins(options);
  expect(second).toHaveLength(10);
  expect(second.every(plan => plan.status === 'skipped')).toBe(true);
  expect(await data.db.select().from(releases)).toHaveLength(10);

  const originalReadDir = walkDir;
  await expect(publishBuiltins({ ...options, dirs: [resolve(root, 'timeline')], readDir: async dir => {
    const files = await originalReadDir(dir);
    return files.map(file => file.path === 'manifest.json' ? file : file.path === 'index.ts'
      ? { ...file, bytes: new TextEncoder().encode(new TextDecoder().decode(file.bytes) + '\n// changed\n') }
      : file);
  } })).rejects.toThrow('bump the version in manifest.json');
  expect(await data.db.select().from(releases)).toHaveLength(10);
}));
