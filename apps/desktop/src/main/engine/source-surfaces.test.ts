import { describe, expect, it, vi } from 'vitest';
import type { SourceVideoFrame } from '../../shared/engine-protocol';
import { SourceSurfaceService, sourceVideoByteLength, SOURCE_SURFACES_PER_WINDOW, SOURCE_SURFACE_RECLAIM_MS, SOURCE_SURFACE_BYTES_LIMIT,
  type NativeSourceSurface, type SourceSharedTexture, type SourceSurfaceTimers } from './source-surfaces';

const video: SourceVideoFrame = { format: 'nv12', primaries: 'smpte170m', transfer: 'bt709', matrix: 'smpte170m', range: 'limited' };

function harness(send: SourceSharedTexture['sendSharedTexture'] = async () => {}, allocBytes = 64) {
  const surfaces: NativeSourceSurface[] = [];
  const imports: Array<{ release: ReturnType<typeof vi.fn>; allReleased: () => void; options: Parameters<SourceSharedTexture['importSharedTexture']>[0] }> = [];
  const callbacks = new Map<number, () => void>();
  let nextTimer = 0;
  const timers: SourceSurfaceTimers = {
    set(callback, delay) { expect(delay).toBe(SOURCE_SURFACE_RECLAIM_MS); const id = ++nextTimer; callbacks.set(id, callback); return id; },
    clear(id) { callbacks.delete(id as number); }
  };
  class Surface implements NativeSourceSurface {
    readonly ioSurface = Buffer.from([1, 2, 3]);
    readonly allocBytes = allocBytes;
    release = vi.fn();
    constructor(readonly width: number, readonly height: number, readonly format: string,
      readonly bytes: Uint8Array | string, readonly fullRange: boolean) { surfaces.push(this); }
    static fromSegment(width: number, height: number, format: string, name: string, length: number, fullRange: boolean) {
      if (length !== sourceVideoByteLength(width, height, { ...video, format: format as SourceVideoFrame['format'] })) throw new Error('segment length');
      return new Surface(width, height, format, name, fullRange);
    }
  }
  const sharedTexture: SourceSharedTexture = {
    importSharedTexture(options) {
      const imported = { release: vi.fn(), allReleased: options.allReferencesReleased, options };
      imports.push(imported);
      return imported;
    },
    sendSharedTexture: send
  };
  const frame = { detached: false, isDestroyed: () => false };
  const service = new SourceSurfaceService({ addon: { SourceVideoSurface: Surface }, sharedTexture,
    resolveFrame: windowId => windowId === 7 ? frame : null, timers });
  const data = (format = video) => new Uint8Array(sourceVideoByteLength(4, 2, format));
  return { service, surfaces, imports, callbacks, frame, data };
}

