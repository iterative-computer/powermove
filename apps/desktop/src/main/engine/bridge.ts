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
 * that this module sends to that window. (Same-origin popouts draw from the
 * opener's realm, so their surfaces go to the opener's frame.)
 *
 * A page that goes away (reload, navigation, a crashed or closed renderer)
 * never reports its imports released: Electron sends the renderer's
 * release notice from a callback that does not run once the page unloads
 * (measured in the Electron integration test; the preload's `pagehide`
 * release does not reach main either). `allReferencesReleased` then never
 * fires and main would hold every surface of that page's pools for the
 * rest of the launch. So once a receiver is gone and main released its
 * own import, main drops its IOSurface reference after a grace period
 * (`RECLAIM_GRACE_MS`). Main's reference is the only one in this process
 * (the import keeps none: the retain count is 1 with an import alive), and
 * the GPU process holds its own for as long as it still needs the surface.
 */
import type { Log } from './connection';

/** Environment variable carrying the bootstrap name to the engine. */
export const FRAME_BRIDGE_ENV = 'POWERMOVE_FRAME_BRIDGE';

/** How long main waits for the engine's check-in. */
export const CHECK_IN_TIMEOUT_MS = 15_000;

/** How long a surface whose receiver is gone may wait for
 *  `allReferencesReleased` before main drops its reference anyway. */
export const RECLAIM_GRACE_MS = 2000;

/** Delays before sending an import again after a failed send (or when no
 *  window owned its client port yet). A surface that never reaches the
 *  preload leaves its pool slot undrawable: every descriptor for it would
 *  park and be released undrawn until the pool is replaced. */
export const SEND_RETRY_MS = [16, 100, 500, 2000] as const;

/** Ended launches kept while Chromium may still release their surfaces. */
export const MAX_ENDED_LAUNCHES = 4;

/** One event from pm-bridge's receive thread (`BridgeEvent`). */
export type BridgeEvent = {
  kind: 'accepted' | 'rejected' | 'surface' | 'retire' | 'remoteLayer' | 'goodbye' | 'gone' | 'error';
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
  /** `remoteLayer` (the C′ prototype): the engine's Core Animation context (0: gone) and its scale. */
  contextId?: number | null;
  scale?: number | null;
};

/** pm-bridge's `BridgeHost` (one per launch). */
export interface BridgeHostLike {
  readonly name: string;
  policy(): string;
  expectChild(pid: number): { pid: number; pidversion: number; parentPid: number };
  start(callback: (event: BridgeEvent) => void, acceptTimeoutMs: number): void;
  releaseSurface(generation: number, poolIndex: number): boolean;
  held(): { count: number; bytes: number };
  /** The retain count of a held surface's IOSurfaceRef in main (tests). */
  surfaceRetainCount?(generation: number, poolIndex: number): number | null;
  /** Returns at once; the receive thread ends on its own shortly after. */
  stop(): void;
  /** Whether the receive thread is still running (tests). */
  running?(): boolean;
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

/** The frame hosting the viewer that owns a client port, and its owner
 *  (the window's `webContents` id, for `FrameBridge.receiverGone`). */
export type Receiver = { frame: ReceiverFrame; owner: number };

/** Finds the receiver of an interactive engine's client port. */
export type ResolveReceiver = (clientPort: number) => Receiver | null;

/** Timers (injected by tests). */
export type BridgeTimers = { set(fn: () => void, ms: number): unknown; clear(handle: unknown): void };

const defaultTimers: BridgeTimers = {
  set: (fn, ms) => {
    const handle = setTimeout(fn, ms);
    (handle as { unref?: () => void }).unref?.();
    return handle;
  },
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>)
};

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
  /** Retries scheduled so far, and the pending retry timer. */
  retries: number;
  retry: unknown;
  /** Main released its own reference (retired, or the launch ended). */
  released: boolean;
  /** The window it was sent to, and whether that page is gone. */
  owner: number | null;
  receiverGone: boolean;
  reclaim: unknown;
};

