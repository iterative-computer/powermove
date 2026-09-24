/*
 * Settings › Advanced › Registry URL: which Powermove Cloud this Mac talks to.
 *
 * Stored by main in `<userData>/cloud/registry.json`, never in the renderer's
 * store: the bearer session is bound to the registry origin, so pointing the
 * app at another registry is a trust decision the renderer cannot make on
 * its own (store plan §2.6). A change goes through `confirmRegistryChange`,
 * a native dialog that shows the exact origin being switched to.
 *
 * P10 adds the Settings UI; this module is the stored value and the gate.
 */
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { app, type MessageBoxOptions } from 'electron';
import { z } from 'zod';

import { normalizeOrigin } from './client';

export const DEFAULT_REGISTRY_ORIGIN = 'https://cloud.trypowermove.com';

const Stored = z.object({ origin: z.string().max(2000) });

export interface RegistryUrlSetting {
  /** The configured origin, or the default. */
  get(): string;
  load(): Promise<string>;
  /** Persist without asking; callers go through `confirmRegistryChange`. */
  set(origin: string): Promise<string>;
  fromEnvironment(): boolean;
}

export function createRegistryUrlSetting(dir: string): RegistryUrlSetting {
  const file = path.join(dir, 'registry.json');
  let current = DEFAULT_REGISTRY_ORIGIN;
  const override = !app?.isPackaged && process.env.POWERMOVE_REGISTRY_URL
    ? normalizeOrigin(process.env.POWERMOVE_REGISTRY_URL)
    : null;
  return {
    get: () => override ?? current,
    fromEnvironment: () => override !== null,
    async load() {
      try {
        const parsed = Stored.safeParse(JSON.parse(await readFile(file, 'utf8')));
        current = parsed.success ? normalizeOrigin(parsed.data.origin) : DEFAULT_REGISTRY_ORIGIN;
      } catch {
        current = DEFAULT_REGISTRY_ORIGIN;
      }
      return override ?? current;
    },
    async set(origin) {
      const next = normalizeOrigin(origin);
      await mkdir(dir, { recursive: true, mode: 0o700 });
      await writeFile(`${file}.tmp`, JSON.stringify({ origin: next }), { mode: 0o600 });
      await rename(`${file}.tmp`, file);
      current = next;
      return override ?? next;
    }
  };
}

export interface ConfirmRegistryChangeOptions {
  setting: RegistryUrlSetting;
  showMessageBox(options: MessageBoxOptions): Promise<{ response: number }>;
  /** Runs after the user confirms and before the new origin is stored: sign out of the old registry. */
  beforeChange?(): Promise<void>;
}

/**
 * The main-owned confirmation for a Registry URL change. The dialog names the
 * concrete origin; the value stored is the one shown, never a re-read.
 * Returns whether the change was made.
 */
export async function confirmRegistryChange(origin: string, options: ConfirmRegistryChangeOptions): Promise<boolean> {
  if (options.setting.fromEnvironment()) return false;
  const next = normalizeOrigin(origin);
  if (next === options.setting.get()) return false;
  const isDefault = next === DEFAULT_REGISTRY_ORIGIN;
  const { response } = await options.showMessageBox({
    type: 'warning',
    message: isDefault ? 'Use Powermove Cloud again?' : `Use the registry at ${next}?`,
    detail: isDefault
      ? 'You’ll be signed out, and the Store will show extensions from Powermove Cloud.'
      : 'You’ll be signed out. The Store will show extensions from this registry, and signing in sends your account details to it. Only use a registry you trust.',
    buttons: [isDefault ? 'Use Powermove Cloud' : 'Use This Registry', 'Cancel'],
    defaultId: 1,
    cancelId: 1,
    noLink: true
  });
  if (response !== 0) return false;
  await options.beforeChange?.();
  await options.setting.set(next);
  return true;
}
