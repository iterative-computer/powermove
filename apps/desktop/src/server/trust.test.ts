import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { expect, it, vi } from 'vitest';
vi.mock('electron', () => import('./electron-stub'));
import { serveTrustFor } from './index';
import { STORE_MARKER } from '../main/cloud/trust';

it('keeps a marked Store extension sandboxed when serve has no provenance', async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'powermove-serve-trust-'));
  try {
    const folder = path.join(dir, 'extensions', 'third-party');
    await mkdir(folder, { recursive: true });
    await writeFile(path.join(folder, STORE_MARKER), JSON.stringify({ repoId: 'repo', releaseId: 'release' }));
    const trust = serveTrustFor(dir);
    expect(await trust('third-party', 'user')).toBe('store');
    expect(await trust('my-extension', 'user')).toBe('local');
    expect(await trust('third-party', 'project')).toBe('local');
  } finally { await rm(dir, { recursive: true, force: true }); }
});
