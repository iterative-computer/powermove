/*
 * Wires the native engine into the Electron app behind "Use New Engine".
 *
 * With the flag off this registers one read-only IPC handler that reports
 * the engine as disabled, and nothing else: no utility process, no ports.
 * With it on, main starts the interactive and job engine processes, gives
 * every editor window a port to each (main frame only, never extension
 * sandbox frames), tells windows about crashes and restarts, and serves
 * the engines' host requests (native-engine.ts).
 */
import { existsSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { MessageChannelMain, utilityProcess, type BrowserWindow, type IpcMain, type IpcMainInvokeEvent, type WebContents } from 'electron';

import { ENGINE_IPC, type EngineStatusMessage, type StageDocumentRequest, type StageDocumentResult } from '../../shared/engine-ipc';
import type { EngineProcess } from '../../shared/engine-protocol';
import type { Log } from './connection';
import { NativeEngine, type NativeEngineStatus } from './native-engine';
import type { ChildLike, PortLike } from './ports';

export { NEW_ENGINE_ENV, NEW_ENGINE_STORE_KEY, newEngineEnabled } from './flag';

export const ADDON_FILE = 'pm_node.node';

/** Where the pm-node addon is: unpacked from the asar in a packaged app
 *  (electron-builder.yml `asarUnpack`), `native/` in development, or
 *  POWERMOVE_ENGINE_ADDON. */
export function engineAddonPath(options: { isPackaged: boolean; resourcesPath: string; appPath: string; env: Record<string, string | undefined> }): string {
  const override = options.env['POWERMOVE_ENGINE_ADDON'];
  if (override && path.isAbsolute(override)) return override;
  return options.isPackaged
    ? path.join(options.resourcesPath, 'app.asar.unpacked', 'native', ADDON_FILE)
    : path.join(options.appPath, 'native', ADDON_FILE);
}

/** The shared host clock in µs: libuv's hrtime reads the same
 *  `mach_absolute_time` as the engines' `CLOCK_UPTIME_RAW` (contract 13). */
export function hostNowUs(): number {
  return Number(process.hrtime.bigint() / 1000n);
}

export type InstallNativeEngineOptions = {
  enabled: boolean;
  ipc: IpcMain;
  isTrustedSender: (event: IpcMainInvokeEvent | Electron.IpcMainEvent) => boolean;
  userData: string;
  home: string;
  appPath: string;
  isPackaged: boolean;
  resourcesPath: string;
  /** Directory holding the built `engine-process.js` (main's `__dirname`). */
  mainDirectory: string;
  env?: Record<string, string | undefined>;
  log?: Log;
  windows: () => BrowserWindow[];
};

export type InstalledNativeEngine = {
  engine: NativeEngine | null;
  attachWindow(window: BrowserWindow): void;
  dispose(): Promise<void>;
};

const defaultLog: Log = (level, message) => {
  const line = `[engine] ${message}`;
  if (level === 'error') console.error(line);
  else if (level === 'warn') console.warn(line);
  else console.info(line);
};

function statusMessage(enabled: boolean, status: NativeEngineStatus | null): EngineStatusMessage {
  const one = (p: EngineProcess) => {
    const s = status?.[p];
    return { state: s?.state ?? 'stopped', generation: s?.generation ?? 0, attempts: s?.attempts ?? 0 };
  };
  return { enabled, processes: { interactive: one('interactive'), job: one('job') } };
}

function validStageRequest(value: unknown): value is StageDocumentRequest {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return (v['process'] === 'interactive' || v['process'] === 'job') &&
    typeof v['session'] === 'number' && Number.isSafeInteger(v['session']) && (v['session'] as number) >= 0 &&
    v['bytes'] instanceof Uint8Array && v['bytes'].byteLength > 0 && v['bytes'].byteLength <= 1024 ** 3;
}

export function installNativeEngine(options: InstallNativeEngineOptions): InstalledNativeEngine {
  const log = options.log ?? defaultLog;
  const env = options.env ?? process.env;
  let engine: NativeEngine | null = null;

  options.ipc.handle(ENGINE_IPC.getStatus, (event) => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    return statusMessage(options.enabled, engine?.status() ?? null);
  });

  if (!options.enabled) {
    return { engine: null, attachWindow: () => {}, dispose: async () => {} };
  }

  const addon = engineAddonPath({ isPackaged: options.isPackaged, resourcesPath: options.resourcesPath, appPath: options.appPath, env });
  const script = path.join(options.mainDirectory, 'engine-process.js');
  if (!existsSync(addon)) log('error', `the engine addon is missing at ${addon}; build it with \`bun run build:engine\``);

  engine = new NativeEngine({
    spawn: (process) => {
      const child = utilityProcess.fork(script, [], {
        serviceName: process === 'interactive' ? 'Powermove Interactive Engine' : 'Powermove Job Engine',
        stdio: ['ignore', 'pipe', 'pipe'],
        env: { ...env, POWERMOVE_ENGINE_PROCESS: process, POWERMOVE_ENGINE_ADDON: addon }
      });
      const forward = (level: 'info' | 'warn') => (chunk: Buffer) => {
        for (const line of chunk.toString().split('\n')) if (line.trim()) log(level, `[${process} engine] ${line}`);
      };
      child.stdout?.on('data', forward('info'));
      child.stderr?.on('data', forward('warn'));
      return child as unknown as ChildLike;
    },
    channel: () => new MessageChannelMain() as unknown as { port1: PortLike; port2: PortLike },
    cacheDirectory: path.join(options.userData, 'Engine Cache'),
    automaticOutputDirectory: path.join(options.home, 'Powermove'),
    physicalMemory: os.totalmem(),
    hostNow: hostNowUs,
    log
  });

  const broadcast = (channel: string, payload: unknown): void => {
    for (const window of options.windows()) {
      if (!window.isDestroyed()) window.webContents.mainFrame.postMessage(channel, payload);
    }
  };
  engine.listen({
    crash: (report) => broadcast(ENGINE_IPC.crash, report),
    status: (status) => broadcast(ENGINE_IPC.status, statusMessage(true, status))
  });

  options.ipc.handle(ENGINE_IPC.stageDocument, async (event, request: unknown): Promise<StageDocumentResult> => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!validStageRequest(request)) throw new Error('A staged document names an engine, a session, and its bytes.');
    return engine!.stageDocument(request.process, request.session, request.bytes);
  });
  options.ipc.on(ENGINE_IPC.definitions, (event, update: unknown) => {
    if (!options.isTrustedSender(event)) return;
    try {
      engine!.updateDefinitions(event.sender.id, update);
    } catch (error) {
      log('warn', `definitions from window ${event.sender.id} refused: ${error instanceof Error ? error.message : String(error)}`);
    }
  });

  void engine.start().catch((error: unknown) => log('error', `the engine could not start: ${error instanceof Error ? error.message : String(error)}`));

  const attachWindow = (window: BrowserWindow): void => {
    const contents: WebContents = window.webContents;
    const id = contents.id;
    let attached = false;
    // Ports go to the main frame only: extension sandboxes are separate
    // frames and never receive them (section 19).
    const deliver = (info: Parameters<Parameters<NativeEngine['addClient']>[1]>[0], port: PortLike): void => {
      if (contents.isDestroyed()) {
        port.close();
        return;
      }
      contents.mainFrame.postMessage(ENGINE_IPC.port, info, [port as unknown as Electron.MessagePortMain]);
    };
    // A page load (first load or reload) gets fresh ports; ports given to
    // a page die with it.
    contents.on('did-finish-load', () => {
      if (!engine) return;
      if (attached) engine.reconnectClient(id);
      else engine.addClient(id, deliver);
      attached = true;
    });
    window.once('closed', () => engine?.removeClient(id));
  };

  return {
    engine,
    attachWindow,
    dispose: async () => {
      options.ipc.removeHandler(ENGINE_IPC.stageDocument);
      options.ipc.removeAllListeners(ENGINE_IPC.definitions);
      await engine?.stop();
    }
  };
}
