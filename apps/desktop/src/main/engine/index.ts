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
import { existsSync, statSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';

import { BrowserWindow, dialog, MessageChannelMain, Notification, powerSaveBlocker, screen, sharedTexture, shell, utilityProcess, webContents, type IpcMain, type IpcMainInvokeEvent, type WebContents } from 'electron';

import { extensionOf, isRenderFormat, migrateFormat, normalizeDelivery } from '../../shared/delivery';
import { ENGINE_IPC, type EngineSnapshotReply, type EngineExportRequest, type EngineExportStart, type EngineProbeRequest, type EngineProbeResult, type EngineQueueRequest, type EngineStatusMessage, type RenderQueueAction, type RenderQueueState, type StageDocumentRequest, type StageDocumentResult, validSheetExportOptions } from '../../shared/engine-ipc';
import { rememberExport } from '../export-sidecar';
import { discoverBlender } from '../blender-service';
import { routeToCurrentExporter } from '../render-encoder';
import type { EngineProcess } from '../../shared/engine-protocol';
import type { Log } from './connection';
import { BRIDGE_ADDON_FILE, FrameBridge, type BridgeAddon, type BridgeLaunch, type SharedTextureLike } from './bridge';
import { EngineFontClient, FONT_CLIENT_ID, installEngineFontService, JOB_FONT_CLIENT_ID } from '../engine-fonts';
import { startEngineFontSeeds } from '../engine-fonts-seeds';
import { NativeEngine, type NativeEngineStatus } from './native-engine';
import { CACHE_DIRS } from './services';
import { AgentJobError, AgentJobs } from './agent-jobs';
import { askAboutOfflineMedia, offlineClips } from './offline-media';
import { isId } from './protocol';
import { RenderQueue } from './render-queue';
import { SnapshotService } from './snapshots';
import { WindowSnapshotSessions } from './window-sessions';
import { CompatFrames } from './compat-frames';
import { ShaderFallbackRelay } from './shader-fallback-relay';
import type { ChildLike, PortLike } from './ports';

export { FONT_CLIENT_ID, JOB_FONT_CLIENT_ID } from '../engine-fonts';

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

/** Main's monotonic clock in µs (libuv's hrtime). On macOS it counts the
 *  time the Mac slept, unlike the engines' `CLOCK_UPTIME_RAW`
 *  (`mach_absolute_time`), so after a sleep the two differ by the sleep:
 *  use `engineHostNowUs` where a value meets an engine's clock. */
export function hostNowUs(): number {
  return Number(process.hrtime.bigint() / 1000n);
}

/** The engines' host clock (contract 9 `HostMicros`, `CLOCK_UPTIME_RAW`)
 *  through pm-bridge, when its addon loaded; else main's own clock. */
export function engineClock(addon: { nowUs(): number } | null): () => number {
  return addon && typeof addon.nowUs === 'function' ? () => addon.nowUs() : hostNowUs;
}

export type InstallNativeEngineOptions = {
  enabled: boolean;
  ipc: IpcMain;
  isTrustedSender: (event: IpcMainInvokeEvent | Electron.IpcMainEvent) => boolean;
  userData: string;
  home: string;
  /** Where agent exports go (default `~/Powermove`). */
  automaticOutputDirectory?: string;
  appPath: string;
  isPackaged: boolean;
  resourcesPath: string;
  /** Directory holding the built `engine-process.js` (main's `__dirname`). */
  mainDirectory: string;
  env?: Record<string, string | undefined>;
  log?: Log;
  windows: () => BrowserWindow[];
  /** The media store's asset resolution for `mediaRead` handles. */
  resolveAsset?: (asset: string, context?: { process: EngineProcess; session: number }) => Promise<{ path: string; contentHash: import('../../shared/engine-protocol').ContentHash | null } | null>;
  /** Keeps job snapshot media available after its window changes or closes. */
  snapshotMedia?(project: Record<string, any>): Promise<Record<string, any>>;
  bindMediaSession?(session: number, project: Record<string, any> | null): Promise<void>;
  /** A proxy of an offline asset for draft renders (Phase 4a generates them). */
  resolveProxy?: (asset: string) => Promise<{ path: string; contentHash: import('../../shared/engine-protocol').ContentHash | null } | null>;
};

export type InstalledNativeEngine = {
  engine: NativeEngine | null;
  /** Agent exports and frames through the job engine (agent-jobs.ts). */
  agentJobs: AgentJobs | null;
  /** The render queue (render-queue.ts). */
  renderQueue?: RenderQueue | null;
  /** The frame bridge, when its addon loaded. */
  frameBridge?: FrameBridge | null;
  /** Engine snapshots for windows (snapshots.ts). */
  snapshots?: SnapshotService | null;
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

/** Largest project JSON an export may carry (the largest corpus history is 60 MB). */
const MAX_EXPORT_PROJECT = 256 * 1024 ** 2;

const MAX_INTERPRET_JSON = 16 * 1024;

function validProbeRequest(value: unknown): value is EngineProbeRequest {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  const asset = v['asset'];
  const interpret = v['interpret'];
  if (!isId(asset)) return false;
  if (interpret === null) return true;
  if (typeof interpret !== 'object' || Array.isArray(interpret)) return false;
  try {
    return JSON.stringify(interpret).length <= MAX_INTERPRET_JSON;
  } catch {
    return false;
  }
}

function validExportRequest(value: unknown): value is EngineExportRequest {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  if (v['sidecars'] !== undefined && !(Array.isArray(v['sidecars']) && v['sidecars'].length <= 32)) return false;
  return typeof v['project'] === 'string' && v['project'].length > 0 && v['project'].length <= MAX_EXPORT_PROJECT &&
    typeof v['name'] === 'string' && v['name'].length <= 240 && validSheetExportOptions(v['options']);
}

/** What a format's save panel asks for: a file of its extension, or (PNG
 *  sequences) the name of the folder that will hold the frames. */
function savePanel(format: ReturnType<typeof migrateFormat>, delivery: ReturnType<typeof normalizeDelivery>, name: string): { extension: string | null; options: Electron.SaveDialogOptions } {
  const extension = isRenderFormat(format) ? extensionOf(format, delivery) : null;
  const safe = name.replace(/[\\/:]/g, '_') || 'Powermove';
  if (extension === null) {
    return { extension, options: { title: 'Export Image Sequence', defaultPath: safe, buttonLabel: 'Export', nameFieldLabel: 'Folder name:', properties: ['createDirectory', 'showOverwriteConfirmation'] } };
  }
  const kind = extension === 'png' ? 'PNG image' : extension === 'webm' ? 'WebM video' : extension === 'mov' ? 'QuickTime movie' : 'MPEG-4 video';
  return { extension, options: { defaultPath: `${safe}.${extension}`, filters: [{ name: kind, extensions: [extension] }], properties: ['createDirectory', 'showOverwriteConfirmation'] } };
}

export function installNativeEngine(options: InstallNativeEngineOptions): InstalledNativeEngine {
  const log = options.log ?? defaultLog;
  const env = options.env ?? process.env;
  let engine: NativeEngine | null = null;

  options.ipc.handle(ENGINE_IPC.getStatus, (event) => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    return statusMessage(options.enabled, engine?.status() ?? null);
  });
  /* Interpret Footage (section 9): the job engine reads the asset's file. */
  options.ipc.handle(ENGINE_IPC.probeMedia, async (event, request: unknown): Promise<EngineProbeResult> => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!validProbeRequest(request)) throw new Error('A probe names an asset and its interpretation.');
    if (!engine) return { status: 'unavailable', reason: 'Interpret Footage needs the new engine (Settings › General › Use New Engine).' };

    try {
      const facts = await engine.probeMedia(request.asset, request.interpret ? JSON.stringify(request.interpret) : null);
      return { status: 'facts', facts };
    } catch (error) {
      return { status: 'unavailable', reason: error instanceof Error ? error.message : String(error) };
    }
  });
  options.ipc.on(ENGINE_IPC.enabledSync, (event) => {
    event.returnValue = options.isTrustedSender(event) && options.enabled;
  });
  /* Snapshots (the Phase 3c render transport, snapshots.ts). A synchronous
     caller always gets exactly one reply, set when the engine answers. */
  let snapshots: SnapshotService | null = null;
  const unavailable: EngineSnapshotReply = { status: 'failed', code: 'unavailable', message: 'The new engine is off (Settings › General › Use New Engine).' };
  const snapshotReply = (event: Electron.IpcMainEvent | IpcMainInvokeEvent, request: unknown): Promise<EngineSnapshotReply> => {
    if (!options.isTrustedSender(event)) return Promise.resolve({ status: 'failed', code: 'refused', message: 'Unauthorized IPC sender' });
    if (!snapshots) return Promise.resolve(unavailable);
    return snapshots.snapshot(event.sender.id, request).catch((error: unknown) => ({ status: 'failed', code: 'internal', message: error instanceof Error ? error.message : String(error) }) as EngineSnapshotReply);
  };
  options.ipc.on(ENGINE_IPC.snapshotSync, (event, request: unknown) => {
    void snapshotReply(event, request).then((reply) => {
      if (!event.sender.isDestroyed()) event.returnValue = reply;
    });
  });
  options.ipc.handle(ENGINE_IPC.snapshot, (event, request: unknown) => snapshotReply(event, request));
  const removeSnapshotIpc = (): void => {
    options.ipc.removeAllListeners(ENGINE_IPC.snapshotSync);
    options.ipc.removeHandler(ENGINE_IPC.snapshot);
  };

  if (!options.enabled) {
    // The probe stays answerable (with the reason it cannot run) while the engine is off.
    return {
      engine: null,
      agentJobs: null,
      attachWindow: () => {},
      dispose: async () => {
        options.ipc.removeHandler(ENGINE_IPC.probeMedia);
        removeSnapshotIpc();
      }
    };
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

  // The job engine's decision 19 fallback requests run in a window's
  // runner (contract 14): the window the job works for, else any window.
  let jobOwners: { ownerOf(session: number): number | null } | null = null;
  const fallbackRelay = new ShaderFallbackRelay({
    send: (id, job) => {
      const contents = webContents.fromId(id);
      if (!contents || contents.isDestroyed()) return false;
      contents.send(ENGINE_IPC.shaderFallback, job);
      return true;
    },
    windows: () => options.windows().filter((w) => !w.isDestroyed()).map((w) => w.webContents.id),
    ownerOf: (session) => {
      const owner = jobOwners?.ownerOf(session) ?? null;
      return owner !== null && owner >= 0 ? owner : null;
    }
  });
  options.ipc.on(ENGINE_IPC.shaderFallbackResult, (event, message: unknown) => {
    if (!options.isTrustedSender(event)) return;
    fallbackRelay.answer(event.sender.id, message);
  });

  const engineNowUs = engineClock(bridgeAddon);
  const cacheDirectory = path.join(options.userData, 'Engine Cache');
  // Exports of layers only a window draws (a `raster` override, a
  // compatibility WebGL2 context): the job engine asks, the windows answer.
  const compatFrames = new CompatFrames({
    windows: () => options.windows()
      .filter((w) => !w.isDestroyed())
      .map((w) => ({ id: w.webContents.id, ask: (ask) => w.webContents.send(ENGINE_IPC.compatAsk, ask) })),
    publishPixels: bridgeAddon?.publishPixels ? (bytes) => bridgeAddon.publishPixels!(bytes) : null,
    unlinkPixels: bridgeAddon?.unlinkPixels ? (name) => bridgeAddon.unlinkPixels!(name) : null
  });
  options.ipc.on(ENGINE_IPC.compatAnswer, (event, answer: unknown) => {
    if (!options.isTrustedSender(event)) return;
    if (!compatFrames.answer(event.sender.id, answer)) log('warn', `window ${event.sender.id} sent a compatibility answer nobody asked for`);
  });
  engine = new NativeEngine({
    // A measurement control only; packaged apps always start jobs on demand.
    eagerJobs: !options.isPackaged && env['POWERMOVE_ENGINE_EAGER_JOBS'] === '1',
    compatFrame: (request) => compatFrames.serve(request),
    spawn: (process) => {
      const launch: BridgeLaunch | null = process === 'interactive' ? (frameBridge?.prepareLaunch() ?? null) : null;
      const child = utilityProcess.fork(script, [], {
        serviceName: process === 'interactive' ? 'Powermove Interactive Engine' : 'Powermove Job Engine',
        stdio: ['ignore', 'pipe', 'pipe'],
        // The interactive engine draws the session's document (the Phase 2
        // tracer bullet); POWERMOVE_ENGINE_FRAME_SOURCE overrides it
        // (`test-pattern` for the bridge harnesses).
        env: {
          POWERMOVE_ENGINE_FRAME_SOURCE: 'document',
          ...env,
          ...(launch?.env ?? {}),
          POWERMOVE_ENGINE_PROCESS: process,
          POWERMOVE_ENGINE_ADDON: addon,
          // Invalidate first-use parity on every engine/browser/OS update.
          // GPU identity and native driver version are added by pm-render.
          POWERMOVE_ENGINE_PARITY_ENV: JSON.stringify({ engine: statSync(addon).mtimeMs, bytes: statSync(addon).size, electron: globalThis.process.versions.electron, chrome: globalThis.process.versions.chrome, os: os.version() }),
          // Translated shader programs, kept across launches (plan section 11).
          POWERMOVE_ENGINE_SHADER_CACHE: path.join(cacheDirectory, CACHE_DIRS.shaderCache)
        }
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
    cacheDirectory,
    automaticOutputDirectory: options.automaticOutputDirectory ?? path.join(options.home, 'Powermove'),
    physicalMemory: os.totalmem(),
    hostNow: engineNowUs,
    ...(options.resolveAsset ? { resolveAsset: options.resolveAsset } : {}),
    ...(options.resolveProxy ? { resolveProxy: options.resolveProxy } : {}),
    // Blender jobs of the job engine (contract 11): the Blender the user
    // chose in Settings (`blender-path.json`, as the current engine's
    // Blender service reads it), else the first one discovered.
    blender: {
      tempRoot: os.tmpdir(),
      locate: async () => {
        const configured = await readFile(path.join(options.userData, 'blender-path.json'), 'utf8')
          .then((raw) => {
            const p = JSON.parse(raw)?.path;
            return typeof p === 'string' && path.isAbsolute(p) ? p : undefined;
          })
          .catch(() => undefined);
        const status = await discoverBlender(configured);
        return status.available && status.path
          ? { available: true, path: status.path, ...(status.version ? { version: status.version } : {}) }
          : { available: false, error: status.error ?? 'This composition renders with Blender. Install Blender 4.5 or later, or set its renderer to Native.' };
      }
    },
    shaderFallback: (session, request, textures) => fallbackRelay.run(session, request, textures),
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
  // Sync snapshots a window's own session cannot serve (none yet, or a
  // staged resync in flight) come from a session main opens for the window.
  const windowSessions = new WindowSnapshotSessions(
    { isReady: (p) => engine.isReady(p), hostPort: (p) => engine.hostPort(p), request: (p, body, session, revision) => engine.request(p, body, session, revision), stageDocument: (p, session, bytes) => engine.stageDocument(p, session, bytes), event: (p, body, session) => engine.event(p, body, session) },
    engine.definitions,
    log,
    undefined,
    (session, owner) => fontClient.setSessionOwner(session, owner)
  );
  snapshots = new SnapshotService({
    engine,
    takePixels: bridgeAddon?.takePixels ? (name, length) => bridgeAddon.takePixels!(name, length) : null,
    windowSessions,
    log
  });

  // The font registry and subsetting run in the interactive engine (plan
  // section 12, Phase 2): main relays pm-wasm's font requests over its own
  // font port and keeps only the project fonts a restarted engine needs.
  const fontClient = new EngineFontClient({ log });
  const jobFontClient = new EngineFontClient({ log });
  fontClient.mirror(jobFontClient);
  const fontSeeds = startEngineFontSeeds(fontClient, log, options.windows);
  const uninstallFonts = installEngineFontService(options.ipc, { isTrustedSender: options.isTrustedSender, seedsFor: fontSeeds.seedsFor, reportUiFonts: fontSeeds.reportUiFonts }, fontClient);
  engine.addClient(FONT_CLIENT_ID, (_info, port) => fontClient.connect(port), { peer: 'fonts', processes: ['interactive'] });
  engine.addClient(JOB_FONT_CLIENT_ID, (_info, port) => jobFontClient.connect(port), { peer: 'fonts', processes: ['job'] });

  // The shared host clock for the renderer's engine connection: it maps
  // performance.now() onto the engines' host µs (clock samples, the
  // viewer's presentation) from the round trip with the least delay.
  options.ipc.on(ENGINE_IPC.requestPorts, (event) => {
    if (!options.isTrustedSender(event)) return;
    engine!.reconnectClient(event.sender.id);
  });
  options.ipc.handle(ENGINE_IPC.hostTime, (event) => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    return engineNowUs();
  });
  options.ipc.handle(ENGINE_IPC.stageDocument, async (event, request: unknown): Promise<StageDocumentResult> => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!validStageRequest(request)) throw new Error('A staged document names an engine, a session, and its bytes.');
    return engine!.stageDocument(request.process, request.session, request.bytes);
  });
  /* The editor's transport: the job engine slices while an editor plays,
     not only while the interactive engine does (the viewer can draw
     playback without it). The frame budget is the window's display's. */
  const playingWindows = new Set<number>();
  options.ipc.on(ENGINE_IPC.playback, (event, playing: unknown) => {
    if (!options.isTrustedSender(event) || typeof playing !== 'boolean') return;
    const sender = event.sender;
    const owner = sender.id;
    const window = BrowserWindow.fromWebContents(sender);
    const hz = window && !window.isDestroyed() ? screen.getDisplayMatching(window.getBounds()).displayFrequency : 0;
    engine!.editorPlayback(owner, playing, hz > 0 ? 1_000_000 / hz : undefined);
    if (playing && !playingWindows.has(owner)) {
      playingWindows.add(owner);
      sender.once('destroyed', () => {
        playingWindows.delete(owner);
        engine?.editorPlayback(owner, false);
      });
    }
  });
  options.ipc.on(ENGINE_IPC.definitions, (event, update: unknown) => {
    if (!options.isTrustedSender(event)) return;
    try {
      engine!.updateDefinitions(event.sender.id, update);
    } catch (error) {
      log('warn', `definitions from window ${event.sender.id} refused: ${error instanceof Error ? error.message : String(error)}`);
    }
  });

  /* Export sheet jobs: where each writes, so a routed one can hand its file
     to the current exporter (render-encoder.ts); while any runs, the Mac
     stays awake, and a finished one posts a notification when its window
     is not focused (section 17, "Comfort"). */
  const userDestinations = new Map<string, string>();
  const awake = new Map<string, number>();
  const keepAwake = (job: string): void => {
    if (!awake.has(job)) awake.set(job, powerSaveBlocker.start('prevent-app-suspension'));
  };
  const letSleep = (job: string): void => {
    const id = awake.get(job);
    if (id !== undefined && powerSaveBlocker.isStarted(id)) powerSaveBlocker.stop(id);
    awake.delete(job);
  };
  const notify = (title: string, body: string): void => {
    if (options.windows().some((w) => !w.isDestroyed() && w.isFocused())) return;
    if (Notification.isSupported()) new Notification({ title, body, silent: false }).show();
  };
  const broadcastQueue = (state: RenderQueueState): void => {
    for (const window of options.windows()) {
      if (!window.isDestroyed()) window.webContents.send(ENGINE_IPC.queueChanged, state);
    }
  };
  let renderQueue: RenderQueue | null = null;
  const agentJobs = new AgentJobs({
    engine,
    ...(options.bindMediaSession ? { bindMediaSession: options.bindMediaSession } : {}),
    log,
    // Each job renders its window's winning registrations (contract 10).
    definitions: (owner) => engine!.definitions.current(owner),
    // A window's exports measure text with its font cache seeds.
    sessionOwner: (session, owner) => jobFontClient.setSessionOwner(session, owner),
    onDelivered: (owner, file) => rememberExport(owner, file),
    onUserStatus: (owner, status) => {
      const file = userDestinations.get(status.jobId);
      const ended = status.state === 'done' || status.state === 'failed' || status.state === 'cancelled';
      if (ended) {
        userDestinations.delete(status.jobId);
        letSleep(status.jobId);
        if (status.state === 'done') notify('Export finished', path.basename(status.path ?? file ?? ''));
        else if (status.state === 'failed' && !status.routed) notify('Export failed', status.error ?? 'The export failed.');
      }
      if (status.routed && file) routeToCurrentExporter(owner, file);
      const contents = webContents.fromId(owner);
      if (contents && !contents.isDestroyed()) contents.send(ENGINE_IPC.exportStatus, status);
    },
    onQueueStatus: (jobId, status) => renderQueue?.status(jobId, status),
    onAgentJob: (job) => renderQueue?.agentJob(job)
  });
  jobOwners = agentJobs;
  renderQueue = new RenderQueue({
    windowOpen: (owner) => { const contents = webContents.fromId(owner); return !!contents && !contents.isDestroyed(); },
    directory: path.join(options.userData, 'Render Queue'),
    engine: agentJobs,
    platform: {
      preventSleep: () => {
        const id = powerSaveBlocker.start('prevent-app-suspension');
        return () => { if (powerSaveBlocker.isStarted(id)) powerSaveBlocker.stop(id); };
      },
      notify,
      reveal: (file) => shell.showItemInFolder(file),
      log
    },
    onChange: broadcastQueue
  });
  engine.listen({
    status: (status) => { if (status.job.state === 'ready') renderQueue?.engineReady(); }
  });

  /** The offline message before an interactive export (section 14.8).
   *  Resolves the options to export with, or the outcome to answer. */
  const checkOffline = async (window: BrowserWindow, sender: WebContents, project: Record<string, any>, request: EngineExportRequest): Promise<EngineExportRequest['options'] | EngineExportStart> => {
    if (!options.resolveAsset || request.options.draft) return request.options;
    const clips = await offlineClips(project, (asset) => options.resolveAsset!(asset));
    if (!clips.length) return request.options;
    let proxies = !!options.resolveProxy;
    if (proxies) {
      for (const clip of clips) if (!await options.resolveProxy!(clip.asset).catch(() => null)) proxies = false;
    }
    const choice = await askAboutOfflineMedia(window, clips, proxies);
    if (choice === 'proxies') return { ...request.options, draft: true };
    if (choice === 'locate') {
      const assets = clips.map((c) => c.asset);
      if (!sender.isDestroyed()) sender.send(ENGINE_IPC.locateMedia, assets);
      return { status: 'locate', assets };
    }
    return { status: 'cancelled' };
  };

  options.ipc.handle(ENGINE_IPC.exportVideo, async (event, request: unknown): Promise<EngineExportStart> => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!validExportRequest(request)) throw new Error('An export names a project and its settings.');
    if (!agentJobs.available) return { status: 'unavailable', reason: 'The job engine is not running.' };
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) throw new Error('Export window is unavailable');
    const project = JSON.parse(request.project);
    const checked = await checkOffline(window, event.sender, project, request);
    if ('status' in checked) return checked;
    const format = migrateFormat(checked.format);
    const delivery = normalizeDelivery(checked.delivery ?? {});
    const panel = savePanel(format, delivery, request.name);
    const chosen = await dialog.showSaveDialog(window, panel.options);
    if (chosen.canceled || !chosen.filePath || window.isDestroyed()) return { status: 'cancelled' };
    const file = panel.extension === null || path.extname(chosen.filePath).toLowerCase() === `.${panel.extension}`
      ? chosen.filePath
      : `${chosen.filePath}.${panel.extension}`;
    try {
      const started = await agentJobs.exportUser(event.sender.id, project, checked, file);
      userDestinations.set(started.jobId, file);
      keepAwake(started.jobId);
      return { status: 'accepted', jobId: started.jobId, frames: started.frames, path: file };
    } catch (error) {
      if (!(error instanceof AgentJobError)) throw error;
      // The current exporter writes the file the user just chose.
      routeToCurrentExporter(event.sender.id, file);
      return { status: 'unavailable', reason: error.message };
    }
  });
  options.ipc.handle(ENGINE_IPC.cancelExport, async (event, jobId: unknown) => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof jobId !== 'string' || !jobId || jobId.length > 256) throw new Error('A cancel names a job.');
    await agentJobs.cancelJob(event.sender.id, jobId);
  });

  /* The render queue: "Add to Queue" asks for the job's destination now, so
     the queue runs unattended later. */
  options.ipc.handle(ENGINE_IPC.queueAdd, async (event, request: unknown) => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!validExportRequest(request)) throw new Error('A queued export names a project and its settings.');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) throw new Error('Export window is unavailable');
    const queueRequest = request as EngineQueueRequest;
    const format = migrateFormat(queueRequest.options.format);
    const delivery = normalizeDelivery(queueRequest.options.delivery ?? {});
    const panel = savePanel(format, delivery, queueRequest.name);
    const chosen = await dialog.showSaveDialog(window, { ...panel.options, buttonLabel: 'Add to Queue' });
    if (chosen.canceled || !chosen.filePath) return { status: 'cancelled' as const };
    const file = panel.extension === null || path.extname(chosen.filePath).toLowerCase() === `.${panel.extension}`
      ? chosen.filePath
      : `${chosen.filePath}.${panel.extension}`;
    const id = await renderQueue!.add({
      owner: event.sender.id,
      name: path.basename(file),
      project: options.snapshotMedia ? await options.snapshotMedia(JSON.parse(queueRequest.project)) : JSON.parse(queueRequest.project),
      options: queueRequest.options,
      destination: file,
      ...(queueRequest.sidecars ? { sidecars: queueRequest.sidecars } : {}),
      ...(typeof queueRequest.preset === 'string' ? { preset: queueRequest.preset.slice(0, 120) } : {}),
      definitions: engine!.definitions.current(event.sender.id)?.updates(1) ?? []
    });
    return { status: 'added' as const, id };
  });
  options.ipc.handle(ENGINE_IPC.queueList, async (event) => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    await renderQueue!.ready();
    return renderQueue!.state();
  });
  options.ipc.handle(ENGINE_IPC.queueAction, async (event, action: unknown) => {
    if (!options.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const a = action as RenderQueueAction | null;
    const kinds = ['start', 'pause', 'remove', 'cancel', 'retry', 'move', 'reveal', 'clearFinished'];
    if (!a || typeof a !== 'object' || !kinds.includes(a.kind)) throw new Error('Unknown render queue action.');
    if ('id' in a && (typeof a.id !== 'string' || a.id.length > 128)) throw new Error('A render queue action names a job.');
    return renderQueue!.act(a, (owner, jobId) => agentJobs.cancelJob(owner, jobId));
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
      // A new page starts its document revisions again.
      windowSessions.forget(id);
      if (!engine) return;
      if (attached) engine.reconnectClient(id);
      else engine.addClient(id, deliver);
      attached = true;
    });
    window.once('closed', () => {
      fallbackRelay.forget(id);
      engine?.removeClient(id);
      windowSessions.forget(id);
      compatFrames.windowGone(id);
      void agentJobs.forgetOwner(id);
    });
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
    agentJobs,
    renderQueue,
    frameBridge,
    snapshots,
    attachWindow,
    dispose: async () => {
      removeSnapshotIpc();
      options.ipc.removeAllListeners(ENGINE_IPC.compatAnswer);
      windowSessions.dispose();
      snapshots = null;
      options.ipc.removeHandler(ENGINE_IPC.stageDocument);
      options.ipc.removeHandler(ENGINE_IPC.probeMedia);
      options.ipc.removeHandler(ENGINE_IPC.hostTime);
      options.ipc.removeAllListeners(ENGINE_IPC.requestPorts);
      options.ipc.removeHandler(ENGINE_IPC.exportVideo);
      options.ipc.removeHandler(ENGINE_IPC.cancelExport);
      options.ipc.removeHandler(ENGINE_IPC.queueAdd);
      options.ipc.removeHandler(ENGINE_IPC.queueList);
      options.ipc.removeHandler(ENGINE_IPC.queueAction);
      await renderQueue?.dispose();
      for (const job of [...awake.keys()]) letSleep(job);
      uninstallFonts();
      fontSeeds.stop();
      fontClient.disconnect();
      jobFontClient.disconnect('the job engine stopped');
      options.ipc.removeAllListeners(ENGINE_IPC.definitions);
      options.ipc.removeAllListeners(ENGINE_IPC.playback);
      options.ipc.removeAllListeners(ENGINE_IPC.shaderFallbackResult);
      agentJobs.dispose();
      await engine?.stop();
      frameBridge?.dispose();
    }
  };
}
