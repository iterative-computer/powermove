import { describe, expect, it, vi } from 'vitest';
import { installSourceSurfaceReceiver, MAX_SOURCE_IMPORTS, SOURCE_IMPORT_TTL_MS, type SourceTimers } from './source-surfaces';
import type { ImportedLike } from './frame-bridge';

function harness() {
  const callbacks = new Map<number, () => void>();
  let next = 0;
  const timers: SourceTimers = {
    set(callback) { const id = ++next; callbacks.set(id, callback); return id; },
    clear(handle) { callbacks.delete(handle as number); }
  };
  const pageListeners: Array<() => void> = [];
  const notices: string[] = [];
  const receiver = installSourceSurfaceReceiver({ addEventListener(_type, listener) { pageListeners.push(listener); } },
    async id => { notices.push(id); }, timers);
  function imported(id: string) {
    const frame = { close: vi.fn() } as unknown as VideoFrame;
    const item = { textureId: id, frame, getVideoFrame: vi.fn(() => frame), release: vi.fn(),
      subtle: { release: vi.fn((_callback?: () => unknown) => {}) }
    };
    return item;
  }
  const descriptor = (token: string) => ({ kind: 'powermove-source-surface', token, width: 320, height: 180 });
  const token = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
  return { receiver, imported, descriptor, token, callbacks, notices, pagehide: () => pageListeners.forEach(callback => callback()) };
}

describe('one-shot source texture receiver', () => {
  it('takes one VideoFrame but retains its import until explicit discard after frame use', async () => {
    const h = harness(), texture = h.imported('texture-1'), token = h.token(1);
    h.receiver.receive(texture, h.descriptor(token));
    expect(h.receiver.sourceFrame(token)).toBe(texture.frame);
    expect(h.receiver.sourceFrame(token)).toBeNull();
    expect(texture.getVideoFrame).toHaveBeenCalledTimes(1);
    expect(texture.subtle.release).not.toHaveBeenCalled();
    expect(h.receiver.stats()).toMatchObject({ held: 1, received: 1, released: 0 });
    h.receiver.discardSource(token);
    h.receiver.discardSource(token);
    expect(texture.subtle.release).toHaveBeenCalledTimes(1);
    expect(texture.subtle.release).toHaveBeenCalledWith();
    expect(h.notices).toEqual([]);
    await Promise.resolve();
    expect(h.notices).toEqual(['texture-1']);
    expect(texture.frame.close).not.toHaveBeenCalled();
    expect(h.receiver.stats()).toMatchObject({ held: 0, received: 1, released: 1 });
  });

  it('drops an import immediately when getVideoFrame fails', async () => {
    const h = harness(), texture = h.imported('bad-frame'), token = h.token(3);
    texture.getVideoFrame.mockImplementation(() => { throw new Error('frame import failed'); });
    h.receiver.receive(texture, h.descriptor(token));
    expect(h.receiver.sourceFrame(token)).toBeNull();
    expect(h.receiver.sourceFrame(token)).toBeNull();
    expect(h.receiver.stats()).toMatchObject({ held: 0, received: 1, released: 1, errors: ['frame import failed'] });
    expect(texture.getVideoFrame).toHaveBeenCalledOnce();
    expect(texture.subtle.release).toHaveBeenCalledOnce();
    expect(texture.subtle.release).toHaveBeenCalledWith();
    await Promise.resolve();
    expect(h.notices).toEqual(['bad-frame']);
  });

  it('discards stale imports at 10 seconds and all pending imports on pagehide', async () => {
    const h = harness(), a = h.imported('a'), b = h.imported('b');
    h.receiver.receive(a, h.descriptor(h.token(1)));
    expect(h.callbacks.size).toBe(1);
    expect(SOURCE_IMPORT_TTL_MS).toBe(10_000);
    [...h.callbacks.values()][0]!();
    expect(h.receiver.sourceFrame(h.token(1))).toBeNull();
    h.receiver.receive(b, h.descriptor(h.token(2)));
    h.pagehide();
    expect(h.receiver.stats()).toMatchObject({ held: 0, expired: 1, released: 2 });
    await Promise.resolve();
    expect(h.notices).toEqual(['a', 'b']);
  });

  it('releases a duplicate transfer without a second notice or release callback', async () => {
    const h = harness(), original = h.imported('same'), duplicate = h.imported('same'), token = h.token(5);
    h.receiver.receive(original, h.descriptor(token));
    h.receiver.receive(duplicate, h.descriptor(token));
    expect(h.receiver.stats()).toMatchObject({ held: 1, received: 2, released: 0 });
    expect(duplicate.subtle.release).toHaveBeenCalledOnce();
    expect(duplicate.subtle.release).toHaveBeenCalledWith();
    await Promise.resolve();
    expect(h.notices).toEqual([]);
    h.receiver.discardSource(token);
    await Promise.resolve();
    expect(original.subtle.release).toHaveBeenCalledWith();
    expect(h.notices).toEqual(['same']);
  });

  it('bounds held imports and rejects malformed descriptors', () => {
    const h = harness();
    const first = h.imported('first');
    h.receiver.receive(first, h.descriptor(h.token(0)));
    for (let n = 1; n <= MAX_SOURCE_IMPORTS; n++) h.receiver.receive(h.imported(`t${n}`), h.descriptor(h.token(n)));
    expect(h.receiver.stats().held).toBe(MAX_SOURCE_IMPORTS);
    expect(h.receiver.sourceFrame(h.token(0))).toBeNull();
    expect(first.subtle.release).toHaveBeenCalledOnce();
    const malformed = h.imported('bad');
    h.receiver.receive(malformed as ImportedLike, { ...h.descriptor(h.token(99)), width: 0 });
    expect(malformed.subtle.release).toHaveBeenCalledOnce();
    expect(h.receiver.stats().errors).toHaveLength(1);
  });

  it('routes source descriptors before the reusable viewer pool receiver', async () => {
    const h = harness(), texture = h.imported('source'), token = h.token(9);
    const { installFrameBridgeReceiver } = await import('./frame-bridge');
    let callback: ((data: { importedSharedTexture: ImportedLike }, args: unknown) => Promise<void>) | null = null;
    const bridge = installFrameBridgeReceiver({ setSharedTextureReceiver(receive) { callback = receive; } }, null, null, h.receiver);
    await callback!({ importedSharedTexture: texture }, h.descriptor(token));
    expect(bridge.sourceFrame(token)).toBe(texture.frame);
    expect(texture.subtle.release).not.toHaveBeenCalled();
    bridge.discardSource(token);
    expect(texture.subtle.release).toHaveBeenCalledOnce();
    expect(bridge.stats().errors).toEqual([]);
  });
});
