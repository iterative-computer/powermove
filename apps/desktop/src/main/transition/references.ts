import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { decodeStorageJSON } from '../storage-codec';
import type { ProxyOf } from '../../shared/transition';
/** Supplemental provenance, never a proof of complete history/reference closure.
 * Packed histories, partial patches and unknown owner formats stay unclassified. */
export async function proxyReferences(directory: string): Promise<Record<string, ProxyOf[]>> {
  const output: Record<string, ProxyOf[]> = Object.create(null);
  const visit = (value: any) => {
    if (!value || typeof value !== 'object') return;
    if (value.playbackProxy || value.playbackProxyVersion > 0) {
      // Legacy projects can resolve their bytes through the bare-id fallback.
      // Preserve the binding under both identities, just as the reader does.
      for (const key of new Set([value.storageKey, value.id])) if (typeof key === 'string') {
        (output[key] ??= []).push({ fingerprint: typeof value.fingerprint === 'string' ? value.fingerprint : null,
          sourcePath: typeof value.sourcePath === 'string' ? value.sourcePath : null });
      }
    }
    for (const child of Object.values(value)) visit(child);
  };
  for (const name of await readdir(directory)) {
    if (!name.endsWith('.json')) continue;
    const { value } = decodeStorageJSON(await readFile(path.join(directory, name), 'utf8'), 768 * 1024 ** 2);
    visit(value);
  }
  return output;
}
