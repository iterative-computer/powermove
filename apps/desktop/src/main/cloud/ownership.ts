/*
 * Whose is this folder? (store plan §2.2, P0 amendments.)
 *
 * Ownership is never stored: it is derived at read time from the publisher
 * that owns the folder's `published` repo, or failing that its `origin` repo,
 * against the signed-in account's cached publisher id. Switching accounts or
 * signing out therefore cannot leave a stale "yours" behind: signed out,
 * nothing is mine.
 */
import type { MeDto } from '@powermove/registry/wire';

import type { ProvenanceRecord } from './provenance';

/** The publisher that owns the folder's repo, as recorded at publish or install. */
export function ownerOf(record: Pick<ProvenanceRecord, 'published' | 'origin'> | null | undefined): string | null {
  return record?.published?.ownerPublisherId ?? record?.origin?.ownerPublisherId ?? null;
}

export function isMine(record: Pick<ProvenanceRecord, 'published' | 'origin'> | null | undefined, me: MeDto | null): boolean {
  const mine = me?.publisher?.id ?? null;
  if (mine === null) return false;
  return ownerOf(record) === mine;
}