export type BridgeLaunchStats = {
  launch: number;
  policy: string;
  name: string;
  accepted: { pid: number; pidversion: number } | null;
  rejections: number;
  surfaces: number;
  imports: number;
  /** Sends per import: 1 for every import (amendment 4), plus retries. */
  sends: number;
  sendFailures: number;
  /** Sends repeated after a failure; imports that never reached a window. */
  sendRetries: number;
  undelivered: number;
  retired: number;
  allReleased: number;
  /** Surfaces dropped after their receiver went away without releasing them. */
  reclaimed: number;
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
    sendRetries: 0,
    undelivered: 0,
    retired: 0,
    allReleased: 0,
    reclaimed: 0,
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
    private readonly listener: (event: BridgeEvent, launch: BridgeLaunch) => void,
    private readonly timers: BridgeTimers = defaultTimers
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

  /** The launch ended: stop receiving and release main's references.
   *  Never blocks: pm-bridge's receive thread sees the stop within 50 ms
   *  (100 ms after the check-in), even when the engine exited before it
   *  checked in, and events it still had in flight are ignored here. */
  ended(): void {
    if (this.stopped) return;
    this.stopped = true;
    this.host.stop();
    for (const entry of this.imports.values()) this.releaseImport(entry);
  }

  /** Drops everything main holds (app quit, or an old ended launch). */
  dispose(): void {
    this.ended();
    for (const entry of this.imports.values()) if (entry.reclaim !== null) this.timers.clear(entry.reclaim);
    this.imports.clear();
    this.host.releaseAll();
  }

  /** Whether the launch ended and main no longer holds any of its surfaces. */
  get finished(): boolean {
    return this.stopped && this.host.held().count === 0;
  }

  /** A window's page went away (reload, navigation, crash, close): what was
   *  sent to it will never be reported released. */
  receiverGone(owner: number): void {
    for (const entry of this.imports.values()) {
      if (entry.owner !== owner || entry.receiverGone) continue;
      entry.receiverGone = true;
      if (entry.retry !== null) {
        this.timers.clear(entry.retry);
        entry.retry = null;
      }
      this.scheduleReclaim(entry);
    }
  }

  private scheduleReclaim(entry: Import): void {
    if (!entry.released || !entry.receiverGone || entry.reclaim !== null) return;
    const key = `${entry.args.generation}:${entry.args.poolIndex}`;
    entry.reclaim = this.timers.set(() => {
      entry.reclaim = null;
      if (this.imports.get(key) !== entry) return;
      this.imports.delete(key);
      this.host.releaseSurface(entry.args.generation, entry.args.poolIndex);
      this.stats.reclaimed++;
    }, RECLAIM_GRACE_MS);
  }

  /** A held surface's IOSurfaceRef retain count in main (tests: main's
   *  own reference plus the ones its imports keep). */
  retainCount(generation: number, poolIndex: number): number | null {
    return this.host.surfaceRetainCount?.(generation, poolIndex) ?? null;
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
    // Events the receive thread had in flight when the launch ended.
    if (this.stopped) return;
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
      case 'remoteLayer':
        // The C′ prototype (docs/engine/bridge.md): the engine presents
        // playback through a remote layer. Hosting it needs private API, so
        // only the measurement harness does (FrameBridge listeners).
        this.log('info', `frame bridge: remote layer context ${event.contextId} for port ${event.clientPort} (C′ prototype)`);
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
          const entry = this.imports.get(key);
          if (entry?.imported !== imported) return;
          this.stats.allReleased++;
          if (entry.reclaim !== null) this.timers.clear(entry.reclaim);
          this.imports.delete(key);
          this.host.releaseSurface(generation, poolIndex);
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
    const entry: Import = { imported, clientPort, args, sent: 0, sendFailures: 0, retries: 0, retry: null, released: false, owner: null, receiverGone: false, reclaim: null };
    this.imports.set(key, entry);
    this.send(entry);
  }

  /** Sends an import to the window that owns its client port: once, or
   *  again after a failed send (`SEND_RETRY_MS`) while the import is live
   *  and its page is still there. */
  private send(entry: Import): void {
    entry.retry = null;
    if (entry.released || entry.receiverGone || this.stopped) return;
    const what = `generation ${entry.args.generation} surface ${entry.args.poolIndex}`;
    const receiver = this.resolve(entry.clientPort);
    const frame = receiver?.frame;
    if (!receiver || !frame || frame.isDestroyed?.() || frame.detached) {
      this.log('warn', `frame bridge: no window for client port ${entry.clientPort}; ${what} not sent`);
      this.retrySend(entry);
      return;
    }
    entry.owner = receiver.owner;
    entry.sent++;
    this.stats.sends++;
    this.sharedTexture.sendSharedTexture({ frame: frame as never, importedSharedTexture: entry.imported }, entry.args).catch((error: unknown) => {
      entry.sendFailures++;
      this.stats.sendFailures++;
      this.log('warn', `frame bridge: sending ${what} failed: ${error instanceof Error ? error.message : String(error)}`);
      this.retrySend(entry);
    });
  }

  private retrySend(entry: Import): void {
    if (entry.released || entry.receiverGone || this.stopped || entry.retry !== null) return;
    const delay = SEND_RETRY_MS[entry.retries];
    if (delay === undefined) {
      this.stats.undelivered++;
      this.log('error', `frame bridge: generation ${entry.args.generation} surface ${entry.args.poolIndex} never reached client port ${entry.clientPort}; that slot stays undrawn until the pool is replaced`);
      return;
    }
    entry.retries++;
    this.stats.sendRetries++;
    entry.retry = this.timers.set(() => this.send(entry), delay);
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
    if (entry.retry !== null) {
      this.timers.clear(entry.retry);
      entry.retry = null;
    }
    try {
      entry.imported.release();
    } catch (error) {
      this.log('warn', `frame bridge: releasing an import failed: ${error instanceof Error ? error.message : String(error)}`);
    }
    this.scheduleReclaim(entry);
  }
}

