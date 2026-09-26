import { realpath, lstat, open } from 'node:fs/promises';
import { constants } from 'node:fs';
import path from 'node:path';
import type { IpcMain, IpcMainEvent, IpcMainInvokeEvent } from 'electron';

import { IpcValidationError, isRecord } from '../../shared/guards';
import { IPC, type ExtensionForkResult } from '../../shared/ipc';
import {
  EXTENSION_ID,
  EXT_IPC,
  MANIFEST_LIMITS,
  type ExtensionCreateRequest,
  type ExtensionHealthReport,
  type ExtensionIdRequest,
  type ExtensionRecord,
  type ExtensionSetEnabledRequest
} from '../../shared/extensions';
import type { ExtensionRegistry } from './registry';
import { forkBuiltinExtension } from './fork';

type ExtensionsIpcEvent = IpcMainEvent | IpcMainInvokeEvent;

let assetBuildDir: string | null = null;
let assetRegistry: ExtensionRegistry | null = null;

/** Open and inspect the same descriptor so a swapped symlink cannot redirect a bundle read. */
export async function readRegularBundle(candidate: string): Promise<Uint8Array | null> {
  try {
    const file = await open(candidate, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
      if (!(await file.stat()).isFile()) return null;
      return new Uint8Array(await file.readFile());
    } finally { await file.close(); }
  } catch { return null; }
}

/**
 * Main-owned record, never permissions supplied by the iframe URL. Store
 * installs run here; local extensions are served too so their author can run
 * the publish-time sandbox check (renderer kernel/sandbox-check.ts). The
 * sandbox document grants nothing, so serving it for local code is harmless.
 */
export function sandboxManifestFor(id: string) {
  return assetRegistry?.list().find(record => record.id === id && (record.trust === 'store' || record.trust === 'local'))?.manifest ?? null;
}

export function registerExtensionsIpc(
  ipcMain: Pick<IpcMain, 'handle' | 'on'>,
  options: {
    registry: ExtensionRegistry;
    resourcesDir: string;
    isTrusted(event: ExtensionsIpcEvent): boolean;
    /**
     * Runs before a user extension is removed (it may ask the user something).
     * A returned function runs only after the removal succeeded.
     */
    beforeRemove?(event: IpcMainInvokeEvent, id: string): Promise<(() => Promise<void>) | undefined>;
  }
): void {
  const { registry } = options;
  assetBuildDir = registry.buildDir;
  assetRegistry = registry;

  const requireTrusted = (event: ExtensionsIpcEvent, channel: string): void => {
    if (!options.isTrusted(event)) throw new IpcValidationError(channel, 'untrusted sender');
  };

  ipcMain.handle(EXT_IPC.list, (event) => {
    requireTrusted(event, EXT_IPC.list);
    return registry.list();
  });

  ipcMain.handle(EXT_IPC.setEnabled, (event, payload: unknown) => {
    requireTrusted(event, EXT_IPC.setEnabled);
    if (!isRecord(payload) || !isExtensionId(payload['id']) || typeof payload['enabled'] !== 'boolean') {
      throw new IpcValidationError(EXT_IPC.setEnabled, 'expected { id, enabled }');
    }
    return registry.setEnabled(payload as unknown as ExtensionSetEnabledRequest);
  });

  ipcMain.handle(EXT_IPC.remove, async (event, payload: unknown) => {
    requireTrusted(event, EXT_IPC.remove);
    const request = extensionIdRequest(payload, EXT_IPC.remove);
    return removeUserExtension({ registry, ...(options.beforeRemove ? { beforeRemove: options.beforeRemove } : {}) }, event, request.id);
  });

  ipcMain.handle(EXT_IPC.reload, (event, payload: unknown) => {
    requireTrusted(event, EXT_IPC.reload);
    return registry.reload(extensionIdRequest(payload, EXT_IPC.reload));
  });

  ipcMain.handle(EXT_IPC.create, (event, payload: unknown) => {
    requireTrusted(event, EXT_IPC.create);
    if (
      !isRecord(payload) ||
      !isRecord(payload['manifest']) ||
      !isExtensionId(payload['manifest']['id']) ||
      !isRecord(payload['files']) ||
      Object.keys(payload['files']).length > MANIFEST_LIMITS.sourceFiles ||
      !Object.values(payload['files']).every((value) => typeof value === 'string')
    ) {
      throw new IpcValidationError(EXT_IPC.create, 'expected { manifest, files }');
    }
    return registry.create(payload as unknown as ExtensionCreateRequest);
  });

  ipcMain.handle(IPC.extensionFork, async (event, payload: unknown): Promise<ExtensionForkResult> => {
    requireTrusted(event, IPC.extensionFork);
    const { id } = extensionIdRequest(payload, IPC.extensionFork);
    const result = await forkBuiltinExtension({
      resourcesDir: options.resourcesDir,
      id,
      targetDir: registry.userDir
    });
    await registry.refresh([id, result.forkId]);
    registry.emitChanged({ ids: [id, result.forkId], reason: 'create' });
    return { id: result.forkId };
  });

  ipcMain.handle(EXT_IPC.reveal, (event, payload: unknown) => {
    requireTrusted(event, EXT_IPC.reveal);
    return registry.reveal(extensionIdRequest(payload, EXT_IPC.reveal));
  });

  ipcMain.handle(EXT_IPC.readSource, (event, payload: unknown) => {
    requireTrusted(event, EXT_IPC.readSource);
    return registry.readSource(extensionIdRequest(payload, EXT_IPC.readSource));
  });

  ipcMain.on(EXT_IPC.reportHealth, (event, payload: unknown) => {
    if (!options.isTrusted(event)) return;
    if (!isHealthReport(payload)) return;
    try {
      registry.reportHealth(payload);
    } catch {
      // A stale renderer can report after an extension was removed. One-way
      // telemetry must not surface as an uncaught main-process exception.
    }
  });
}

