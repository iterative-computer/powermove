import { describe, expect, it } from 'vitest';
import type { MeDto } from '@powermove/registry/wire';

import type { ExtensionRecord } from '../../shared/extensions';
import type { ProvenanceRecord } from './provenance';
import { trustDialog, trustLevelFor } from './trust';

const MINE = '66666666-6666-4666-8666-666666666666';
const THEIRS = '22222222-2222-4222-8222-222222222222';
const REPO = '11111111-1111-4111-8111-111111111111';

const me = (publisherId: string | null): MeDto => ({
  user: { id: '55555555-5555-4555-8555-555555555555', name: null, email: 'j@example.com', image: null },
  publisher: publisherId ? { id: publisherId, handle: 'jude', tombstoned: false } : null,
  settings: { rememberInstalls: true }
});
const origin = (owner: string): NonNullable<ProvenanceRecord['origin']> =>
  ({ repoId: REPO, releaseId: REPO, coordinate: 'mara/glass-blur', version: '1.0.0', treeSha: 't', commitSha: 'c', ownerPublisherId: owner });
const published = (owner: string): NonNullable<ProvenanceRecord['published']> =>
  ({ repoId: REPO, releaseId: REPO, version: '1.0.0', ownerPublisherId: owner });
const trusted = { at: '2026-09-23T00:00:00.000Z', permissions: ['full-access' as const] };

describe('trustLevelFor', () => {
  const matrix: Array<[string, ExtensionRecord['scope'], Partial<ProvenanceRecord> | null, MeDto | null, string]> = [
    ['built-in', 'builtin', null, me(MINE), 'builtin'],
    ['built-in, even with a stray provenance entry', 'builtin', { origin: origin(THEIRS) }, null, 'builtin'],
    ['made here, no provenance', 'user', null, null, 'local'],
    ['made here, values key only', 'user', { envKey: 'local:1' }, me(MINE), 'local'],
    ['project folder', 'project', { origin: origin(THEIRS) }, null, 'local'],
    ['someone else’s install', 'user', { origin: origin(THEIRS) }, me(MINE), 'store'],
    ['someone else’s install, signed out', 'user', { origin: origin(THEIRS) }, null, 'store'],
    ['someone else’s install, trusted', 'user', { origin: origin(THEIRS), trusted }, me(MINE), 'store-trusted'],
    ['my own release installed here', 'user', { origin: origin(MINE) }, me(MINE), 'local'],
    ['my own release, signed out', 'user', { origin: origin(MINE) }, null, 'store'],
    ['my own release, no handle yet', 'user', { origin: origin(MINE) }, me(null), 'store'],
    ['a fork I published of someone else’s', 'user', { origin: origin(THEIRS), published: published(MINE) }, me(MINE), 'local'],
    ['published only (no origin)', 'user', { published: published(MINE) }, null, 'local']
  ];
  for (const [label, scope, entry, account, expected] of matrix) {
    it(label, () => {
      const provenance = entry ? { localId: 'x', envKey: REPO, ...entry } as ProvenanceRecord : null;
      expect(trustLevelFor({ scope }, provenance, account)).toBe(expected);
    });
  }
});

describe('trustDialog', () => {
  it('names the extension and the risks, with Cancel as the default', () => {
    const options = trustDialog('Glass blur');
    expect(options.message).toBe('Give Glass blur full access to Powermove?');
    expect(options.detail).toContain('your projects, files you open, the network, and other extensions’ values');
    expect(options.detail).toContain('You can revoke this from the Library.');
    expect(options.buttons).toEqual(['Trust', 'Cancel']);
    expect(options.defaultId).toBe(1);
    expect(options.cancelId).toBe(1);
  });
});
