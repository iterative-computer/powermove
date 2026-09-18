// @vitest-environment happy-dom
import { expect, it, vi } from 'vitest';
import { prepareFrame } from './frame-preparation';

it('ignores a queued seek completion until the requested frame has decoded', async () => {
  const video = new EventTarget() as any;
  Object.assign(video, { paused: true, seeking: true, readyState: 2, currentTime: 0, pause() {} });
  const draw = vi.fn();
  const context = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage: draw } as any);
  const layer = { id: 'clip', type: 'video', from: 0, d: { asset: 'source', speed: 1, trim: 0 } };
  const asset = { el: video, w: 16, h: 16, dur: 2 };
  const PM: any = { proj: { layers: [layer], fps: 30 }, active: () => true,
    scope: { push() {}, pop() {} }, assets: { get: () => asset } };
  try {
    const pending = prepareFrame(PM, 1.5);
    video.dispatchEvent(new Event('seeked'));
    await Promise.resolve();
    expect(draw).not.toHaveBeenCalled();
    video.seeking = false; video.currentTime = 0;
    video.dispatchEvent(new Event('seeked'));
    await Promise.resolve();
    expect(draw).not.toHaveBeenCalled();
    video.currentTime = 1.5;
    video.dispatchEvent(new Event('seeked'));
    await pending;
    expect(draw).toHaveBeenCalledOnce();
    expect(PM.preparedVideoFrames.has('clip@1.5')).toBe(true);
  } finally { context.mockRestore(); }
});
