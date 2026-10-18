import { describe, expect, it, vi } from 'vitest';

import { installFrameBridgeReceiver, type ImportedLike, type ReceiverModule } from './frame-bridge';

function harness() {
  let receiver: Parameters<ReceiverModule['setSharedTextureReceiver']>[0] | null = null;
  const hide: (() => void)[] = [];
  const api = installFrameBridgeReceiver({ setSharedTextureReceiver: (cb) => (receiver = cb) }, { addEventListener: (_type, listener) => hide.push(listener) });
  const imported = (): ImportedLike & { released: number; frames: number } => {
    const self = {
      released: 0,
      frames: 0,
      getVideoFrame: () => {
        self.frames++;
        return { close() {} } as unknown as VideoFrame;
      },
      release: () => {
        self.released++;
      }
    };
    return self;
  };
  const send = (texture: ImportedLike, args: unknown) => receiver!({ importedSharedTexture: texture }, args);
  const args = (launch: number, generation: number, poolIndex: number) => ({
    kind: 'powermove-frame-surface',
    launch,
    generation,
    poolIndex,
    poolSize: 8,
    width: 64,
    height: 32,
    format: 'bgra'
  });
  const pagehide = () => hide.forEach((listener) => listener());
  return { api, imported, send, args, pagehide };
}

describe('frame bridge receiver', () => {
  it('keeps each import and makes a VideoFrame per request', async () => {
    const { api, imported, send, args } = harness();
    const listener = vi.fn();
    api.onSurface(listener);
    const a = imported();
    await send(a, args(1, 1, 3));
    expect(listener).toHaveBeenCalledWith(1, 3);
    expect(api.has(1, 3)).toBe(true);
    expect(api.videoFrame(1, 3)).not.toBeNull();
    expect(api.videoFrame(1, 3)).not.toBeNull();
    expect(a.frames).toBe(2);
    expect(api.videoFrame(1, 4)).toBeNull();
    expect(api.stats()).toMatchObject({ launch: 1, held: 1, received: 1, videoFrames: 2, misses: 1 });
  });

  it('releases a retired generation and everything of an older launch', async () => {
    const { api, imported, send, args } = harness();
    const g1 = [imported(), imported()];
    await send(g1[0]!, args(1, 1, 0));
    await send(g1[1]!, args(1, 1, 1));
    const g2 = imported();
    await send(g2, args(1, 2, 0));
    expect(api.retire(1)).toBe(2);
    expect(g1.map((t) => t.released)).toEqual([1, 1]);
    expect(api.has(1, 0)).toBe(false);
    // The engine restarted: launch 2's generation 1 replaces everything.
    const fresh = imported();
    await send(fresh, args(2, 1, 0));
    expect(g2.released).toBe(1);
    expect(api.has(1, 0)).toBe(true);
    // A late surface of the old launch is released at once.
    const late = imported();
    await send(late, args(1, 3, 0));
    expect(late.released).toBe(1);
    expect(api.stats().held).toBe(1);
  });

  it('refuses textures without frame surface arguments', async () => {
    const { api, imported, send, args } = harness();
    const stray = imported();
    await send(stray, { ...args(1, 1, 9), poolIndex: 9, poolSize: 8 });
    await send(imported(), undefined);
    expect(stray.released).toBe(1);
    expect(api.stats()).toMatchObject({ held: 0, received: 0 });
    expect(api.stats().errors).toHaveLength(2);
  });

  it('releases every import when the page goes away (reload, navigation, close)', async () => {
    const { api, imported, send, args, pagehide } = harness();
    const a = imported();
    const b = imported();
    await send(a, args(1, 1, 0));
    await send(b, args(1, 2, 0));
    pagehide();
    expect([a.released, b.released]).toEqual([1, 1]);
    expect(api.stats().held).toBe(0);
  });

  /** Electron 44's renderer import: an id, the renderer's handle, and the notice its `release()` sends. */
  function electronImport(id: string, notices: string[]) {
    const callbacks: (() => unknown)[] = [];
    const self = {
      textureId: id,
      handleReleases: 0,
      getVideoFrame: () => ({ close() {} }) as unknown as VideoFrame,
      subtle: {
        release: (callback: () => unknown) => {
          self.handleReleases++;
          callbacks.push(callback);
        }
      },
      // Electron's own release: the renderer's handle, then an unhandled notice.
      release: () => {
        self.handleReleases++;
        notices.push(`unhandled:${id}`);
      },
      /** The GPU finished reading: Electron runs the release callbacks. */
      gpuDone: () => Promise.all(callbacks.splice(0).map((cb) => cb()))
    };
    return self;
  }

  function electronHarness(known: Set<string>) {
    let receiver: Parameters<ReceiverModule['setSharedTextureReceiver']>[0] | null = null;
    const notices: string[] = [];
    const api = installFrameBridgeReceiver({ setSharedTextureReceiver: (cb) => (receiver = cb) }, null, async (id) => {
      notices.push(id);
      // Main's handler throws for a texture it no longer has (Electron 44).
      if (!known.has(id)) throw new Error(`Error invoking remote method 'IMPORT_SHARED_TEXTURE_RELEASE_RENDERER_TO_MAIN': Error: Shared texture with id ${id} not found`);
    });
    const args = (generation: number, poolIndex: number) => ({ kind: 'powermove-frame-surface', launch: 1, generation, poolIndex, poolSize: 8, width: 64, height: 32, format: 'bgra' });
    const send = (texture: ImportedLike, a: unknown) => receiver!({ importedSharedTexture: texture }, a);
    return { api, notices, args, send };
  }

  it('keeps one copy of a texture main sent twice and tells main once (a timed-out transfer that landed, D3)', async () => {
    const known = new Set(['t1']);
    const { api, notices, args, send } = electronHarness(known);
    const first = electronImport('t1', notices);
    const again = electronImport('t1', notices);
    await send(first, args(1, 0));
    await send(again, args(1, 0));
    // The copy lets go of its renderer handle, without a notice.
    expect(again.handleReleases).toBe(1);
    await again.gpuDone();
    expect(notices).toEqual([]);
    expect(api.stats()).toMatchObject({ held: 1, duplicates: 1, released: 0 });
    expect(api.videoFrame(1, 0)).not.toBeNull();
    // Retiring the generation sends the one notice main counted.
    expect(api.retire(1)).toBe(1);
    await first.gpuDone();
    expect(notices).toEqual(['t1']);
    expect(api.stats()).toMatchObject({ held: 0, released: 1, lateReleases: 0, errors: [] });
  });

  it('counts a release notice main can no longer place instead of failing the page', async () => {
    const { api, notices, args, send } = electronHarness(new Set());
    const gone = electronImport('t2', notices);
    await send(gone, args(1, 0));
    api.retire(1);
    // No unhandled rejection: the notice's failure is handled and counted.
    await expect(gone.gpuDone()).resolves.toBeDefined();
    expect(notices).toEqual(['t2']);
    expect(api.stats()).toMatchObject({ released: 1, lateReleases: 1, errors: [] });
  });

  it('a different texture for the same slot replaces the held one with a notice', async () => {
    const known = new Set(['a', 'b']);
    const { api, notices, args, send } = electronHarness(known);
    const a = electronImport('a', notices);
    const b = electronImport('b', notices);
    await send(a, args(1, 0));
    await send(b, args(1, 0));
    await a.gpuDone();
    expect(notices).toEqual(['a']);
    expect(api.stats()).toMatchObject({ held: 1, duplicates: 0, released: 1 });
  });
});
