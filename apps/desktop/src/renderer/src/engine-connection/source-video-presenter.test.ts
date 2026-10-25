// @vitest-environment happy-dom
import { afterEach, describe, expect, it, onTestFinished, vi } from 'vitest';
import { SourceVideoPresenter, type SourceVideoDescriptor } from './source-video-presenter';

type Created = { frame: VideoFrame; source: unknown; timestamp: number; close: ReturnType<typeof vi.fn> };

function fixture() {
  const created: Created[] = [];
  const tracks: Array<{ stop: ReturnType<typeof vi.fn> }> = [];
  const write = vi.fn(async (_frame: VideoFrame) => {});
  const abort = vi.fn(async (_reason: unknown) => {});
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
    constructor(readonly source: { timestamp?: number }, readonly options?: { timestamp: number; format?: string }) {
      this.timestamp = options?.timestamp ?? source.timestamp!;
      created.push({ frame: this as unknown as VideoFrame, source, timestamp: this.timestamp, close: this.close });
    }
  });
  return { created, tracks, write, abort, play, pause };
}

const BYTES: Record<string, number> = { nv12: 12, i420a: 20, i422: 16, i420p10: 24 };
const planes = (timestamp: number, format = 'nv12'): SourceVideoDescriptor => ({ width: 4, height: 2,
  data: new Uint8ClampedArray(BYTES[format]!),
  video: { format, timestamp, primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', range: 'limited' } });

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('SourceVideoPresenter', () => {
  it.each([['nv12', 'NV12'], ['i420a', 'I420A'], ['i422', 'I422'], ['i420p10', 'I420P10']])('presents native %s planes as a WebCodecs %s buffer frame', async (format, webcodecs) => {
    const f = fixture();
    const presenter = new SourceVideoPresenter();
    const descriptor = planes(1000, format);
    await presenter.paint(descriptor);
    expect(f.created[0]!.source).toBe(descriptor.data);
    expect((f.created[0]!.frame as unknown as { options: { format: string } }).options.format).toBe(webcodecs);
    expect(f.created[0]!.timestamp).toBe(1000);
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.created.every(frame => frame.close.mock.calls.length === 1)).toBe(true);
    presenter.dispose();
  });

  it('starts playback before writing, closes both frames, and preserves source delivery stamps', async () => {
    const f = fixture();
    let accept!: () => void;
    f.write.mockImplementationOnce(() => new Promise<void>(resolve => { accept = resolve; }));
    const presenter = new SourceVideoPresenter();
    const first = presenter.paint(planes(1_000_001));
    expect(presenter.surface.dataset.nativeSource).toBe('true');
    expect(presenter.surface.muted).toBe(true);
    expect(presenter.surface.playsInline).toBe(true);
    expect(presenter.surface.autoplay).toBe(true);
    expect(f.play).toHaveBeenCalledOnce();
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.play.mock.invocationCallOrder[0]).toBeLessThan(f.write.mock.invocationCallOrder[0]!);
    expect([presenter.surface.width, presenter.surface.height]).toEqual([4, 2]);
    expect(f.created[0]!.close).not.toHaveBeenCalled();
    expect(f.created[1]!.close).not.toHaveBeenCalled();
    accept();
    await first;
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    expect(f.created[1]!.close).toHaveBeenCalledOnce();

    await presenter.paint(planes(1_000_002));
    expect(f.created.map(frame => frame.timestamp)).toEqual([1_000_001, 1_000_001, 1_000_002, 1_000_002]);
    expect(f.play).toHaveBeenCalledOnce();
    expect(f.tracks).toHaveLength(1);
    presenter.dispose();
  });

  it('replays the held first frame until an asynchronously attached video sink starts', async () => {
    vi.useFakeTimers();
    const f = fixture();
    let start!: () => void;
    f.play.mockImplementationOnce(() => new Promise<void>(resolve => { start = resolve; }));
    const presenter = new SourceVideoPresenter();
    const painting = presenter.paint(planes(5));
    await vi.advanceTimersByTimeAsync(0);
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).not.toHaveBeenCalled();
    expect(f.created[1]!.close).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(16);
    expect(f.write).toHaveBeenCalledTimes(2);
    expect(f.created[2]!.source).toBe(f.created[0]!.frame);
    expect(f.created[2]!.timestamp).toBe(f.created[1]!.timestamp);
    start();
    await painting;
    expect(f.created[2]!.close).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    presenter.dispose();
  });

  it('writes a frame only after the element presented the previous one, and bounds a silent element', async () => {
    vi.useFakeTimers();
    const f = fixture();
    let presented!: VideoFrameRequestCallback;
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', { configurable: true,
      value: (callback: VideoFrameRequestCallback) => { presented = callback; return 1; } });
    onTestFinished(() => { delete (HTMLVideoElement.prototype as Partial<HTMLVideoElement>).requestVideoFrameCallback; });
    const presenter = new SourceVideoPresenter();
    document.body.append(presenter.surface);
    await presenter.paint(planes(1000));
    const written = f.created.at(-1)!.timestamp;
    let second = false;
    const painting = presenter.paint(planes(2000)).then(() => { second = true; });
    await vi.advanceTimersByTimeAsync(0);
    // An earlier frame's presentation does not release the next write.
    presented(0, { presentedFrames: 1, mediaTime: (written - 40_000) / 1e6 } as VideoFrameCallbackMetadata);
    await vi.advanceTimersByTimeAsync(0);
    expect(f.write).toHaveBeenCalledOnce();
    expect(second).toBe(false);
    presented(0, { presentedFrames: 2, mediaTime: written / 1e6 } as VideoFrameCallbackMetadata);
    await painting;
    expect(f.write).toHaveBeenCalledTimes(2);
    expect(f.created.at(-1)!.timestamp).toBeGreaterThan(written);
    // Never presented (the element is not rendered): the third write waits the bound, no longer.
    const third = presenter.paint(planes(3000));
    await vi.advanceTimersByTimeAsync(249);
    expect(f.write).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);
    await third;
    expect(f.write).toHaveBeenCalledTimes(3);
    presenter.dispose();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('matches adjacent microsecond stamps exactly and rejects old callbacks', async () => {
    vi.useFakeTimers();
    const f = fixture();
    let presented!: VideoFrameRequestCallback;
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', { configurable: true,
      value: (callback: VideoFrameRequestCallback) => { presented = callback; return 1; } });
    onTestFinished(() => { delete (HTMLVideoElement.prototype as Partial<HTMLVideoElement>).requestVideoFrameCallback; });
    const presenter = new SourceVideoPresenter();
    document.body.append(presenter.surface);
    await presenter.paint(planes(1_000_000));
    const second = presenter.paint(planes(1_000_001));
    presented(0, { mediaTime: 1 } as VideoFrameCallbackMetadata);
    await second;
    expect(f.write).toHaveBeenCalledTimes(2);
    const third = presenter.paint(planes(1_000_002));
    // A repeated callback for the old frame must still wait for the new one.
    presented(0, { mediaTime: 1 } as VideoFrameCallbackMetadata);
    await vi.advanceTimersByTimeAsync(0);
    expect(f.write).toHaveBeenCalledTimes(2);
    presented(0, { mediaTime: 1.000001 } as VideoFrameCallbackMetadata);
    await third;
    expect(f.write).toHaveBeenCalledTimes(3);
    presenter.dispose();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('bounds an unattachable video sink and tears down the failed stream', async () => {
    vi.useFakeTimers();
    const f = fixture();
    f.play.mockImplementationOnce(() => new Promise<void>(() => {}));
    const presenter = new SourceVideoPresenter();
    const rejected = expect(presenter.paint(planes(7))).rejects.toThrow(/did not start/);
    await vi.advanceTimersByTimeAsync(5_000);
    await rejected;
    expect(f.write.mock.calls.length).toBeGreaterThan(1);
    expect(f.abort).toHaveBeenCalledOnce();
    expect(f.tracks[0]!.stop).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    presenter.dispose();
  });

  it('cancels one paint without tearing down the starting video stream', async () => {
    vi.useFakeTimers();
    const f = fixture();
    let start!: () => void;
    f.play.mockImplementationOnce(() => new Promise<void>(resolve => { start = resolve; }));
    const presenter = new SourceVideoPresenter();
    const controller = new AbortController();
    const rejected = expect(presenter.paint(planes(8), controller.signal)).rejects.toMatchObject({ name: 'AbortError' });
    await vi.advanceTimersByTimeAsync(0);
    controller.abort();
    await rejected;
    expect(f.abort).not.toHaveBeenCalled();
    expect(f.tracks[0]!.stop).not.toHaveBeenCalled();
    expect(presenter.surface.srcObject).not.toBeNull();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    start();
    await presenter.paint(planes(9));
    expect(f.play).toHaveBeenCalledOnce();
    expect(f.tracks).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(0);
    presenter.dispose();
  });

  it('rejects malformed planes, formats and stamps without a browser decoder', async () => {
    const f = fixture();
    const presenter = new SourceVideoPresenter();
    await expect(presenter.paint({ ...planes(1), data: new Uint8ClampedArray(11) })).rejects.toThrow(/plane length/);
    await expect(presenter.paint(planes(1, 'p010le'))).rejects.toThrow(/frame format/);
    await expect(presenter.paint(planes(-1))).rejects.toThrow(/delivery timestamp/);
    expect(f.created).toHaveLength(0);
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
    const presenter = new SourceVideoPresenter();
    await expect(presenter.paint(planes(10))).rejects.toThrow('play failed');
    expect(f.abort).toHaveBeenCalledOnce();
    expect(f.tracks[0]!.stop).toHaveBeenCalledOnce();
    expect(f.created.every(frame => frame.close.mock.calls.length === 1)).toBe(true);
    presenter.dispose();
  });

  it('stops and detaches the track on dispose and rejects later paint', async () => {
    const f = fixture();
    const presenter = new SourceVideoPresenter();
    document.body.append(presenter.surface);
    await presenter.paint(planes(11));
    expect(presenter.surface.srcObject).not.toBeNull();
    f.write.mockImplementationOnce(() => new Promise<void>(() => {}));
    const pending = expect(presenter.paint(planes(12))).rejects.toMatchObject({ name: 'AbortError' });
    presenter.dispose();
    await pending;
    presenter.dispose();
    expect(f.abort).toHaveBeenCalledOnce();
    expect(f.tracks[0]!.stop).toHaveBeenCalledOnce();
    expect(f.pause).toHaveBeenCalledOnce();
    expect(presenter.surface.srcObject).toBeNull();
    expect(presenter.surface.isConnected).toBe(false);
    await expect(presenter.paint(planes(13))).rejects.toThrow(/disposed/);
    expect(f.created.every(frame => frame.close.mock.calls.length === 1)).toBe(true);
  });
});
