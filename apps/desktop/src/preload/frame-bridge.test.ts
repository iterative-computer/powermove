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
});
