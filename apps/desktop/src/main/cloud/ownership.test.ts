import { describe, expect, it } from 'vitest';
import type { MeDto } from '@powermove/registry/wire';

import { isMine, ownerOf } from './ownership';
import type { ProvenanceRecord } from './provenance';

const ME = '66666666-6666-4666-8666-666666666666';
const OTHER = '77777777-7777-4777-8777-777777777777';

function me(publisherId: string | null): MeDto {
  return {
    user: { id: '55555555-5555-4555-8555-555555555555', name: null, email: 'jude@example.com', image: null },
    publisher: publisherId ? { id: publisherId, handle: 'jude', tombstoned: false } : null,
    settings: { rememberInstalls: true }
  };
}

const origin = (owner: string): NonNullable<ProvenanceRecord['origin']> => ({
  repoId: 'r', releaseId: 'x', coordinate: 'mara/glass-blur', version: '1.0.0', treeSha: 't', commitSha: 'c', ownerPublisherId: owner
});
const published = (owner: string): NonNullable<ProvenanceRecord['published']> => ({ repoId: 'p', releaseId: 'y', version: '1.0.0', ownerPublisherId: owner });

describe('ownership', () => {
  it('prefers the published owner over the origin owner', () => {
    expect(ownerOf({ origin: origin(OTHER), published: published(ME) })).toBe(ME);
    expect(ownerOf({ origin: origin(OTHER) })).toBe(OTHER);
    expect(ownerOf({})).toBeNull();
    expect(ownerOf(null)).toBeNull();
  });

  it('is mine only when the owner is the signed-in publisher', () => {
    expect(isMine({ origin: origin(OTHER), published: published(ME) }, me(ME))).toBe(true);
    expect(isMine({ origin: origin(ME) }, me(ME))).toBe(true);
    expect(isMine({ origin: origin(OTHER) }, me(ME))).toBe(false);
    expect(isMine({ published: published(ME) }, me(OTHER))).toBe(false);
  });

  it('makes nothing mine when signed out or without a handle', () => {
    expect(isMine({ published: published(ME) }, null)).toBe(false);
    expect(isMine({ published: published(ME) }, me(null))).toBe(false);
    expect(isMine({}, me(ME))).toBe(false);
  });
});
