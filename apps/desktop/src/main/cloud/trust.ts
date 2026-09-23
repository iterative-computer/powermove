/*
 * Trust levels (extension sandbox design §2): who wrote an extension, and
 * therefore where it runs.
 *
 *   builtin        ships in the app
 *   local          made or forked on this Mac, by the user or their agent
 *   store          installed from the Store: someone else's code
 *   store-trusted  a store install the user gave full access, behind the
 *                  native dialog below; recorded in provenance
 *
 * Like ownership, the level is derived at read time and never stored: a
 * release I published and installed here is mine while I'm signed in as its
 * owner, and someone else's otherwise.
 */
import type { MessageBoxOptions } from 'electron';
import type { MeDto } from '@powermove/registry/wire';

import type { ExtensionRecord, TrustLevel } from '../../shared/extensions';
import { isMine } from './ownership';
import type { ProvenanceRecord } from './provenance';

export type { TrustLevel };

export function trustLevelFor(
  record: Pick<ExtensionRecord, 'scope'>,
  provenance: Pick<ProvenanceRecord, 'origin' | 'published' | 'trusted'> | null | undefined,
  me: MeDto | null
): TrustLevel {
  if (record.scope === 'builtin') return 'builtin';
  // Project folders and folders without a store origin were made here.
  if (record.scope !== 'user' || !provenance?.origin) return 'local';
  if (isMine(provenance, me)) return 'local';
  return provenance.trusted ? 'store-trusted' : 'store';
}

/** The native dialog `store:trust` shows before it writes anything. Cancel is the default. */
export function trustDialog(name: string): MessageBoxOptions {
  return {
    type: 'warning',
    message: `Give ${name} full access to Powermove?`,
    detail: 'It will run with the same access as the app: your projects, files you open, the network, and other extensions’ values. Only trust extensions from people you know. You can revoke this from the Library.',
    buttons: ['Trust', 'Cancel'],
    defaultId: 1,
    cancelId: 1,
    noLink: true
  };
}
