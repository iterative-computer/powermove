import { describe, expect, it, vi } from 'vitest';

import { FrameBridge, FRAME_BRIDGE_ENV, importColorSpace, MAX_ENDED_LAUNCHES, RECLAIM_GRACE_MS, SEND_RETRY_MS, type BridgeEvent, type BridgeHostLike, type ImportedTextureLike, type SharedTextureLike } from './bridge';

type Imported = ImportedTextureLike & { released: number; allReleased: () => void; info: unknown };

function fakes(options: { allowUnsignedDev?: boolean } = {}) {
  const hosts: (BridgeHostLike & { emit(e: BridgeEvent): void; released: [number, number][]; stopped: number; heldCount: number; releasedAll: number })[] = [];
  const addon = {
    serviceName: () => `com.powermove.engine-bridge.${'0'.repeat(31)}${hosts.length}`,
    nowUs: () => 0,
    BridgeHost: class {
      callback: ((e: BridgeEvent) => void) | null = null;
      released: [number, number][] = [];
      stopped = 0;
      heldCount = 0;
      releasedAll = 0;
      expected: number | null = null;
      constructor(readonly name: string, readonly allow: boolean) {
        hosts.push(this as never);
      }
      policy() {
        return this.allow ? 'unsigned-dev-bypass' : 'team-id:ABCDE12345';
      }
      expectChild(pid: number) {
        this.expected = pid;
        return { pid, pidversion: 7, parentPid: 1 };
      }
      start(callback: (e: BridgeEvent) => void) {
        this.callback = callback;
      }
      emit(event: BridgeEvent) {
        this.callback?.(event);
      }
      releaseSurface(generation: number, poolIndex: number) {
        this.released.push([generation, poolIndex]);
        return true;
      }
      held() {
        return { count: this.heldCount, bytes: 0 };
      }
      stop() {
        this.stopped++;
      }
      releaseAll() {
        this.releasedAll++;
        this.heldCount = 0;
      }
    }
  };
  const imports: Imported[] = [];
  const sends: { frame: unknown; args: unknown; texture: Imported }[] = [];
  let failSends = 0;
  const sharedTexture: SharedTextureLike = {
    importSharedTexture(options) {
      const imported: Imported = {
        released: 0,
        info: options.textureInfo,
        allReleased: options.allReferencesReleased,
        release() {
          this.released++;
        }
      };
      imports.push(imported);
      return imported;
    },
    sendSharedTexture(options, ...args) {
      sends.push({ frame: options.frame, args: args[0], texture: options.importedSharedTexture as Imported });
      if (failSends > 0) {
        failSends--;
        return Promise.reject(new Error('receiver gone'));
      }
      return Promise.resolve();
    }
  };
  const windows = new Map<number, { frame: { detached?: boolean }; owner: number }>([[11, { frame: {}, owner: 3 }]]);
  const log = vi.fn();
  const timers: { fn: () => void; ms: number; cleared: boolean }[] = [];
  const bridge = new FrameBridge({
    addon: addon as never,
    sharedTexture,
    allowUnsignedDev: options.allowUnsignedDev ?? true,
    resolve: (port) => windows.get(port) ?? null,
    log,
    timers: {
      set: (fn, ms) => {
        const timer = { fn, ms, cleared: false };
        timers.push(timer);
        return timer;
      },
      clear: (handle) => {
        (handle as { cleared: boolean }).cleared = true;
      }
    }
  });
  const runTimers = () => {
    for (const timer of timers.splice(0)) if (!timer.cleared) timer.fn();
  };
  return { bridge, hosts, imports, sends, windows, log, timers, runTimers, failSends: (n = Infinity) => (failSends = n) };
}

const surface = (generation: number, poolIndex: number, clientPort = 11, format: 'bgra' | 'rgbaf16' = 'bgra'): BridgeEvent => ({
  kind: 'surface',
  clientPort,
  generation,
  poolIndex,
  poolSize: 6,
  width: 1280,
  height: 720,
  format,
  bytesPerRow: 5120,
  ioSurface: Buffer.alloc(8, 1)
});

