import { readFile } from 'node:fs/promises';
import path from 'node:path';
import type { TRStatus } from '../../shared/transition';

/** Signed-bundle configuration only. This module never contacts a feed or
 * modifies the maintenance updater. The owner must supply the production URL. */
export type CutoverConfig = { version: 1; enabled: boolean; feedURL: string | null };
export async function readCutoverConfig(resources: string): Promise<CutoverConfig> {
  const disabled: CutoverConfig = { version: 1, enabled: false, feedURL: null };
  let value: CutoverConfig;
  try { value = JSON.parse(await readFile(path.join(resources, 'transition', 'cutover.json'), 'utf8')); }
  catch (error: any) { if (error.code === 'ENOENT') return disabled; throw new Error('Invalid transition feed configuration.'); }
  if (value?.version !== 1 || typeof value.enabled !== 'boolean'
    || (value.feedURL !== null && typeof value.feedURL !== 'string')) throw new Error('Invalid transition feed configuration.');
  if (value.feedURL !== null) {
    const url = new URL(value.feedURL);
    if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash)
      throw new Error('Transition feed must use a dedicated HTTPS URL.');
  }
  if (value.enabled && !value.feedURL) throw new Error('Enabled transition feed requires a URL.');
  return value;
}

/** Eligibility is advisory until the signed installation/restore gates pass.
 * Darwin 23 is macOS 14. A complete TR copy alone never approves cutover (N4). */
export function cutoverFeed(config: CutoverConfig, status: TRStatus, gate: {
  platform: string; darwinMajor: number; ownerApproved: boolean; signedMigrationVerified: boolean;
}): string | null {
  return config.enabled && config.feedURL && gate.platform === 'darwin' && gate.darwinMajor >= 23
    && gate.ownerApproved && gate.signedMigrationVerified && status.state === 'complete' && status.cutoverReady
    && status.mediaComplete && status.localStorageComplete && status.secretsComplete
    ? config.feedURL : null;
}
