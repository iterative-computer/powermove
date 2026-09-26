/* The extension contract lives in @powermove/registry so the desktop and the
   cloud share one parser. Kept as a shim so existing imports do not move. */
import type { ExtensionRecord } from '@powermove/registry/manifest';

export * from '@powermove/registry/manifest';

/** Who wrote an extension (sandbox design §2). */
export type TrustLevel = NonNullable<ExtensionRecord['trust']>;

/**
 * Someone else's code that declares `full-access` runs only once the user
 * has trusted it. Local extensions ignore permissions; built-ins are ours.
 */
export function needsTrust(record: Pick<ExtensionRecord, 'trust' | 'manifest'>): boolean {
  return record.trust === 'store' && (record.manifest?.permissions ?? []).includes('full-access');
}
