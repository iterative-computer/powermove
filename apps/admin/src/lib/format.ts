import type { AdminExtension } from '@powermove/registry/wire';

// Server-rendered in UTC so the page reads the same wherever the Worker runs.
const day = new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeZone: 'UTC' });
const moment = new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeStyle: 'short', timeZone: 'UTC' });
export const date = (iso: string) => day.format(new Date(iso));
export const time = (iso: string) => `${moment.format(new Date(iso))} UTC`;

export function extensionState(e: AdminExtension): string {
  if (e.moderation === 'removed') return 'Removed';
  if (e.tombstoned) return 'Deleted by owner';
  if (e.moderation === 'hidden') return 'Hidden';
  return e.visibility === 'unlisted' ? 'Unlisted' : 'Listed';
}

const ACTIONS: Record<string, string> = {
  hide: 'Hidden', unhide: 'Restored', remove: 'Removed', yank: 'Version withdrawn', tombstone: 'Deleted by owner',
  verify: 'Verified', unverify: 'Unverified', grant_admin: 'Admin granted', revoke_admin: 'Admin revoked', unblock_aero: 'Aero unblocked',
};
export const actionLabel = (action: string) => ACTIONS[action] ?? action;