describe('FrameBridge', () => {
  it('passes a per-launch name to the engine and verifies the spawned child', () => {
    const { bridge, hosts } = fakes();
    const launch = bridge.prepareLaunch()!;
    expect(launch.env[FRAME_BRIDGE_ENV]).toBe(hosts[0]!.name);
    launch.spawned(4242);
    expect((hosts[0] as unknown as { expected: number }).expected).toBe(4242);
    // A second launch ends the first.
    bridge.prepareLaunch();
    expect(hosts[0]!.stopped).toBe(1);
  });

  it('imports each surface once and sends it once to the window owning the port', () => {
    const { bridge, hosts, imports, sends } = fakes();
    const launch = bridge.prepareLaunch()!;
    launch.spawned(1);
    hosts[0]!.emit({ kind: 'accepted', pid: 1, pidversion: 7 });
    for (let i = 0; i < 6; i++) hosts[0]!.emit(surface(1, i));
    expect(imports).toHaveLength(6);
    expect(sends).toHaveLength(6);
    expect(imports[0]!.info).toMatchObject({ pixelFormat: 'bgra', codedSize: { width: 1280, height: 720 }, colorSpace: importColorSpace('bgra') });
    expect(sends[3]!.args).toEqual({ kind: 'powermove-frame-surface', launch: 1, generation: 1, poolIndex: 3, poolSize: 6, width: 1280, height: 720, format: 'bgra' });
    expect(launch.status()).toMatchObject({ surfaces: 6, imports: 6, sends: 6, live: 6, accepted: { pid: 1, pidversion: 7 } });
  });

  it('releases main\'s references on retire and frees the surface when every reference is released', () => {
    const { bridge, hosts, imports } = fakes();
    const launch = bridge.prepareLaunch()!;
    launch.spawned(1);
    hosts[0]!.emit(surface(1, 0));
    hosts[0]!.emit(surface(1, 1));
    hosts[0]!.emit(surface(2, 0, 11, 'rgbaf16'));
    hosts[0]!.emit({ kind: 'retire', clientPort: 11, generation: 1 });
    expect(imports.map((i) => i.released)).toEqual([1, 1, 0]);
    expect(hosts[0]!.released).toEqual([]);
    imports[0]!.allReleased();
    imports[1]!.allReleased();
    expect(hosts[0]!.released).toEqual([[1, 0], [1, 1]]);
    expect(launch.status()).toMatchObject({ retired: 2, allReleased: 2, live: 1 });
    expect(imports[2]!.info).toMatchObject({ colorSpace: { transfer: 'srgb-hdr' } });
  });

  it('keeps an import it could not send, and logs a failed send', async () => {
    const { bridge, hosts, sends, log, failSends } = fakes();
    const launch = bridge.prepareLaunch()!;
    launch.spawned(1);
    hosts[0]!.emit(surface(1, 0, 99));
    expect(sends).toHaveLength(0);
    expect(log).toHaveBeenCalledWith('warn', expect.stringContaining('no window for client port 99'));
    failSends();
    hosts[0]!.emit(surface(1, 1));
    await new Promise((r) => setTimeout(r, 0));
    expect(launch.status().sendFailures).toBe(1);
  });

  it('sends an import again after a failed send, until it reaches the window', async () => {
    const { bridge, hosts, sends, timers, runTimers, failSends } = fakes();
    const launch = bridge.prepareLaunch()!;
    launch.spawned(1);
    failSends(2);
    hosts[0]!.emit(surface(1, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(timers.map((t) => t.ms)).toEqual([SEND_RETRY_MS[0]]);
    runTimers();
    await new Promise((r) => setTimeout(r, 0));
    expect(timers.map((t) => t.ms)).toEqual([SEND_RETRY_MS[1]]);
    runTimers();
    await new Promise((r) => setTimeout(r, 0));
    expect(timers).toHaveLength(0);
    expect(sends).toHaveLength(3);
    expect(sends.every((s) => s.texture === sends[0]!.texture)).toBe(true);
    expect(launch.status()).toMatchObject({ sendFailures: 2, sendRetries: 2, undelivered: 0 });
  });

  it('gives up after the last retry, and never retries a released import', async () => {
    const { bridge, hosts, sends, timers, runTimers, failSends, log } = fakes();
    const launch = bridge.prepareLaunch()!;
    launch.spawned(1);
    failSends();
    hosts[0]!.emit(surface(1, 0));
    for (let i = 0; i <= SEND_RETRY_MS.length; i++) {
      await new Promise((r) => setTimeout(r, 0));
      runTimers();
    }
    await new Promise((r) => setTimeout(r, 0));
    expect(sends).toHaveLength(SEND_RETRY_MS.length + 1);
    expect(launch.status()).toMatchObject({ undelivered: 1, sendRetries: SEND_RETRY_MS.length });
    expect(log).toHaveBeenCalledWith('error', expect.stringContaining('never reached client port 11'));
    // A retry pending when the pool is retired is cancelled.
    hosts[0]!.emit(surface(2, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(timers.filter((t) => !t.cleared)).toHaveLength(1);
    hosts[0]!.emit({ kind: 'retire', clientPort: 11, generation: 2 });
    expect(timers.filter((t) => !t.cleared)).toHaveLength(0);
  });

  it('ignores events the receive thread had in flight when the launch ended', () => {
    const { bridge, hosts, imports } = fakes();
    const launch = bridge.prepareLaunch()!;
    launch.spawned(1);
    launch.ended();
    hosts[0]!.emit(surface(1, 0));
    expect(imports).toHaveLength(0);
    expect(launch.status().surfaces).toBe(0);
  });

  it('disposes ended launches beyond the newest few instead of leaving them to the garbage collector', () => {
    const { bridge, hosts } = fakes();
    const total = MAX_ENDED_LAUNCHES + 3;
    for (let i = 0; i < total; i++) {
      bridge.prepareLaunch()!.spawned(1);
      // Every launch still holds a surface Chromium has not released.
      hosts[i]!.heldCount = 1;
    }
    bridge.prepareLaunch();
    // The current launch plus MAX_ENDED_LAUNCHES ended ones are kept; the
    // oldest ended launches were disposed (main dropped their references).
    const disposed = hosts.filter((h) => h.releasedAll > 0).length;
    expect(disposed).toBe(total - MAX_ENDED_LAUNCHES);
    expect(hosts.slice(0, disposed).every((h) => h.releasedAll === 1)).toBe(true);
    // A launch that holds nothing any more is simply dropped.
    for (const h of hosts) h.heldCount = 0;
    bridge.prepareLaunch();
    expect(hosts.filter((h) => h.releasedAll > 0).length).toBe(disposed);
  });

  it('releases everything when the engine is gone and on launch end', () => {
    const { bridge, hosts, imports } = fakes();
    const launch = bridge.prepareLaunch()!;
    launch.spawned(1);
    hosts[0]!.emit(surface(1, 0));
    hosts[0]!.emit({ kind: 'gone' });
    expect(imports[0]!.released).toBe(1);
    launch.ended();
    expect(imports[0]!.released).toBe(1);
    expect(hosts[0]!.stopped).toBe(1);
  });

  it('does not start without a pid and logs refused check-ins', () => {
    const { bridge, hosts, log } = fakes();
    const launch = bridge.prepareLaunch()!;
    launch.spawned(undefined);
    expect(log).toHaveBeenCalledWith('error', expect.stringContaining('no pid'));
    launch.spawned(5);
    hosts[0]!.emit({ kind: 'rejected', pid: 77, reason: 'sender pid 77 is not the engine pid 5' });
    expect(launch.status().rejections).toBe(1);
  });

  it('drops the surfaces of a page that went away once main released them, unless Chromium reports them first', () => {
    const { bridge, hosts, imports, timers, runTimers } = fakes();
    const launch = bridge.prepareLaunch()!;
    launch.spawned(1);
    for (let i = 0; i < 3; i++) hosts[0]!.emit(surface(1, i));
    // The page reloads: nothing is dropped while main still holds its imports.
    bridge.receiverGone(3);
    expect(timers).toHaveLength(0);
    // The engine retires the old page's pool (its port closed).
    hosts[0]!.emit({ kind: 'retire', clientPort: 11, generation: 1 });
    expect(timers.map((t) => t.ms)).toEqual([RECLAIM_GRACE_MS, RECLAIM_GRACE_MS, RECLAIM_GRACE_MS]);
    // One surface's release notice did arrive in time.
    imports[2]!.allReleased();
    runTimers();
    expect(hosts[0]!.released).toEqual([[1, 2], [1, 0], [1, 1]]);
    expect(launch.status()).toMatchObject({ allReleased: 1, reclaimed: 2, live: 0 });
    // A late notice for a reclaimed surface changes nothing.
    imports[0]!.allReleased();
    expect(hosts[0]!.released).toHaveLength(3);
  });

  it('keeps the surfaces of other windows and of live pages', () => {
    const { bridge, hosts, timers, windows } = fakes();
    windows.set(12, { frame: {}, owner: 4 });
    const launch = bridge.prepareLaunch()!;
    launch.spawned(1);
    hosts[0]!.emit(surface(1, 0));
    hosts[0]!.emit(surface(2, 0, 12));
    hosts[0]!.emit({ kind: 'retire', clientPort: 11, generation: 1 });
    hosts[0]!.emit({ kind: 'retire', clientPort: 12, generation: 2 });
    bridge.receiverGone(4);
    expect(timers).toHaveLength(1);
    expect(launch.status().live).toBe(0);
  });
});
