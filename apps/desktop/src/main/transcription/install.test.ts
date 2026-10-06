import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({}));

import { warmOnce } from './install';

let dir = '';
beforeEach(async () => { dir = await mkdtemp(path.join(os.tmpdir(), 'pm-warm-')); });
afterEach(async () => { await rm(dir, { recursive: true, force: true }); });

describe('the launch warm-up', () => {
  it('runs once per key, and only records a load that landed', async () => {
    const marker = path.join(dir, 'transcription-warm.json');
    const warm = vi.fn(async () => false);
    await warmOnce({ warm }, marker, '1.2.0');
    expect(warm).toHaveBeenCalledWith({ release: true });
    // Nothing loaded (no model yet, or a job was loading one): try again next launch.
    await expect(readFile(marker, 'utf8')).rejects.toThrow();
    warm.mockResolvedValue(true);
    await warmOnce({ warm }, marker, '1.2.0');
    expect(JSON.parse(await readFile(marker, 'utf8'))).toEqual({ key: '1.2.0' });
    await warmOnce({ warm }, marker, '1.2.0');
    expect(warm).toHaveBeenCalledTimes(2);
    // A new version warms again.
    await warmOnce({ warm }, marker, '1.3.0');
    expect(warm).toHaveBeenCalledTimes(3);
  });
});
