/*
 * The frame bridge in main (plan section 18; amendments 4 and 5).
 *
 * For every interactive engine launch main registers a per-launch Mach
 * bootstrap name (pm-bridge's `BridgeHost`) before the fork, passes it to
 * the engine in `POWERMOVE_FRAME_BRIDGE`, records the engine's identity
 * (pid and pidversion, parent, uid, and a code signature checked against
 * main's Team ID) right after the spawn, and accepts exactly one check-in
 * from that process. Then:
 *
 * - each pool surface arrives once over Mach and is imported once with
 *   `sharedTexture.importSharedTexture`;
 * - each import is sent once to the window whose viewer the pool serves
 *   (the engine names the client port; main knows its window), where the
 *   preload keeps it for the pool's lifetime;
 * - per-frame descriptors never pass through main: they go from the engine
 *   to the viewer over the viewer's `MessageChannelMain` port;
 * - when the engine retires a pool generation (resize, format change, the
 *   viewer's port closed), main releases its own reference; the surface
 *   goes away when Chromium's `allReferencesReleased` says every process
 *   let go of it.
 *
 * Popouts and reloads follow from ports: a popped-out or reloaded viewer
 * opens a session on its window's own port, and the engine makes it a pool
 * that this module sends to that window.
 */
import type { Log } from './connection';

/** Environment variable carrying the bootstrap name to the engine. */
export const FRAME_BRIDGE_ENV = 'POWERMOVE_FRAME_BRIDGE';

/** How long main waits for the engine's check-in. */
export const CHECK_IN_TIMEOUT_MS = 15_000;

/** One event from pm-bridge's receive thread (`BridgeEvent`). */
export type BridgeEvent = {
  kind: 'accepted' | 'rejected' | 'surface' | 'retire' | 'goodbye' | 'gone' | 'error';
  reason?: string | null;
  pid?: number | null;
  pidversion?: number | null;
  clientPort?: number | null;
  generation?: number | null;
  poolIndex?: number | null;
  poolSize?: number | null;
  width?: number | null;
  height?: number | null;
  format?: 'bgra' | 'rgbaf16' | null;
  bytesPerRow?: number | null;
  allocBytes?: number | null;
  ioSurface?: Buffer | null;
  receivedUs?: number | null;
};

/** pm-bridge's `BridgeHost` (one per launch). */
export interface BridgeHostLike {
  readonly name: string;
  policy(): string;
  expectChild(pid: number): { pid: number; pidversion: number; parentPid: number };
  start(callback: (event: BridgeEvent) => void, acceptTimeoutMs: number): void;
  releaseSurface(generation: number, poolIndex: number): boolean;
  held(): { count: number; bytes: number };
  stop(): void;
  releaseAll(): void;
}

/** The pm-bridge addon as main uses it. */
export interface BridgeAddon {
  BridgeHost: new (name: string, allowUnsignedDev: boolean) => BridgeHostLike;
  serviceName(): string;
  nowUs(): number;
}

/** Electron's `SharedTextureImported`. */
export interface ImportedTextureLike {
  release(): void;
}

/** The slice of Electron's `sharedTexture` module main uses. */
export interface SharedTextureLike {
  importSharedTexture(options: {
    textureInfo: {
      pixelFormat: 'bgra' | 'rgbaf16';
      codedSize: { width: number; height: number };
      colorSpace: { primaries: 'bt709'; transfer: 'srgb' | 'srgb-hdr'; matrix: 'rgb'; range: 'full' };
      handle: { ioSurface: Buffer };
    };
    allReferencesReleased: () => void;
  }): ImportedTextureLike;
  sendSharedTexture(options: { frame: never; importedSharedTexture: ImportedTextureLike }, ...args: unknown[]): Promise<void>;
}

/** What the preload receives with each import (`sendSharedTexture` args). */
export type SurfaceArgs = {
  kind: 'powermove-frame-surface';
  launch: number;
  generation: number;
  poolIndex: number;
  poolSize: number;
  width: number;
  height: number;
  format: 'bgra' | 'rgbaf16';
};

/** A window frame a surface can be sent to (`WebFrameMain`). */
export type ReceiverFrame = { readonly detached?: boolean; readonly isDestroyed?: () => boolean };

/** Finds the frame hosting the viewer that owns an interactive engine's client port. */
export type ResolveReceiver = (clientPort: number) => ReceiverFrame | null;

/**
 * Colour tags at import: the engine writes display-encoded BT.709 values
 * (or display-p3 values when the viewer's canvas is display-p3; the viewer
 * then imports with no conversion and its canvas interprets them). Half
 * floats carry values above 1.0.
 */
