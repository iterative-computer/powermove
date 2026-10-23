import { afterEach, describe, expect, it, vi } from 'vitest';
import { assetAudio, assetSource, audioObjectUrl } from './live-media';

function pm(assets: Record<string, any>, live: Record<string, any>) {
  const listeners: Array<() => void> = [];
  return {
    proj: { assets },
    assets: { get: (id: string) => live[id] },
    bus: { on: (event: string, fn: () => void) => { if (event === 'assets') listeners.push(fn); return () => listeners.splice(listeners.indexOf(fn), 1); } },
    emit: () => listeners.forEach((fn) => fn()),
    listeners
  };
}

afterEach(() => vi.useRealTimers());

describe('media.assets.source and audio', () => {
  it('resolve with the same media the deprecated live fields hold', async () => {
    const video = { kind: 'video', url: 'blob:video', el: { currentSrc: 'blob:video' }, blob: new Blob([], { type: 'video/mp4' }), audioBlob: new Blob(['a']) };
    const PM = pm({ v: { id: 'v' } }, { v: video });
    await expect(assetSource(PM, 'v')).resolves.toEqual({ url: 'blob:video', kind: 'video', mime: 'video/mp4', rendition: false });
    await expect(assetAudio(PM, 'v')).resolves.toBe(video.audioBlob);
  });

  it('waits for the asset to load and its rendition to exist, then stops listening', async () => {
    vi.useFakeTimers();
    const live: Record<string, any> = {};
    const PM = pm({ v: { id: 'v' } }, live);
    const pending = assetSource(PM, 'v');
    live.v = { kind: 'video', playbackProxy: true };
    PM.emit();
    await vi.advanceTimersByTimeAsync(200);
    live.v.url = 'blob:rendition';
    await vi.advanceTimersByTimeAsync(100);
    await expect(pending).resolves.toEqual({ url: 'blob:rendition', kind: 'video', mime: null, rendition: true });
    expect(PM.listeners).toHaveLength(0);
  });

  it('gives an audio asset one object URL of its bytes', async () => {
    const created = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:audio');
    const audio = { kind: 'audio', audioBlob: new Blob(['x'], { type: 'audio/wav' }) };
    const PM = pm({ a: { id: 'a' } }, { a: audio });
    await expect(assetSource(PM, 'a')).resolves.toEqual({ url: 'blob:audio', kind: 'audio', mime: 'audio/wav', rendition: false });
    await assetSource(PM, 'a');
    expect(created).toHaveBeenCalledTimes(1);
    created.mockRestore();
  });

  it('revokes an audio asset\'s object URL when its bytes are replaced', () => {
    let n = 0;
    const created = vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:audio-${++n}`);
    const revoked = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    const live = {};
    const first = new Blob(['a']), second = new Blob(['b']);
    expect(audioObjectUrl(live, first)).toBe('blob:audio-1');
    expect(audioObjectUrl(live, first)).toBe('blob:audio-1');
    expect(revoked).not.toHaveBeenCalled();
    expect(audioObjectUrl(live, second)).toBe('blob:audio-2');
    expect(revoked).toHaveBeenCalledExactlyOnceWith('blob:audio-1');
    created.mockRestore();
    revoked.mockRestore();
  });

  it('is null for missing assets, models, silent videos, failures and timeouts', async () => {
    vi.useFakeTimers();
    const PM = pm({ m: {}, s: {}, f: {}, slow: {} }, { m: { kind: 'model' }, s: { kind: 'video', url: 'blob:s', hasAudio: false }, f: { kind: 'video', error: 'bad' }, slow: { kind: 'video' } });
    await expect(assetSource(PM, 'missing')).resolves.toBeNull();
    await expect(assetSource(PM, 'm')).resolves.toBeNull();
    await expect(assetAudio(PM, 's')).resolves.toBeNull();
    await expect(assetSource(PM, 'f')).resolves.toBeNull();
    const slow = assetSource(PM, 'slow', { timeoutMs: 500 });
    await vi.advanceTimersByTimeAsync(600);
    await expect(slow).resolves.toBeNull();
  });
});
