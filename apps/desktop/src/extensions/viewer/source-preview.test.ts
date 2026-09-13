// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { installSourcePreview } from './source-preview';
import type { PowermoveAPI, ViewerService } from 'powermove';

function stage(): HTMLElement {
  const host = document.createElement('div');
  host.id = 'stage';
  const inner = document.createElement('div');
  inner.id = 'stage-inner';
  host.append(inner);
  document.body.append(host);
  return host;
}

function harness(records: Record<string, { id?: string; kind: string; name?: string }>, live: Map<string, any>) {
  let playing = false;
  const pause = vi.fn(() => { playing = false; });
  const play = vi.fn(() => { playing = true; });
  const viewer = { preview: undefined, layout: vi.fn() } as unknown as ViewerService;
  const api = {
    project: { get: () => ({ assets: records }) },
    media: { assets: { get: (id: string) => live.get(id) } },
    transport: { playing: () => playing, pause, play },
    ui: { icon: () => '<svg></svg>' },
  } as unknown as PowermoveAPI;
  return { api, viewer, pause, play, setPlaying(value: boolean) { playing = value; } };
}

beforeEach(() => {
  vi.stubGlobal('URL', {
    ...URL,
    createObjectURL: vi.fn(() => 'blob:audio-preview'),
    revokeObjectURL: vi.fn(),
  });
  vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {});
});

afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('source preview', () => {
  it('names the source and offers a way back so the stage never reads as a broken composition', () => {
    const ctx = harness(
      { video: { id: 'video', kind: 'video', name: 'b-roll.mov' } },
      new Map([['video', { url: 'blob:video' }]])
    );
    const preview = installSourcePreview(ctx.api, ctx.viewer, stage());

    expect(preview.show('video')).toBe(true);
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    expect(root.dataset.open).toBe('true');
    expect(root.querySelector('.sp-badge')).toBeNull();
    expect(root.querySelector('.sp-name')?.textContent).toBe('b-roll.mov');
    expect(root.querySelector('.sp-hint')).toBeNull();
    expect(root.querySelector<HTMLElement>('.sp-foot')?.hidden).toBe(false);

    root.querySelector<HTMLButtonElement>('.sp-close')!.click();
    expect(preview.activeId).toBeNull();
    expect(root.dataset.open).toBe('false');

    preview.dispose();
  });

  it('shows the audio filename only once below the audio icon', () => {
    const ctx = harness(
      { audio: { id: 'audio', kind: 'audio', name: 'score.wav' } },
      new Map([['audio', { audioBlob: new Blob(['sound'], { type: 'audio/wav' }) }]])
    );
    const preview = installSourcePreview(ctx.api, ctx.viewer, stage());

    expect(preview.show('audio')).toBe(true);
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    expect(root.querySelector('.sp-media > audio')).not.toBeNull();
    expect(root.querySelector('.sp-audio .sp-name')?.textContent).toBe('score.wav');
    expect(root.querySelectorAll('.sp-name')).toHaveLength(1);
    expect(root.querySelector('.sp-foot')?.textContent).toBe('');
    expect(root.querySelector('.sp-media > img, .sp-media > video')).toBeNull();
    expect(root.querySelector<HTMLElement>('.sp-foot')?.hidden).toBe(false);
    expect(root.querySelector('.sp-time, .sp-dur')).toBeNull();

    preview.dispose();
  });

  it('hides the transport for a still image, which has nothing to play', () => {
    const ctx = harness(
      { still: { id: 'still', kind: 'image', name: 'plate.png' } },
      new Map([['still', { url: 'blob:image' }]])
    );
    const preview = installSourcePreview(ctx.api, ctx.viewer, stage());

    expect(preview.show('still')).toBe(true);
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    const foot = root.querySelector<HTMLElement>('.sp-foot')!;
    expect(foot.hidden).toBe(false);
    expect(root.querySelector<HTMLElement>('.sp-play')?.hidden).toBe(true);
    expect(root.querySelector<HTMLElement>('.sp-scrub')?.hidden).toBe(true);
    expect(foot.textContent).toBe('plate.png');

    preview.dispose();
  });

  it('seeks with the keyboard and keeps playback time accessible without visible timestamps', () => {
    const ctx = harness(
      { video: { kind: 'video', name: 'clip.mov' } },
      new Map([['video', { url: 'blob:video' }]])
    );
    const preview = installSourcePreview(ctx.api, ctx.viewer, stage());
    preview.show('video');
    const video = document.querySelector('video')!;
    Object.defineProperty(video, 'duration', { value: 12 });
    video.currentTime = 10;
    const scrub = document.querySelector<HTMLElement>('.sp-scrub')!;
    scrub.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    expect(video.currentTime).toBe(12);
    expect(scrub.getAttribute('aria-valuetext')).toBe('0:12 of 0:12');
    scrub.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    expect(video.currentTime).toBe(0);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(preview.activeId).toBeNull();
    preview.dispose();
  });

  it('pauses the composition once instead of restarting it behind the monitor', () => {
    const ctx = harness(
      { video: { id: 'video', kind: 'video' } },
      new Map([['video', { url: 'blob:video' }]])
    );
    ctx.setPlaying(true);
    // pause is provided by the typed transport fake.
    // play is provided by the typed transport fake.
    const preview = installSourcePreview(ctx.api, ctx.viewer, stage());

    expect(preview.show('video')).toBe(true);
    expect(ctx.pause).toHaveBeenCalledOnce();
    expect(ctx.play).not.toHaveBeenCalled();

    preview.dispose();
  });

  it('restores video preview from the live media element when its cached url field is absent', () => {
    const ctx = harness(
      { video: { id: 'video', kind: 'video' } },
      new Map([['video', { el: { currentSrc: 'blob:video-frame' } }]])
    );
    const preview = installSourcePreview(ctx.api, ctx.viewer, stage());

    expect(preview.show('video')).toBe(true);
    expect(document.querySelector<HTMLVideoElement>('#source-preview video')?.src).toBe('blob:video-frame');

    preview.dispose();
  });

  it('replaces only the preview controller during a hot update', () => {
    const ctx = harness({}, new Map());
    const host = stage();
    const first = installSourcePreview(ctx.api, ctx.viewer, host);
    const firstRoot = host.querySelector('#source-preview');
    const dispose = vi.spyOn(first, 'dispose');

    const second = installSourcePreview(ctx.api, ctx.viewer, host);

    expect(dispose).toHaveBeenCalledOnce();
    expect(host.querySelectorAll('#source-preview')).toHaveLength(1);
    expect(host.querySelector('#source-preview')).not.toBe(firstRoot);
    expect(ctx.viewer.preview).toBe(second);
    second.dispose();
  });
});