export function importColorSpace(format: 'bgra' | 'rgbaf16'): { primaries: 'bt709'; transfer: 'srgb' | 'srgb-hdr'; matrix: 'rgb'; range: 'full' } {
  return { primaries: 'bt709', transfer: format === 'rgbaf16' ? 'srgb-hdr' : 'srgb', matrix: 'rgb', range: 'full' };
}

type Import = {
  imported: ImportedTextureLike;
  clientPort: number;
  args: SurfaceArgs;
  sent: number;
  sendFailures: number;
  /** Main released its own reference (retired, or the launch ended). */
  released: boolean;
};

export type BridgeLaunchStats = {
  launch: number;
  policy: string;
  name: string;
  accepted: { pid: number; pidversion: number } | null;
  rejections: number;
  surfaces: number;
  imports: number;
  /** Sends per import: 1 for every import (amendment 4). */
  sends: number;
  sendFailures: number;
  retired: number;
  allReleased: number;
  held: { count: number; bytes: number };
  live: number;
  gone: boolean;
};

/** One interactive engine launch's bridge. */
export class BridgeLaunch {
  readonly env: Record<string, string>;
  private readonly imports = new Map<string, Import>();
  private stats: Omit<BridgeLaunchStats, 'held' | 'live' | 'policy' | 'name' | 'launch'> = {
    accepted: null,
    rejections: 0,
    surfaces: 0,
    imports: 0,
    sends: 0,
    sendFailures: 0,
    retired: 0,
    allReleased: 0,
    gone: false
  };
  private started = false;
  private stopped = false;

  constructor(
    readonly id: number,
    private readonly host: BridgeHostLike,
    private readonly sharedTexture: SharedTextureLike,
    private readonly resolve: ResolveReceiver,
    private readonly log: Log,
    private readonly listener: (event: BridgeEvent, launch: BridgeLaunch) => void
  ) {
    this.env = { [FRAME_BRIDGE_ENV]: host.name };
  }

