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
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';

import { MessageChannelMain, sharedTexture, utilityProcess, webContents, type BrowserWindow, type IpcMain, type IpcMainInvokeEvent, type WebContents } from 'electron';

import { ENGINE_IPC, type EngineStatusMessage, type StageDocumentRequest, type StageDocumentResult } from '../../shared/engine-ipc';
import type { EngineProcess } from '../../shared/engine-protocol';
import type { Log } from './connection';
import { BRIDGE_ADDON_FILE, FrameBridge, type BridgeAddon, type BridgeLaunch, type SharedTextureLike } from './bridge';
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

/** Where the pm-bridge addon is (the frame bridge in main, section 18):
 *  POWERMOVE_PM_BRIDGE, else unpacked from the asar in a packaged app, else
 *  the development build that the font registry also loads
 *  (engine/target/pm-bridge, engine-fonts.ts), so main loads one copy. */
export function bridgeAddonPath(options: { isPackaged: boolean; resourcesPath: string; appPath: string; env: Record<string, string | undefined> }): string {
  const override = options.env['POWERMOVE_PM_BRIDGE'];
  if (override && path.isAbsolute(override)) return override;
  return options.isPackaged
    ? path.join(options.resourcesPath, 'app.asar.unpacked', 'native', BRIDGE_ADDON_FILE)
    : path.resolve(options.appPath, '../../engine/target/pm-bridge', BRIDGE_ADDON_FILE);
}

/** Loads pm-bridge, or explains why viewer frames are unavailable. */
function loadBridgeAddon(file: string, log: Log): BridgeAddon | null {
  if (!existsSync(file)) {
    log('warn', `the frame bridge addon is missing at ${file}; the new engine cannot present viewer frames (build it with \`bun run build:engine\`)`);
    return null;
  }
  try {
    const addon = createRequire(__filename)(file) as BridgeAddon;
    if (typeof addon.BridgeHost !== 'function') throw new Error('it does not export BridgeHost');
    return addon;
  } catch (error) {
    log('error', `the frame bridge addon at ${file} does not load: ${error instanceof Error ? error.message : String(error)}`);
    return null;
  }
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
  /** The frame bridge, when its addon loaded. */
  frameBridge?: FrameBridge | null;
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

/** Largest staged resync document (the largest corpus history is 60 MB). */
const MAX_STAGED_BYTES = 256 * 1024 ** 2;

function validStageRequest(value: unknown): value is StageDocumentRequest {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return (v['process'] === 'interactive' || v['process'] === 'job') &&
    typeof v['session'] === 'number' && Number.isSafeInteger(v['session']) && (v['session'] as number) >= 0 &&
    v['bytes'] instanceof Uint8Array && v['bytes'].byteLength > 0 && v['bytes'].byteLength <= MAX_STAGED_BYTES;
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

  // The frame bridge (section 18): one per-launch rendezvous for every
  // interactive engine; surfaces go to the window owning the viewer's port.
  const bridgeAddon = loadBridgeAddon(bridgeAddonPath({ isPackaged: options.isPackaged, resourcesPath: options.resourcesPath, appPath: options.appPath, env }), log);
  const frameBridge = bridgeAddon
    ? new FrameBridge({
        addon: bridgeAddon,
        sharedTexture: sharedTexture as unknown as SharedTextureLike,
        // Only unsigned development builds; a signed main always requires its Team ID.
        allowUnsignedDev: !options.isPackaged,
        resolve: (port) => {
          const id = engine?.clientForPort('interactive', port);
          const contents = id == null ? null : webContents.fromId(id);
          return contents && !contents.isDestroyed() ? { frame: contents.mainFrame, owner: contents.id } : null;
        },
        log
      })
    : null;

  engine = new NativeEngine({
    spawn: (process) => {
      const launch: BridgeLaunch | null = process === 'interactive' ? (frameBridge?.prepareLaunch() ?? null) : null;
      const child = utilityProcess.fork(script, [], {
        serviceName: process === 'interactive' ? 'Powermove Interactive Engine' : 'Powermove Job Engine',
        stdio: ['ignore', 'pipe', 'pipe'],
        env: { ...env, ...(launch?.env ?? {}), POWERMOVE_ENGINE_PROCESS: process, POWERMOVE_ENGINE_ADDON: addon }
      });
      if (launch) {
        child.once('spawn', () => launch.spawned(child.pid));
        child.once('exit', () => launch.ended());
      }
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
    // A page that goes away never reports its frame surfaces released.
    if (frameBridge) {
      const gone = (): void => frameBridge.receiverGone(id);
      contents.on('did-start-navigation', (details) => {
        if (details.isMainFrame && !details.isSameDocument) gone();
      });
      contents.on('render-process-gone', gone);
      contents.once('destroyed', gone);
    }
  };

  return {
    engine,
    frameBridge,
    attachWindow,
    dispose: async () => {
      options.ipc.removeHandler(ENGINE_IPC.stageDocument);
      options.ipc.removeAllListeners(ENGINE_IPC.definitions);
      await engine?.stop();
      frameBridge?.dispose();
    }
  };
}
