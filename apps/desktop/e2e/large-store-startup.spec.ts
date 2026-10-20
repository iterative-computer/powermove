import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { expect, test, launchApp } from './helpers/app';

test('opens with a large recovery tree and preserves every record', async () => {
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-large-startup-'));
  const store = path.join(userData, 'store');
  await mkdir(store);
  const records = Array.from({ length: 20_000 }, (_, index) => ({
    id: `saved-${index}`, channels: Array.from({ length: 8 }, (_, channel) => ({
      keys: [{ t: index / 30, v: channel, ease: { type: 'bezier', points: [0, 0, 1, 1] } }],
    })),
  }));
  await writeFile(path.join(store, 'projectHistory.bulk.json'), JSON.stringify({ records }));
  const digest = createHash('sha256').update(JSON.stringify(records)).digest('hex');
  const started = performance.now();
  const session = await launchApp({ userData });
  try {
    const restored = await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      const records = PM.store.get('projectHistory.bulk').records;
      const bytes = new TextEncoder().encode(JSON.stringify(records));
      const hash = await crypto.subtle.digest('SHA-256', bytes);
      const digest = Array.from(new Uint8Array(hash), byte => byte.toString(16).padStart(2, '0')).join('');
      return { count: records.length, last: records.at(-1), digest };
    });
    expect(restored).toEqual({ count: records.length, last: records.at(-1), digest });
    expect(session.diagnostics.pageErrors).toEqual([]);
    console.log(`Large recovery startup verified in ${Math.round(performance.now() - started)}ms`);
  } finally {
    await session.close();
    await rm(userData, { recursive: true, force: true });
  }
});
