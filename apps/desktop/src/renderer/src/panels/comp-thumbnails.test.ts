// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createCompThumbnails } from './comp-thumbnails.svelte';

afterEach(() => vi.useRealTimers());

describe('composition thumbnails with the new engine on', () => {
  it('draw through the engine and encode only once nothing plays or gestures', async () => {
    vi.useFakeTimers();
    const comp = { id: 'c1', compId: 'c1', w: 640, h: 360, dur: 2, layers: [] };
    const toDataURL = vi.fn(() => 'data:image/jpeg;thumb');
    const snapshot = { width: 320, height: 180 };
    const engine = { thumbnails: { request: vi.fn(async () => snapshot) }, toCanvas: vi.fn(() => ({ toDataURL })) };
    const PM: Record<string, any> = { proj: comp, Comps: { list: () => [comp] }, EngineRender: engine, playing: false };
    const thumbs = createCompThumbnails(PM);
    thumbs.refresh('k1');
    await vi.advanceTimersByTimeAsync(400);
    expect(engine.thumbnails.request).toHaveBeenCalledWith(expect.objectContaining({ key: 'composition:c1', composition: 'c1', width: 320, height: 180, draft3d: true }));
    await vi.advanceTimersByTimeAsync(0);
    expect(thumbs.urls['c1']).toBe('data:image/jpeg;thumb');
    // A result that lands while playback runs waits to be encoded.
    let land: (value: unknown) => void = () => {};
    engine.thumbnails.request.mockImplementationOnce(() => new Promise((resolve) => { land = resolve; }) as never);
    thumbs.refresh('k2');
    await vi.advanceTimersByTimeAsync(400);
    expect(engine.thumbnails.request).toHaveBeenCalledTimes(2);
    toDataURL.mockClear();
    PM.playing = true;
    land(snapshot);
    await vi.advanceTimersByTimeAsync(2000);
    expect(toDataURL).not.toHaveBeenCalled();
    PM.playing = false;
    await vi.advanceTimersByTimeAsync(600);
    expect(toDataURL).toHaveBeenCalledTimes(1);
    thumbs.dispose();
  });
});
