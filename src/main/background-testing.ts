import path from 'node:path';
import type { BrowserWindowConstructorOptions } from 'electron';

/** Refuse a hidden test launch unless it names a separate, absolute profile. */
export function backgroundTesting(env: NodeJS.ProcessEnv, liveProfile: string): boolean {
  if (env.POWERMOVE_BACKGROUND_TEST !== '1') return false;
  const profile = env.POWERMOVE_USER_DATA;
  if (!profile || !path.isAbsolute(profile) || path.resolve(profile) === path.resolve(liveProfile)) {
    throw new Error('Background tests require an isolated absolute POWERMOVE_USER_DATA directory.');
  }
  return true;
}

export function backgroundWindowOptions(background: boolean): BrowserWindowConstructorOptions {
  return background ? {
    show: false,
    skipTaskbar: true,
    focusable: false,
    webPreferences: { backgroundThrottling: false, devTools: false }
  } : {};
}
