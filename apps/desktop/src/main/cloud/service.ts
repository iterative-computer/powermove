/*
 * Wires the Powermove Cloud account into main: the Registry URL setting, the
 * session, sign-in and the `cloud:*` IPC. Called once after `app.whenReady()`
 * (safeStorage is only usable then).
 *
 * `powermove://` links can arrive before that (`open-url` fires before
 * `ready` on a cold start), so `createDeepLinkQueue` holds them until the
 * account is ready and then hands them over in order.
 */
import type { IpcMain, IpcMainInvokeEvent, MessageBoxOptions } from 'electron';
import path from 'node:path';
import type { ApiErrorBody, MeDto } from '@powermove/registry/wire';

import { CLOUD_IPC } from '../../shared/cloud-ipc';
import type { SafeStorageLike } from '../env/store';
import { createCloudAuth, DEEP_LINK_SCHEME, type CloudAuth } from './auth';
import { registerCloudIpc } from './ipc';
import { confirmRegistryChange, createRegistryUrlSetting, type RegistryUrlSetting } from './registry-url';
import { createCloudSession, type CloudSession } from './session';

export interface DeepLinkQueue {
  /** Deliver now when ready, otherwise hold. */
  push(url: string): void;
  /** Start delivering: drains everything held so far. */
  ready(handler: (url: string) => void): void;
}

export function createDeepLinkQueue(): DeepLinkQueue {
  const held: string[] = [];
  let handler: ((url: string) => void) | null = null;
  return {
    push(url) {
      if (handler) { handler(url); return; }
      // Only sign-in links are worth holding; when full, keep the newest.
      if (!/^powermove:\/\/auth\?/i.test(url)) return;
      if (held.length >= 8) held.shift();
      held.push(url);
    },
    ready(next) {
      handler = next;
      for (const url of held.splice(0)) next(url);
    }
  };
}

/** `powermove://…` arguments in a second instance's argv (Windows and Linux deliver links this way). */
export function deepLinksIn(argv: readonly string[]): string[] {
  return argv.filter((arg) => arg.toLowerCase().startsWith(`${DEEP_LINK_SCHEME}://`));
}

export const DELETE_ACCOUNT_DIALOG: MessageBoxOptions = {
  type: 'warning',
  message: 'Delete your Powermove account?',
  detail: 'Your published extensions stay on the Store under your handle. This can’t be undone.',
  buttons: ['Delete Account', 'Cancel'],
  defaultId: 1,
  cancelId: 1,
  noLink: true
};

export interface CloudServiceOptions {
  userData: string;
  appVersion: string;
  safeStorage(): SafeStorageLike;
  ipcMain: Pick<IpcMain, 'handle'>;
  isTrusted(event: Pick<IpcMainInvokeEvent, 'sender' | 'senderFrame'>): boolean;
  /** Send one event to every editor window. */
  broadcast(channel: string, payload: unknown): void;
  openExternal(url: string): Promise<void>;
  /** A native message box, attached to the focused window when there is one. */
  showMessageBox(options: MessageBoxOptions): Promise<{ response: number }>;
}

export interface CloudService {
  session: CloudSession;
  auth: CloudAuth;
  registry: RegistryUrlSetting;
  /** Settings › Advanced › Registry URL: main's confirmation, then sign out and switch. */
  confirmRegistryChange(origin: string): Promise<boolean>;
}

export async function startCloudService(options: CloudServiceOptions): Promise<CloudService> {
  const dir = path.join(options.userData, 'cloud');
  const registry = createRegistryUrlSetting(dir);
  await registry.load();
  const session = createCloudSession({
    dir,
    safeStorage: options.safeStorage,
    origin: () => registry.get(),
    appVersion: options.appVersion
  });
  await session.load();
  const notifyAccount = (me: MeDto | null): void => options.broadcast(CLOUD_IPC.accountChanged, me);
  const auth = createCloudAuth({
    session,
    openExternal: options.openExternal,
    notifyAccount,
    notifySignInFailed: (error: ApiErrorBody) => options.broadcast(CLOUD_IPC.signInFailed, error),
    confirmDelete: async () => (await options.showMessageBox(DELETE_ACCOUNT_DIALOG)).response === 0
  });
  const changeRegistry = async (origin: string): Promise<boolean> => {
    const changed = await confirmRegistryChange(origin, {
      setting: registry,
      showMessageBox: options.showMessageBox,
      // Through auth so a sign-in still waiting on the old registry is dropped too.
      beforeChange: async () => {
        await auth.signOut();
        await session.clear();
      }
    });
    if (changed) notifyAccount(null);
    return changed;
  };
  registerCloudIpc(options.ipcMain, {
    auth,
    session,
    registry: { get: () => registry.get(), fromEnvironment: () => registry.fromEnvironment(), change: changeRegistry },
    isTrusted: options.isTrusted
  });
  return {
    session,
    auth,
    registry,
    confirmRegistryChange: changeRegistry
  };
}
