import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { NEW_ENGINE_ENV, NEW_ENGINE_STORE_KEY, newEngineEnabled } from '../main/engine/flag';
import { parseStoreKey, storeFileName } from '../shared/store-keys';

/** The Chromium-only host cannot satisfy the native-engine contract. Refuse
 * it explicitly rather than silently rendering an enabled profile in WebGL. */
export async function requireHiddenHostRuntime(userData: string, env: NodeJS.ProcessEnv = process.env, platform: NodeJS.Platform = process.platform): Promise<void> {
  if (platform !== 'darwin') return; // Linux retains Chromium through cutover.
  let stored: unknown = false;
  if (!['0', 'false', '1', 'true'].includes(env[NEW_ENGINE_ENV] ?? '')) {
    const parsed = parseStoreKey(NEW_ENGINE_STORE_KEY);
    if (!parsed) throw new Error('Invalid native engine setting key.');
    try { stored = JSON.parse(await readFile(path.join(userData, 'store', storeFileName(parsed)), 'utf8')); }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw new Error(`native-engine-setting-unreadable: ${(error as Error).message}`);
    }
  }
  if (newEngineEnabled(env, stored)) {
    throw new Error('native-hidden-host-unavailable: This CLI build has no native hidden-host render service. Connect to a running native desktop with powermove mcp --connect --user-data <desktop profile>. The hidden host cannot honor the enabled engine setting or per-machine shader pins, so it did not start the Chromium-only renderer.');
  }
}
