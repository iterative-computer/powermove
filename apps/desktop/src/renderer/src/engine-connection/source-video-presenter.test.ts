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
  const sourceFrame = vi.fn((_token: string): VideoFrame | null => ({ close: vi.fn() }) as unknown as VideoFrame);
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
    constructor(readonly source: VideoFrame, readonly options: { timestamp: number }) {
      created.push({ source, timestamp: options.timestamp, close: this.close });
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
      video: { format, primaries: 'smpte170m', transfer: 'smpte170m', matrix: 'smpte170m', range: 'limited' } });
    expect(f.created[0]!.source).toBe(data);
    expect(f.created[0]!.timestamp).toBeGreaterThanOrEqual(0);
    expect(f.sourceFrame).not.toHaveBeenCalled();
    expect(f.discardSource).not.toHaveBeenCalled();
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    presenter.dispose();
  });

  it('starts playback before writing, closes both frames after acceptance, and stamps nonchronological seeks monotonically', async () => {
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
    expect(f.created.map(frame => frame.timestamp)).toEqual([1_000_000, 1_000_001]);
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
    expect(f.created[1]!.timestamp).toBeGreaterThan(f.created[0]!.timestamp);
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
    let count = 0;
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', { configurable: true,
      value: (callback: VideoFrameRequestCallback) => { presented = callback; return 1; } });
    onTestFinished(() => { delete (HTMLVideoElement.prototype as Partial<HTMLVideoElement>).requestVideoFrameCallback; });
    const presenter = new SourceVideoPresenter(f.bridge);
    const painting = presenter.paint({ token: 'surface', width: 4, height: 2 });
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.discardSource).not.toHaveBeenCalled();
    presented(0, { presentedFrames: ++count } as VideoFrameCallbackMetadata);
    await painting;
    expect(f.discardSource).toHaveBeenCalledWith('surface');
    presenter.dispose();
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
