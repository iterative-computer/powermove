/*
 * Which Powermove Cloud this computer talks to. A released build talks only to
 * Powermove Cloud: the Store, accounts and publishing are Powermove's
 * service, and another registry could serve updates to extensions installed
 * from ours. So a packaged build ignores `<userData>/cloud/registry.json`
 * and the environment, and refuses to change.
 *
 * Development builds (unpackaged) may still point elsewhere, for the local
 * cloud and tests: `POWERMOVE_REGISTRY_URL`, else the stored file. A change
 * still goes through `confirmRegistryChange`, a native dialog naming the
 * exact origin.
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

/** Released builds are locked to Powermove Cloud. */
export function registryLocked(): boolean {
  return app?.isPackaged === true;
}

export function createRegistryUrlSetting(dir: string, locked: () => boolean = registryLocked): RegistryUrlSetting {
  const file = path.join(dir, 'registry.json');
  let current = DEFAULT_REGISTRY_ORIGIN;
  const override = !locked() && process.env.POWERMOVE_REGISTRY_URL
    ? normalizeOrigin(process.env.POWERMOVE_REGISTRY_URL)
    : null;
  return {
    get: () => (locked() ? DEFAULT_REGISTRY_ORIGIN : override ?? current),
    fromEnvironment: () => override !== null,
    async load() {
      if (locked()) return DEFAULT_REGISTRY_ORIGIN;
      try {
        const parsed = Stored.safeParse(JSON.parse(await readFile(file, 'utf8')));
        current = parsed.success ? normalizeOrigin(parsed.data.origin) : DEFAULT_REGISTRY_ORIGIN;
      } catch {
        current = DEFAULT_REGISTRY_ORIGIN;
      }
      return override ?? current;
    },
    async set(origin) {
      if (locked()) return DEFAULT_REGISTRY_ORIGIN;
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
export async function confirmRegistryChange(origin: string, options: ConfirmRegistryChangeOptions & { locked?: () => boolean }): Promise<boolean> {
  if ((options.locked ?? registryLocked)() || options.setting.fromEnvironment()) return false;
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
