// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SourceVideoPresenter, type SourceVideoDescriptor } from './source-video-presenter';

type Created = { source: unknown; timestamp: number; format?: string; close: ReturnType<typeof vi.fn> };

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
    constructor(readonly source: unknown, readonly options: VideoFrameBufferInit | { timestamp: number }) {
      created.push({ source, timestamp: options.timestamp, format: (options as VideoFrameBufferInit).format, close: this.close });
    }
  });
  // created[0] wraps the planes; later frames are timestamped copies written to the track.
  const planes = (): SourceVideoDescriptor => ({ width: 4, height: 2, data: new Uint8ClampedArray(12),
    video: { format: 'nv12', primaries: 'smpte170m', transfer: 'smpte170m', matrix: 'smpte170m', range: 'limited' } });
  // Frames written to the track wrap another VideoFrame, not plane bytes.
  const stamped = () => created.filter(frame => !(frame.source instanceof Uint8ClampedArray));
  return { created, stamped, tracks, write, abort, play, pause, planes };
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('SourceVideoPresenter', () => {
  it.each([
    ['nv12', 'NV12', 12], ['i420a', 'I420A', 20], ['i422', 'I422', 16], ['i420p10', 'I420P10', 24]
  ] as const)('wraps %s planes as a WebCodecs %s frame without an RGB conversion', async (format, webCodecs, bytes) => {
    const f = fixture();
    const presenter = new SourceVideoPresenter();
    const data = new Uint8ClampedArray(bytes);
    await presenter.paint({ width: 4, height: 2, data,
      video: { format, primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', range: 'limited' } });
    expect(f.created[0]!.source).toBe(data);
    expect(f.created[0]!.format).toBe(webCodecs);
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    expect(f.created[1]!.close).toHaveBeenCalledOnce();
    presenter.dispose();
  });

  it('rejects planes whose length does not match their layout', async () => {
    fixture();
    const presenter = new SourceVideoPresenter();
    await expect(presenter.paint({ width: 4, height: 2, data: new Uint8ClampedArray(12),
      video: { format: 'i420p10', primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', range: 'limited' } })).rejects.toThrow(/plane length/);
    presenter.dispose();
  });

  it('starts playback before writing, closes both frames after acceptance, and stamps nonchronological seeks monotonically', async () => {
    const f = fixture();
    let accept!: () => void;
    f.write.mockImplementationOnce(() => new Promise<void>(resolve => { accept = resolve; }));
    vi.spyOn(performance, 'now').mockReturnValue(1000);
    const presenter = new SourceVideoPresenter();
    const first = presenter.paint(f.planes());
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

    await presenter.paint(f.planes());
    expect(f.stamped().map(frame => frame.timestamp)).toEqual([1_000_000, 1_000_001]);
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
    const painting = presenter.paint(f.planes());
    await vi.advanceTimersByTimeAsync(0);
    expect(f.write).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).not.toHaveBeenCalled();
    expect(f.stamped()[0]!.close).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(16);
    expect(f.write).toHaveBeenCalledTimes(2);
    expect(f.stamped()[1]!.source).toBe(f.stamped()[0]!.source);
    expect(f.stamped()[1]!.timestamp).toBeGreaterThan(f.stamped()[0]!.timestamp);
    start();
    await painting;
    expect(f.stamped()[1]!.close).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    presenter.dispose();
  });

  it('bounds an unattachable video sink and tears down the failed stream', async () => {
    vi.useFakeTimers();
    const f = fixture();
    f.play.mockImplementationOnce(() => new Promise<void>(() => {}));
    const presenter = new SourceVideoPresenter();
    const rejected = expect(presenter.paint(f.planes()))
      .rejects.toThrow(/did not start/);
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
    const rejected = expect(presenter.paint(f.planes(), controller.signal))
      .rejects.toMatchObject({ name: 'AbortError' });
    await vi.advanceTimersByTimeAsync(0);
    controller.abort();
    await rejected;
    expect(f.abort).not.toHaveBeenCalled();
    expect(f.tracks[0]!.stop).not.toHaveBeenCalled();
    expect(presenter.surface.srcObject).not.toBeNull();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    start();
    await presenter.paint(f.planes());
    expect(f.play).toHaveBeenCalledOnce();
    expect(f.tracks).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(0);
    presenter.dispose();
  });

  it('propagates playback errors, releases frames, and stops the failed track', async () => {
    const f = fixture();
    f.play.mockRejectedValueOnce(new Error('play failed'));
    // A video track may hold its first write until playback starts.
    f.write.mockImplementationOnce(() => new Promise<void>(() => {}));
    const presenter = new SourceVideoPresenter();
    await expect(presenter.paint(f.planes())).rejects.toThrow('play failed');
    expect(f.abort).toHaveBeenCalledOnce();
    expect(f.tracks[0]!.stop).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    expect(f.created[0]!.close).toHaveBeenCalledOnce();
    presenter.dispose();
  });

  it('stops and detaches the track on dispose and rejects later paint', async () => {
    const f = fixture();
    const presenter = new SourceVideoPresenter();
    document.body.append(presenter.surface);
    await presenter.paint(f.planes());
    expect(presenter.surface.srcObject).not.toBeNull();
    f.write.mockImplementationOnce(() => new Promise<void>(() => {}));
    const pending = expect(presenter.paint(f.planes()))
      .rejects.toMatchObject({ name: 'AbortError' });
    presenter.dispose();
    await pending;
    presenter.dispose();
    expect(f.abort).toHaveBeenCalledOnce();
    expect(f.tracks[0]!.stop).toHaveBeenCalledOnce();
    expect(f.pause).toHaveBeenCalledOnce();
    expect(presenter.surface.srcObject).toBeNull();
    expect(presenter.surface.isConnected).toBe(false);
    await expect(presenter.paint(f.planes())).rejects.toThrow(/disposed/);
  });
});
