import { describe, expect, it, vi } from 'vitest';

import { FrameBridge, FRAME_BRIDGE_ENV, importColorSpace, RECLAIM_GRACE_MS, type BridgeEvent, type BridgeHostLike, type ImportedTextureLike, type SharedTextureLike } from './bridge';

type Imported = ImportedTextureLike & { released: number; allReleased: () => void; info: unknown };

function fakes(options: { allowUnsignedDev?: boolean } = {}) {
  const hosts: (BridgeHostLike & { emit(e: BridgeEvent): void; released: [number, number][]; stopped: number })[] = [];
  const addon = {
    serviceName: () => `com.powermove.engine-bridge.${'0'.repeat(31)}${hosts.length}`,
    nowUs: () => 0,
    BridgeHost: class {
      callback: ((e: BridgeEvent) => void) | null = null;
      released: [number, number][] = [];
      stopped = 0;
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
        return { count: 0, bytes: 0 };
      }
      stop() {
        this.stopped++;
      }
      releaseAll() {}
    }
  };
  const imports: Imported[] = [];
  const sends: { frame: unknown; args: unknown; texture: Imported }[] = [];
  let failSends = false;
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
      return failSends ? Promise.reject(new Error('receiver gone')) : Promise.resolve();
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
  return { bridge, hosts, imports, sends, windows, log, timers, runTimers, failSends: () => (failSends = true) };
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
