// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { captureAgentFrame } from './agent-frame-capture';
import { prepareFrame } from './frame-preparation';
import { VideoFrameDecodeError, withCaptureSignal } from './video-frame-decode';

vi.mock('./frame-preparation', () => ({ prepareFrame: vi.fn() }));
const prepare = vi.mocked(prepareFrame);
function fixture() {
  const listeners = new Map<string, Set<() => void>>();
  const PM: any = {
    proj: { id: 'a', compId: 'main', revision: 0, w: 1920, h: 1080 }, playing: true,
    pause: vi.fn(), play: vi.fn(), invalidate: vi.fn(),
    bus: {
      on(event: string, listener: () => void) {
        const set = listeners.get(event) ?? new Set(); listeners.set(event, set); set.add(listener);
        return () => set.delete(listener);
      },
      emit(event: string) { listeners.get(event)?.forEach(listener => listener()); }
    }
  };
  return { PM, exporter: { busy: false, snapshot: vi.fn(() => 'jpeg') }, listeners };
}
beforeEach(() => {
  prepare.mockReset();
  vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => {});
  vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {});
});
afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers(); });

it('retries a stalled decoder once on a fresh resource and releases every owned resource', async () => {
  const { PM, exporter, listeners } = fixture();
  const source = document.createElement('video'); source.src = 'blob:test';
  const decoders: HTMLVideoElement[] = [], canvases: HTMLCanvasElement[] = [];
  prepare.mockImplementation(async (pm, _time, _project, options) => {
    const v = options!.videoForCapture!({ el: source }); decoders.push(v);
    expect(v).not.toBe(source);
    if (decoders.length === 1) throw new VideoFrameDecodeError();
    const c = document.createElement('canvas'); canvases.push(c); options!.onVideoFrame!(c);
    pm.preparedVideoFrames = new Map([['clip@1', c]]);
  });
  await expect(captureAgentFrame(PM, exporter, 1)).resolves.toBe('jpeg');
  expect(decoders).toHaveLength(2); expect(decoders[0]).not.toBe(decoders[1]);
  expect(decoders.every(v => !v.hasAttribute('src'))).toBe(true);
  expect(source.getAttribute('src')).toBe('blob:test');
  expect(canvases[0]!.width).toBe(0);
  expect(PM.agentFrameCapture).toBe(false); expect(PM.preparedVideoFrames).toBeNull();
  expect(PM.play).toHaveBeenCalledOnce();
  expect([...listeners.values()].every(set => set.size === 0)).toBe(true);
});

it('cancels a project switch promptly without rendering, resuming, or clearing the new project', async () => {
  const { PM, exporter } = fixture();
  prepare.mockImplementation((_pm, _time, _project, options) => withCaptureSignal(new Promise(() => {}), options?.signal));
  const pending = captureAgentFrame(PM, exporter, 1);
  const cancelled = expect(pending).rejects.toHaveProperty('name', 'AbortError');
  await vi.waitFor(() => expect(prepare).toHaveBeenCalledOnce());
  const newFrames = new Map(); PM.proj = { id: 'b' }; PM.preparedVideoFrames = newFrames;
  PM.bus.emit('project'); await cancelled;
  await vi.waitFor(() => expect(PM.agentFrameCapture).toBe(false));
  expect(exporter.snapshot).not.toHaveBeenCalled(); expect(PM.play).not.toHaveBeenCalled();
  expect(PM.preparedVideoFrames).toBe(newFrames);
});

it('queues concurrent reviews, and a failed request does not poison later captures', async () => {
  const { PM, exporter } = fixture();
  prepare.mockRejectedValueOnce(new Error('Invalid frame')).mockResolvedValue(undefined);
  const one = captureAgentFrame(PM, exporter, 1), two = captureAgentFrame(PM, exporter, 2);
  await expect(one).rejects.toThrow('Invalid frame');
  await expect(two).resolves.toBe('jpeg');
  expect(exporter.snapshot).toHaveBeenCalledExactlyOnceWith(2, 480);
});

it('reports repeated decoder failure after exactly one recovery attempt', async () => {
  const { PM, exporter } = fixture();
  prepare.mockRejectedValue(new VideoFrameDecodeError('Could not decode video frame'));
  await expect(captureAgentFrame(PM, exporter, 1)).rejects.toThrow('Could not decode video frame');
  expect(prepare).toHaveBeenCalledTimes(2);
  expect(PM.agentFrameCapture).toBe(false);
  expect(exporter.snapshot).not.toHaveBeenCalled();
});

it('bounds recovery by the original deadline and permits a later successful request', async () => {
  vi.useFakeTimers(); const { PM, exporter, listeners } = fixture();
  prepare.mockImplementation((_pm, _time, _project, options) => withCaptureSignal(new Promise(() => {}), options?.signal));
  const pending = captureAgentFrame(PM, exporter, 1), failed = expect(pending).rejects.toThrow('Video frame decode timed out');
  await vi.advanceTimersByTimeAsync(10_000); await failed;
  await vi.runAllTimersAsync();
  expect(PM.agentFrameCapture).toBe(false);
  expect(vi.getTimerCount()).toBe(0);
  expect([...listeners.values()].every(set => set.size === 0)).toBe(true);
  prepare.mockResolvedValue(undefined);
  await expect(captureAgentFrame(PM, exporter, 2)).resolves.toBe('jpeg');
});

it('cancels queued caller requests without touching a decoder or rendering them', async () => {
  const { PM, exporter } = fixture(); const controller = new AbortController();
  let release!: () => void;
  prepare.mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve; }));
  const first = captureAgentFrame(PM, exporter, 1);
  await vi.waitFor(() => expect(prepare).toHaveBeenCalledOnce());
  const second = captureAgentFrame(PM, exporter, 2, 480, { signal: controller.signal });
  const cancelled = expect(second).rejects.toHaveProperty('name', 'AbortError'); controller.abort(); await cancelled;
  release(); await first;
  expect(prepare).toHaveBeenCalledOnce(); expect(exporter.snapshot).toHaveBeenCalledOnce();
});