export type FrameBridgeOptions = {
  addon: BridgeAddon;
  sharedTexture: SharedTextureLike;
  /** Unsigned development builds only (main has no Team ID). */
  allowUnsignedDev: boolean;
  resolve: ResolveReceiver;
  log: Log;
  timers?: BridgeTimers;
};

/** The bridge across interactive engine launches. */
export class FrameBridge {
  private nextLaunch = 1;
  private current: BridgeLaunch | null = null;
  /** Ended launches whose surfaces Chromium may still hold. */
  private previous: BridgeLaunch[] = [];
  private listeners = new Set<(event: BridgeEvent, launch: BridgeLaunch) => void>();

  constructor(private readonly options: FrameBridgeOptions) {}

  /**
   * A new interactive engine launch is about to be forked: registers its
   * bootstrap name. Returns `null` (and logs why) when the bridge cannot
   * run; the engine then starts without one and presents nothing.
   */
  prepareLaunch(): BridgeLaunch | null {
    if (this.current) {
      this.current.ended();
      this.previous.push(this.current);
      // An ended launch's imports are released; keep it while Chromium may
      // still report (or a reclaim may still drop) its surfaces. Launches
      // past the newest few are disposed (main drops its references) rather
      // than left to the garbage collector.
      const keep: BridgeLaunch[] = [];
      for (const launch of this.previous) if (!launch.finished) keep.push(launch);
      for (const launch of keep.splice(0, Math.max(0, keep.length - MAX_ENDED_LAUNCHES))) launch.dispose();
      this.previous = keep;
    }
    try {
      const host = new this.options.addon.BridgeHost(this.options.addon.serviceName(), this.options.allowUnsignedDev);
      const launch = new BridgeLaunch(
        this.nextLaunch++,
        host,
        this.options.sharedTexture,
        this.options.resolve,
        this.options.log,
        (event, from) => {
          for (const l of this.listeners) l(event, from);
        },
        this.options.timers
      );
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

  /** A window's page went away: its surfaces will never be reported released. */
  receiverGone(owner: number): void {
    this.current?.receiverGone(owner);
    for (const launch of this.previous) launch.receiverGone(owner);
  }

  listen(listener: (event: BridgeEvent, launch: BridgeLaunch) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  dispose(): void {
    this.current?.dispose();
    this.current = null;
    for (const launch of this.previous) launch.dispose();
    this.previous = [];
  }
}

/** Where the pm-bridge addon is: unpacked from the asar in a packaged app,
 *  `native/` in development, or POWERMOVE_PM_BRIDGE. */
export const BRIDGE_ADDON_FILE = 'pm_bridge.node';
