import { afterEach, describe, expect, it, vi } from 'vitest';

import { capturePoster } from './poster';

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function posterCanvas() {
  const drawImage = vi.fn();
  const canvas: any = {
    width: 0,
    height: 0,
    getContext: () => ({ drawImage }),
    toBlob(callback: (blob: Blob | null) => void, type: string) {
      callback(new Blob(['poster'], { type }));
    },
  };
  vi.stubGlobal('window', { document: { createElement: () => canvas } });
  return { canvas, drawImage };
}

describe('media poster capture', () => {
  it('scales an image to the requested maximum edge while preserving aspect', async () => {
    const { canvas, drawImage } = posterCanvas();
    const image = { width: 1920, height: 1080 };

    const blob = await capturePoster({ kind: 'image', el: image, w: 1920, h: 1080 });

    expect(blob?.type).toBe('image/webp');
    expect(canvas).toMatchObject({ width: 320, height: 180 });
    expect(drawImage).toHaveBeenCalledWith(image, 0, 0, 320, 180);
  });

  it('seeks a video, draws its frame, and restores its previous time', async () => {
    const { drawImage } = posterCanvas();
    let seeked: (() => void) | null = null;
    const assigned: number[] = [];
    let time = 4;
    const video: any = {
      videoWidth: 1280,
      videoHeight: 720,
      addEventListener(name: string, listener: () => void) { if (name === 'seeked') seeked = listener; },
      removeEventListener() { seeked = null; },
      get currentTime() { return time; },
      set currentTime(value: number) {
        time = value;
        assigned.push(value);
        if (assigned.length === 1) queueMicrotask(() => seeked?.());
      },
    };

    const blob = await capturePoster({ kind: 'video', el: video, dur: 20 });

    expect(blob).toBeInstanceOf(Blob);
    expect(assigned).toEqual([.5, 4]);
    expect(video.currentTime).toBe(4);
    expect(drawImage).toHaveBeenCalledWith(video, 0, 0, 320, 180);
  });

  it('returns null for zero-sized video and a seek timeout', async () => {
    const { drawImage } = posterCanvas();
    const zero = { videoWidth: 0, videoHeight: 720, currentTime: 0 };
    await expect(capturePoster({ kind: 'video', el: zero, dur: 10 })).resolves.toBeNull();

    vi.useFakeTimers();
    const stalled: any = {
      videoWidth: 640, videoHeight: 360, currentTime: 2,
      addEventListener() {}, removeEventListener() {},
    };
    const pending = capturePoster({ kind: 'video', el: stalled, dur: 10 });
    await vi.advanceTimersByTimeAsync(2000);

    await expect(pending).resolves.toBeNull();
    expect(stalled.currentTime).toBe(2);
    expect(drawImage).not.toHaveBeenCalled();
  });

  it('never throws when canvas creation or encoding is unavailable', async () => {
    vi.stubGlobal('window', { document: { createElement: () => { throw new Error('no canvas'); } } });
    await expect(capturePoster({ kind: 'image', el: { width: 10, height: 10 } })).resolves.toBeNull();

    vi.stubGlobal('window', { document: { createElement: () => ({
      width: 0, height: 0, getContext: () => ({ drawImage() {} }),
    }) } });
    await expect(capturePoster({ kind: 'image', el: { width: 10, height: 10 } })).resolves.toBeNull();
  });
});
