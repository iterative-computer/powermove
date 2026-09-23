import { realpath, readFile, stat } from 'node:fs/promises';
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
    const candidate = await realpath(path.join(root, match[1]!, 'bundle.js'));
    const relative = path.relative(root, candidate);
    if (
      relative === '' ||
      relative === '..' ||
      relative.startsWith(`..${path.sep}`) ||
      path.isAbsolute(relative)
    ) {
      return null;
    }
    const metadata = await stat(candidate);
    if (!metadata.isFile()) return null;
    const contents = await readFile(candidate);
    return new Response(contents, {
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