/**
 * The one way a user extension is removed: `beforeRemove` first (it may ask
 * about the extension's values), then the folder, then whatever the prompt
 * decided. `ext:remove` and the Store's Uninstall both go through here.
 */
export async function removeUserExtension(
  options: {
    registry: Pick<ExtensionRegistry, 'remove'>;
    beforeRemove?(event: IpcMainInvokeEvent, id: string): Promise<(() => Promise<void>) | undefined>;
  },
  event: IpcMainInvokeEvent,
  id: string
): Promise<ExtensionRecord[]> {
  const after = await options.beforeRemove?.(event, id);
  const records = await options.registry.remove({ id });
  if (after) {
    try {
      await after();
    } catch (error) {
      // The folder is already gone; a leftover values file is harmless.
      console.error('[extensions] cleanup after remove failed', error);
    }
  }
  return records;
}

function extensionIdRequest(payload: unknown, channel: string): ExtensionIdRequest {
  if (!isRecord(payload) || !isExtensionId(payload['id'])) {
    throw new IpcValidationError(channel, 'expected { id }');
  }
  return { id: payload['id'] };
}

function isExtensionId(value: unknown): value is string {
  return typeof value === 'string' && EXTENSION_ID.test(value);
}

function isHealthReport(payload: unknown): payload is ExtensionHealthReport {
  if (!isRecord(payload) || !isExtensionId(payload['id']) || !isRecord(payload['health'])) return false;
  const state = payload['health']['state'];
  if (state === 'ok') return Object.keys(payload['health']).length === 1;
  return (
    (state === 'activation-error' || state === 'runtime-error') &&
    typeof payload['health']['error'] === 'string' &&
    payload['health']['error'].length > 0 &&
    payload['health']['error'].length <= MANIFEST_LIMITS.errorChars &&
    Object.keys(payload['health']).length === 2
  );
}

/**
 * Serve the one generated asset extensions are allowed to expose. The registry
 * configures the build root before any renderer window is created.
 */
export async function serveExtensionAsset(pathname: string): Promise<Response | null> {
  if (assetBuildDir === null) return null;

  const match = /^\/?ext\/([a-z0-9][a-z0-9-]{1,63})\/bundle\.js$/.exec(pathname);
  if (!match || !EXTENSION_ID.test(match[1] ?? '')) return null;

  try {
    const root = await realpath(assetBuildDir);
    const id = match[1];
    if (!id) return null;
    const directory = path.join(root, id);
    if (!(await lstat(directory)).isDirectory()) return null;
    const realDirectory = await realpath(directory);
    if (realDirectory !== directory) return null;
    const candidate = path.join(realDirectory, 'bundle.js');
    const contents = await readRegularBundle(candidate);
    if (!contents) return null;
    const body = new ArrayBuffer(contents.byteLength);
    new Uint8Array(body).set(contents);
    return new Response(body, {
      headers: {
        'Cache-Control': 'no-store',
        'Content-Type': 'text/javascript; charset=utf-8',
        'X-Content-Type-Options': 'nosniff'
      }
    });
  } catch {
    return null;
  }
}

/**
 * Development renders the app from Vite's http://localhost origin while
 * generated extensions still come from app://powermove. Module imports enforce
 * CORS across that boundary, so allow only the exact renderer origin Electron
 * gave us. Packaged builds have no dev URL and therefore expose no CORS header.
 */
export function extensionAssetCorsHeaders(devRendererUrl: string | undefined): Record<string, string> {
  if (!devRendererUrl) return {};
  try {
    return { 'Access-Control-Allow-Origin': new URL(devRendererUrl).origin };
  } catch {
    return {};
  }
}