  /** The engine process exists: record its identity and start receiving. */
  spawned(pid: number | undefined): void {
    if (this.started || this.stopped) return;
    if (!pid) {
      this.log('error', 'frame bridge: the interactive engine has no pid; its check-in cannot be verified');
      return;
    }
    try {
      const expected = this.host.expectChild(pid);
      this.host.start((event) => this.onEvent(event), CHECK_IN_TIMEOUT_MS);
      this.started = true;
      this.log('info', `frame bridge: waiting for pid ${expected.pid} (pidversion ${expected.pidversion}, ${this.host.policy()})`);
    } catch (error) {
      this.log('error', `frame bridge: cannot verify the interactive engine: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  /** The launch ended: stop receiving and release main's references. */
  ended(): void {
    if (this.stopped) return;
    this.stopped = true;
    this.host.stop();
    for (const entry of this.imports.values()) this.releaseImport(entry);
  }

  /** Drops everything main holds (app quit). */
  dispose(): void {
    this.ended();
    this.host.releaseAll();
  }

  status(): BridgeLaunchStats {
    return {
      launch: this.id,
      policy: this.host.policy(),
      name: this.host.name,
      ...this.stats,
      held: this.host.held(),
      live: [...this.imports.values()].filter((i) => !i.released).length
    };
  }

  private onEvent(event: BridgeEvent): void {
    try {
      this.handle(event);
    } catch (error) {
      this.log('error', `frame bridge: ${event.kind} failed: ${error instanceof Error ? error.message : String(error)}`);
    }
    this.listener(event, this);
  }

  private handle(event: BridgeEvent): void {
    switch (event.kind) {
      case 'accepted':
        this.stats.accepted = { pid: event.pid ?? -1, pidversion: event.pidversion ?? -1 };
        this.log('info', `frame bridge: the interactive engine checked in (pid ${event.pid})`);
        return;
      case 'rejected':
        this.stats.rejections++;
        this.log('warn', `frame bridge: refused a message${event.pid && event.pid > 0 ? ` from pid ${event.pid}` : ''}: ${event.reason}`);
        return;
      case 'surface':
        this.onSurface(event);
        return;
      case 'retire':
        this.retire(event.clientPort ?? -1, event.generation ?? -1);
        return;
      case 'goodbye':
        return;
      case 'gone':
        this.stats.gone = true;
        for (const entry of this.imports.values()) this.releaseImport(entry);
        return;
      case 'error':
        this.log('error', `frame bridge: ${event.reason}`);
        return;
    }
  }

  private onSurface(event: BridgeEvent): void {
    const { clientPort, generation, poolIndex, poolSize, width, height, format, ioSurface } = event;
    if (clientPort == null || generation == null || poolIndex == null || poolSize == null || width == null || height == null || !format || !ioSurface) {
      throw new Error('a surface event is incomplete');
    }
    this.stats.surfaces++;
    const key = `${generation}:${poolIndex}`;
    const previous = this.imports.get(key);
    if (previous) this.releaseImport(previous);
    let imported: ImportedTextureLike;
    try {
      imported = this.sharedTexture.importSharedTexture({
        textureInfo: { pixelFormat: format, codedSize: { width, height }, colorSpace: importColorSpace(format), handle: { ioSurface } },
        allReferencesReleased: () => {
          this.stats.allReleased++;
          this.host.releaseSurface(generation, poolIndex);
          if (this.imports.get(key)?.imported === imported) this.imports.delete(key);
        }
      });
    } catch (error) {
      // Main still holds the IOSurface; nothing will release it through
      // Chromium, so drop it here.
      this.host.releaseSurface(generation, poolIndex);
      throw error;
    }
    this.stats.imports++;
    const args: SurfaceArgs = { kind: 'powermove-frame-surface', launch: this.id, generation, poolIndex, poolSize, width, height, format };
    const entry: Import = { imported, clientPort, args, sent: 0, sendFailures: 0, released: false };
    this.imports.set(key, entry);
    this.send(entry);
  }

  /** Sends an import to the window that owns its client port, once. */
  private send(entry: Import): void {
    const frame = this.resolve(entry.clientPort);
    if (!frame || frame.isDestroyed?.() || frame.detached) {
      this.log('warn', `frame bridge: no window for client port ${entry.clientPort}; generation ${entry.args.generation} surface ${entry.args.poolIndex} not sent`);
      return;
    }
    entry.sent++;
    this.stats.sends++;
    this.sharedTexture.sendSharedTexture({ frame: frame as never, importedSharedTexture: entry.imported }, entry.args).catch((error: unknown) => {
      entry.sendFailures++;
      this.stats.sendFailures++;
      this.log('warn', `frame bridge: sending generation ${entry.args.generation} surface ${entry.args.poolIndex} failed: ${error instanceof Error ? error.message : String(error)}`);
    });
  }

  private retire(clientPort: number, generation: number): void {
    let n = 0;
    for (const entry of this.imports.values()) {
      if (entry.args.generation === generation && entry.clientPort === clientPort && !entry.released) {
        this.releaseImport(entry);
        n++;
      }
    }
    if (n) this.stats.retired += n;
  }

  private releaseImport(entry: Import): void {
    if (entry.released) return;
    entry.released = true;
    try {
      entry.imported.release();
    } catch (error) {
      this.log('warn', `frame bridge: releasing an import failed: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
}

export type FrameBridgeOptions = {
  addon: BridgeAddon;
  sharedTexture: SharedTextureLike;
  /** Unsigned development builds only (main has no Team ID). */
  allowUnsignedDev: boolean;
  resolve: ResolveReceiver;
  log: Log;
};

/** The bridge across interactive engine launches. */
export class FrameBridge {
  private nextLaunch = 1;
  private current: BridgeLaunch | null = null;
  private listeners = new Set<(event: BridgeEvent, launch: BridgeLaunch) => void>();

  constructor(private readonly options: FrameBridgeOptions) {}

  /**
   * A new interactive engine launch is about to be forked: registers its
   * bootstrap name. Returns `null` (and logs why) when the bridge cannot
   * run; the engine then starts without one and presents nothing.
   */
  prepareLaunch(): BridgeLaunch | null {
    this.current?.ended();
    try {
      const host = new this.options.addon.BridgeHost(this.options.addon.serviceName(), this.options.allowUnsignedDev);
      const launch = new BridgeLaunch(this.nextLaunch++, host, this.options.sharedTexture, this.options.resolve, this.options.log, (event, from) => {
        for (const l of this.listeners) l(event, from);
      });
      this.current = launch;
      return launch;
    } catch (error) {
      this.options.log('error', `frame bridge: cannot register: ${error instanceof Error ? error.message : String(error)}`);
      return null;
    }
  }

  get launch(): BridgeLaunch | null {
    return this.current;
  }

  listen(listener: (event: BridgeEvent, launch: BridgeLaunch) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  dispose(): void {
    this.current?.dispose();
    this.current = null;
  }
}

/** Where the pm-bridge addon is: unpacked from the asar in a packaged app,
 *  `native/` in development, or POWERMOVE_PM_BRIDGE. */
export const BRIDGE_ADDON_FILE = 'pm_bridge.node';
