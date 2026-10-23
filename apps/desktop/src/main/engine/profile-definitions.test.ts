import { mkdtemp, readFile, readdir, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { DefinitionRelay } from './definitions';
import { ProfileDefinitionsWriter, profileDefinitionsPath } from './profile-definitions';

const effect = (id: string, body: string) => ({ kind: 'effect', id, origin: 'user', glsl: { source: body } }) as never;

it('keeps the window\'s winning set as a pm-cli definitions snapshot, owner-only and coalesced', async () => {
  const profile = await mkdtemp(path.join(os.tmpdir(), 'pm-profile-defs-'));
  try {
    const relay = new DefinitionRelay();
    const writer = new ProfileDefinitionsWriter(profileDefinitionsPath(profile), () => {}, 10_000);
    relay.update(7, { generation: 1, full: true, upserts: [effect('glow', 'a'), effect('invert', 'fork')], removals: [] });
    writer.write(relay.snapshotSet(7)!);
    relay.update(7, { generation: 2, full: false, upserts: [], removals: [{ kind: 'effect', id: 'glow' }] });
    writer.write(relay.snapshotSet(7)!);
    await writer.flush();
    const file = profileDefinitionsPath(profile);
    // Only the latest set reaches the disk: what pm-cli reads as DefinitionSetSnapshot.
    expect(JSON.parse(await readFile(file, 'utf8'))).toEqual({ generation: 2, definitions: [effect('invert', 'fork')], missing: ['glow'] });
    expect((await stat(file)).mode & 0o777).toBe(0o600);
    expect((await readdir(path.dirname(file))).filter(name => name.endsWith('.tmp'))).toEqual([]);
    expect(relay.snapshotSet(8)).toBeNull();
  } finally { await rm(profile, { recursive: true, force: true }); }
});