describe('source video IOSurface presentation', () => {
  it('uses protocol plane lengths, including odd dimensions, and rejects invalid tags or bytes', async () => {
    expect(sourceVideoByteLength(4, 2, video)).toBe(12);
    expect(sourceVideoByteLength(3, 3, video)).toBe(17);
    expect(sourceVideoByteLength(4, 2, { ...video, format: 'nv16' })).toBe(16);
    expect(sourceVideoByteLength(4, 2, { ...video, format: 'p010le' })).toBe(24);
    const h = harness();
    await expect(h.service.present(7, 4, 2, video, new Uint8Array(11), 123456)).rejects.toThrow('plane length');
    await expect(h.service.present(7, 4, 2, { ...video, transfer: 'private-curve' }, h.data(), 123456)).rejects.toThrow('colour tags');
    await expect(h.service.present(99, 4, 2, video, h.data(), 123456)).rejects.toThrow('window is gone');
    expect(h.surfaces).toHaveLength(0);
  });

  it('fills a surface straight from the engine segment, checking its length first', async () => {
    const h = harness();
    const p010 = { ...video, format: 'p010le' as const };
    await expect(h.service.present(7, 4, 2, p010, { segment: '/pm.0123456789abcdef01234567', length: 23 }, 1)).rejects.toThrow('plane length');
    expect(h.surfaces).toHaveLength(0);
    await h.service.present(7, 4, 2, p010, { segment: '/pm.0123456789abcdef01234567', length: 24 }, 1);
    expect(h.surfaces[0]).toMatchObject({ bytes: '/pm.0123456789abcdef01234567', format: 'p010le' });
    h.service.dispose();
  });

  it('preserves a padded GPU frame visible rectangle and public display size', async () => {
    const h = harness();
    const padded = { ...video, format: 'p010le', visibleWidth: 4 };
    const descriptor = await h.service.present(7, 8, 2, padded, new Uint8Array(sourceVideoByteLength(8, 2, padded)), 123456);
    expect(descriptor).toMatchObject({ width: 4, height: 2 });
    expect(h.imports[0]!.options.textureInfo).toMatchObject({ codedSize: { width: 8, height: 2 },
      visibleRect: { x: 0, y: 0, width: 4, height: 2 } });
    h.service.dispose();
  });

  it('sends one descriptor with the decoded video tags and releases native only after all references', async () => {
    const h = harness();
    const descriptor = await h.service.present(7, 4, 2, video, h.data(), 123456);
    expect(descriptor).toMatchObject({ kind: 'powermove-source-surface', width: 4, height: 2 });
    expect(descriptor.token).toMatch(/^[0-9a-f-]{36}$/);
    expect(h.imports).toHaveLength(1);
    expect(h.imports[0]!.options.textureInfo).toMatchObject({ timestamp: 123456, pixelFormat: 'nv12', colorSpace: {
      primaries: video.primaries, transfer: video.transfer, matrix: video.matrix, range: video.range } });
    expect(h.imports[0]!.release).toHaveBeenCalledOnce();
    expect(h.surfaces[0]!.release).not.toHaveBeenCalled();
    expect(h.service.bytes).toBe(64);
    h.imports[0]!.allReleased();
    h.imports[0]!.allReleased();
    expect(h.surfaces[0]!.release).toHaveBeenCalledOnce();
    expect(h.service.bytes).toBe(0);
    expect(h.service.held).toBe(0);
  });

  it('releases main on send failure, preserving native until Chromium releases its references', async () => {
    const h = harness(async () => { throw new Error('transfer timed out'); });
    await expect(h.service.present(7, 4, 2, video, h.data(), 123456)).rejects.toThrow('transfer timed out');
    expect(h.imports[0]!.release).toHaveBeenCalledOnce();
    expect(h.surfaces[0]!.release).not.toHaveBeenCalled();
    h.imports[0]!.allReleased();
    expect(h.surfaces[0]!.release).toHaveBeenCalledOnce();
  });

  it('rejects an IOSurface whose padded allocation exceeds the global budget before import', async () => {
    const h = harness(async () => {}, SOURCE_SURFACE_BYTES_LIMIT + 1);
    await expect(h.service.present(7, 4, 2, video, h.data(), 123456)).rejects.toThrow('memory budget');
    expect(h.surfaces[0]!.release).toHaveBeenCalledOnce();
    expect(h.imports).toHaveLength(0);
    expect(h.service.bytes).toBe(0);
  });

  it('rejects a frame whose target disappears while the transfer is in flight', async () => {
    let finishSend: (() => void) | undefined;
    const h = harness(() => new Promise<void>(resolve => { finishSend = resolve; }));
    const presenting = h.service.present(7, 4, 2, video, h.data(), 123456);
    h.service.windowGone(7);
    finishSend!();
    await expect(presenting).rejects.toThrow('went away during transfer');
    expect(h.imports[0]!.release).toHaveBeenCalledOnce();
    expect(h.surfaces[0]!.release).not.toHaveBeenCalled();
    expect(h.callbacks.size).toBe(1);
    [...h.callbacks.values()][0]!();
    expect(h.surfaces[0]!.release).toHaveBeenCalledOnce();
  });

  it('bounds in-flight surfaces and reclaims a renderer-gone import after the GPU grace', async () => {
    const h = harness();
    for (let n = 0; n < SOURCE_SURFACES_PER_WINDOW; n++) await h.service.present(7, 4, 2, video, h.data(), 123456);
    await expect(h.service.present(7, 4, 2, video, h.data(), 123456)).rejects.toThrow('window budget');
    h.service.windowGone(7);
    expect(h.callbacks.size).toBe(SOURCE_SURFACES_PER_WINDOW);
    for (const callback of [...h.callbacks.values()]) callback();
    expect(h.service.held).toBe(0);
    for (const surface of h.surfaces) expect(surface.release).toHaveBeenCalledOnce();
  });
});
