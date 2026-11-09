import type { CloudFileState } from '../shared/ipc';

async function nativeState(file: string): Promise<CloudFileState> {
  if (process.platform !== 'darwin') return 'unknown';
  try {
    const module = await import('@powermove/macos-haptics');
    const native = module.default ?? module;
    return await native.cloudFileState(file);
  }
  catch (error: any) {
    // Standalone Node hosts may not ship the desktop addon.
    if (error.code === 'MODULE_NOT_FOUND' || error.code === 'ERR_MODULE_NOT_FOUND') return 'unknown';
    throw error;
  }
}

/** Metadata only: never open a placeholder's data fork on the main thread. */
export async function inspectFileCloudState(file: string, inspect = nativeState, timeoutMs = 10_000): Promise<CloudFileState> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([inspect(file), new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('This file is taking too long to respond. Check your cloud storage or network connection, then try again.')), timeoutMs);
    })]);
  } finally { clearTimeout(timer); }
}

/** Conversion/analysis must use the existing media download flow first. */
export async function requireLocalMedia(file: string, inspect = inspectFileCloudState): Promise<void> {
  const state = await inspect(file);
  if (state === 'icloud' || state === 'cloud') {
    throw new Error(`This file is stored in ${state === 'icloud' ? 'iCloud' : 'cloud storage'}. Download it in Finder, then try again.`);
  }
}
