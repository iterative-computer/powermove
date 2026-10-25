// @vitest-environment happy-dom
import { afterEach, describe, expect, it, onTestFinished, vi } from 'vitest';
import type { FrameBridgeApi } from '../../../shared/frame-bridge';
import { SourceVideoPresenter } from './source-video-presenter';

type Bridge = Pick<FrameBridgeApi, 'sourceFrame' | 'discardSource'>;

function fixture() {
  const created: Array<{ source: VideoFrame; timestamp: number; close: ReturnType<typeof vi.fn> }> = [];
  const tracks: Array<{ stop: ReturnType<typeof vi.fn> }> = [];
  const write = vi.fn(async (_frame: VideoFrame) => {});
  const abort = vi.fn(async (_reason: unknown) => {});
  let nextTimestamp = 1_000_000;
  const sourceFrame = vi.fn((_token: string): VideoFrame | null => ({ close: vi.fn(), timestamp: ++nextTimestamp }) as unknown as VideoFrame);
  const discardSource = vi.fn((_token: string) => {});
  const bridge: Bridge = { sourceFrame, discardSource };
  const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue();
  const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {});
  vi.stubGlobal('MediaStreamTrackGenerator', class {
    stop = vi.fn();
    writable = { getWriter: () => ({ write, abort }) };
    constructor(options: { kind: string }) {
      expect(options).toEqual({ kind: 'video' });
      tracks.push(this);
    }
  });
  vi.stubGlobal('VideoFrame', class {
    close = vi.fn();
    readonly timestamp: number;
    constructor(readonly source: VideoFrame, readonly options?: { timestamp: number }) {
      this.timestamp = options?.timestamp ?? source.timestamp;
      created.push({ source, timestamp: this.timestamp, close: this.close });
    }
  });
  return { bridge, sourceFrame, discardSource, created, tracks, write, abort, play, pause };
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('SourceVideoPresenter', () => {
  it.each(['nv12', 'i420a'])('preserves native %s planes without an RGB conversion or shared-texture lookup', async format => {
    const f = fixture();
    const presenter = new SourceVideoPresenter(undefined);
    const data = new Uint8ClampedArray(format === 'i420a' ? 20 : 12);
    await presenter.paint({ width: 4, height: 2, data,
      video: { format, timestamp: 1000, primaries: 'smpte170m', transfer: 'smpte170m', matrix: 'smpte170m', range: 'limited' } });
    expect(f.created[0]!.source).toBe(data);
    expect(f.created[0]!.timestamp).toBeGreaterThanOrEqual(0);
    expect(f.sourceFrame).not.toHaveBeenCalled();
    expect(f.discardSource).not.toHaveBeenCalled();
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    presenter.dispose();
  });

  it('starts playback before writing, closes both frames, and preserves source delivery stamps', async () => {
    const f = fixture();
    let accept!: () => void;
    f.write.mockImplementationOnce(() => new Promise<void>(resolve => { accept = resolve; }));
    vi.spyOn(performance, 'now').mockReturnValue(1000);
    const presenter = new SourceVideoPresenter(f.bridge);
    const first = presenter.paint({ token: 'first', width: 4, height: 2 });
    expect(presenter.surface.dataset.nativeSource).toBe('true');
    expect(presenter.surface.muted).toBe(true);
    expect(presenter.surface.playsInline).toBe(true);
    expect(presenter.surface.autoplay).toBe(true);
    expect(f.play).toHaveBeenCalledOnce();
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.play.mock.invocationCallOrder[0]).toBeLessThan(f.write.mock.invocationCallOrder[0]!);
    expect([presenter.surface.width, presenter.surface.height]).toEqual([4, 2]);
    expect(f.created[0]!.close).not.toHaveBeenCalled();
    expect(f.sourceFrame.mock.results[0]!.value.close).not.toHaveBeenCalled();
    accept();
    await first;
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    expect(f.sourceFrame.mock.results[0]!.value.close).toHaveBeenCalledOnce();
    expect(f.discardSource).toHaveBeenCalledWith('first');

    await presenter.paint({ token: 'earlier-seek', width: 4, height: 2 });
    expect(f.created.map(frame => frame.timestamp)).toEqual([1_000_001, 1_000_002]);
    expect(f.play).toHaveBeenCalledOnce();
    expect(f.tracks).toHaveLength(1);
    expect(f.discardSource).toHaveBeenCalledTimes(2);
    presenter.dispose();
  });

  it('replays the held first frame until an asynchronously attached video sink starts', async () => {
    vi.useFakeTimers();
    const f = fixture();
    let start!: () => void;
    f.play.mockImplementationOnce(() => new Promise<void>(resolve => { start = resolve; }));
    const presenter = new SourceVideoPresenter(f.bridge);
    const painting = presenter.paint({ token: 'startup', width: 4, height: 2 });
    await vi.advanceTimersByTimeAsync(0);
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.sourceFrame.mock.results[0]!.value.close).not.toHaveBeenCalled();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(16);
    expect(f.write).toHaveBeenCalledTimes(2);
    expect(f.created[1]!.source).toBe(f.sourceFrame.mock.results[0]!.value);
    expect(f.created[1]!.timestamp).toBe(f.created[0]!.timestamp);
    start();
    await painting;
    expect(f.created[1]!.close).toHaveBeenCalledOnce();
    expect(f.sourceFrame.mock.results[0]!.value.close).toHaveBeenCalledOnce();
    expect(f.discardSource).toHaveBeenCalledWith('startup');
    expect(vi.getTimerCount()).toBe(0);
    presenter.dispose();
  });

  it('keeps a shared-texture import until the element presents its frame', async () => {
    const f = fixture();
    let presented!: VideoFrameRequestCallback;
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', { configurable: true,
      value: (callback: VideoFrameRequestCallback) => { presented = callback; return 1; } });
    onTestFinished(() => { delete (HTMLVideoElement.prototype as Partial<HTMLVideoElement>).requestVideoFrameCallback; });
    const presenter = new SourceVideoPresenter(f.bridge);
    document.body.append(presenter.surface);
    await presenter.paint({ token: 'surface', width: 4, height: 2 });
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.discardSource).not.toHaveBeenCalled();
    presented(0, { presentedFrames: 1, mediaTime: f.created.at(-1)!.timestamp / 1e6 } as VideoFrameCallbackMetadata);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(f.discardSource).toHaveBeenCalledWith('surface');
    presenter.dispose();
  });

  it('writes a frame only after the element presented the previous one, and bounds a silent element', async () => {
    vi.useFakeTimers();
    const f = fixture();
    let presented!: VideoFrameRequestCallback;
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', { configurable: true,
      value: (callback: VideoFrameRequestCallback) => { presented = callback; return 1; } });
    onTestFinished(() => { delete (HTMLVideoElement.prototype as Partial<HTMLVideoElement>).requestVideoFrameCallback; });
    const presenter = new SourceVideoPresenter(undefined);
    document.body.append(presenter.surface);
    const planes = (fill: number) => ({ width: 4, height: 2, data: new Uint8ClampedArray(12).fill(fill),
      video: { format: 'nv12', timestamp: fill * 1000, primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', range: 'limited' } as const });
    await presenter.paint(planes(1));
    const written = f.created.at(-1)!.timestamp;
    let second = false;
    const painting = presenter.paint(planes(2)).then(() => { second = true; });
    await vi.advanceTimersByTimeAsync(0);
    // An earlier frame's presentation does not release the next write.
    presented(0, { presentedFrames: 1, mediaTime: (written - 40_000) / 1e6 } as VideoFrameCallbackMetadata);
    await vi.advanceTimersByTimeAsync(0);
    expect(f.write).toHaveBeenCalledOnce();
    expect(second).toBe(false);
    presented(0, { presentedFrames: 2, mediaTime: written / 1e6 } as VideoFrameCallbackMetadata);
    await painting;
    expect(f.write).toHaveBeenCalledTimes(2);
    const latest = f.created.at(-1)!.timestamp;
    expect(latest).toBeGreaterThan(written);
    // Never presented (the element is not rendered): the third write waits the bound, no longer.
    const third = presenter.paint(planes(3));
    await vi.advanceTimersByTimeAsync(249);
    expect(f.write).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);
    await third;
    expect(f.write).toHaveBeenCalledTimes(3);
    presenter.dispose();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('matches adjacent microseconds across raw/shared frames and rejects old callbacks', async () => {
    vi.useFakeTimers();
    const f = fixture();
    vi.spyOn(performance, 'now').mockReturnValue(1000);
    let presented!: VideoFrameRequestCallback;
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', { configurable: true,
      value: (callback: VideoFrameRequestCallback) => { presented = callback; return 1; } });
    onTestFinished(() => { delete (HTMLVideoElement.prototype as Partial<HTMLVideoElement>).requestVideoFrameCallback; });
    const presenter = new SourceVideoPresenter(f.bridge);
    document.body.append(presenter.surface);
    const planes = (timestamp: number) => ({ width: 4, height: 2, data: new Uint8ClampedArray(12),
      video: { timestamp, format: 'nv12', primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', range: 'limited' } as const });
    await presenter.paint(planes(1_000_000));
    const first = f.created.at(-1)!.timestamp;
    const second = presenter.paint({ token: 'second', width: 4, height: 2 });
    presented(0, { mediaTime: first / 1e6 } as VideoFrameCallbackMetadata);
    await second;
    expect(f.write).toHaveBeenCalledTimes(2);
    const latest = f.created.at(-1)!.timestamp;
    expect(latest - first).toBe(1);
    const third = presenter.paint(planes(1_000_002));
    // A repeated callback for the old frame must still wait for the new one.
    presented(0, { mediaTime: first / 1e6 } as VideoFrameCallbackMetadata);
    await vi.advanceTimersByTimeAsync(0);
    expect(f.write).toHaveBeenCalledTimes(2);
    presented(0, { mediaTime: latest / 1e6 } as VideoFrameCallbackMetadata);
    await third;
    expect(f.write).toHaveBeenCalledTimes(3);
    expect(f.discardSource).toHaveBeenCalledWith('second');
    presenter.dispose();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('bounds an unattachable video sink and tears down the failed stream', async () => {
    vi.useFakeTimers();
    const f = fixture();
    f.play.mockImplementationOnce(() => new Promise<void>(() => {}));
    const presenter = new SourceVideoPresenter(f.bridge);
    const rejected = expect(presenter.paint({ token: 'unattachable', width: 4, height: 2 }))
      .rejects.toThrow(/did not start/);
    await vi.advanceTimersByTimeAsync(5_000);
    await rejected;
    expect(f.write.mock.calls.length).toBeGreaterThan(1);
    expect(f.abort).toHaveBeenCalledOnce();
    expect(f.tracks[0]!.stop).toHaveBeenCalledOnce();
    expect(f.sourceFrame.mock.results[0]!.value.close).toHaveBeenCalledOnce();
    expect(f.discardSource).toHaveBeenCalledWith('unattachable');
    expect(vi.getTimerCount()).toBe(0);
    presenter.dispose();
  });

  it('cancels one paint without tearing down the starting video stream', async () => {
    vi.useFakeTimers();
    const f = fixture();
    let start!: () => void;
    f.play.mockImplementationOnce(() => new Promise<void>(resolve => { start = resolve; }));
    const presenter = new SourceVideoPresenter(f.bridge);
    const controller = new AbortController();
    const rejected = expect(presenter.paint({ token: 'aborted', width: 4, height: 2 }, controller.signal))
      .rejects.toMatchObject({ name: 'AbortError' });
    await vi.advanceTimersByTimeAsync(0);
    controller.abort();
    await rejected;
    expect(f.abort).not.toHaveBeenCalled();
    expect(f.tracks[0]!.stop).not.toHaveBeenCalled();
    expect(presenter.surface.srcObject).not.toBeNull();
    expect(f.sourceFrame.mock.results[0]!.value.close).toHaveBeenCalledOnce();
    expect(f.discardSource).toHaveBeenCalledWith('aborted');
    start();
    await presenter.paint({ token: 'latest', width: 4, height: 2 });
    expect(f.play).toHaveBeenCalledOnce();
    expect(f.tracks).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(0);
    presenter.dispose();
  });

  it('throws on a missing bridge or missing native surface without using a browser decoder', async () => {
    const f = fixture();
    const unavailable = new SourceVideoPresenter(undefined);
    await expect(unavailable.paint({ token: 'missing', width: 4, height: 2 })).rejects.toThrow(/bridge is unavailable/);
    unavailable.dispose();
    f.sourceFrame.mockReturnValueOnce(null);
    const presenter = new SourceVideoPresenter(f.bridge);
    await expect(presenter.paint({ token: 'absent', width: 4, height: 2 })).rejects.toThrow(/was not delivered/);
    expect(f.discardSource).toHaveBeenCalledWith('absent');
    expect(f.tracks).toHaveLength(0);
    expect(presenter.surface.srcObject).toBeNull();
    expect(presenter.surface.hasAttribute('src')).toBe(false);
    presenter.dispose();
  });

  it('propagates playback errors, releases frames, and stops the failed track', async () => {
    const f = fixture();
    f.play.mockRejectedValueOnce(new Error('play failed'));
    // A video track may hold its first write until playback starts.
    f.write.mockImplementationOnce(() => new Promise<void>(() => {}));
    const presenter = new SourceVideoPresenter(f.bridge);
    await expect(presenter.paint({ token: 'play-failure', width: 4, height: 2 })).rejects.toThrow('play failed');
    expect(f.abort).toHaveBeenCalledOnce();
    expect(f.tracks[0]!.stop).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    expect(f.sourceFrame.mock.results[0]!.value.close).toHaveBeenCalledOnce();
    expect(f.discardSource).toHaveBeenCalledWith('play-failure');
    presenter.dispose();
  });

  it('stops and detaches the track on dispose and rejects later paint', async () => {
    const f = fixture();
    const presenter = new SourceVideoPresenter(f.bridge);
    document.body.append(presenter.surface);
    await presenter.paint({ token: 'shown', width: 4, height: 2 });
    expect(presenter.surface.srcObject).not.toBeNull();
    f.write.mockImplementationOnce(() => new Promise<void>(() => {}));
    const pending = expect(presenter.paint({ token: 'pending', width: 4, height: 2 }))
      .rejects.toMatchObject({ name: 'AbortError' });
    presenter.dispose();
    await pending;
    presenter.dispose();
    expect(f.abort).toHaveBeenCalledOnce();
    expect(f.tracks[0]!.stop).toHaveBeenCalledOnce();
    expect(f.pause).toHaveBeenCalledOnce();
    expect(presenter.surface.srcObject).toBeNull();
    expect(presenter.surface.isConnected).toBe(false);
    expect(f.discardSource).toHaveBeenCalledWith('pending');
    await expect(presenter.paint({ token: 'late', width: 4, height: 2 })).rejects.toThrow(/disposed/);
    expect(f.sourceFrame).toHaveBeenCalledTimes(2);
  });
});
