import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { NEW_ENGINE_ENV, NEW_ENGINE_STORE_KEY, newEngineEnabled } from '../main/engine/flag';
import { parseStoreKey, storeFileName } from '../shared/store-keys';

/** Read the desktop launch setting; Linux retains its Chromium host. */
export async function nativeHiddenHostEnabled(userData: string, env: NodeJS.ProcessEnv = process.env, platform: NodeJS.Platform = process.platform): Promise<boolean> {
  if (platform !== 'darwin') return false;
  let stored: unknown = false;
  if (!['0', 'false', '1', 'true'].includes(env[NEW_ENGINE_ENV] ?? '')) {
    const parsed = parseStoreKey(NEW_ENGINE_STORE_KEY);
    if (!parsed) throw new Error('Invalid native engine setting key.');
    try { stored = JSON.parse(await readFile(path.join(userData, 'store', storeFileName(parsed)), 'utf8')); }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw new Error(`native-engine-setting-unreadable: ${(error as Error).message}`);
    }
  }
  return newEngineEnabled(env, stored);
}
