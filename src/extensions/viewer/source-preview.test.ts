// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { installSourcePreview } from './source-preview';

function stage(): HTMLElement {
  const host = document.createElement('div');
  host.id = 'stage';
  const inner = document.createElement('div');
  inner.id = 'stage-inner';
  host.append(inner);
  document.body.append(host);
  return host;
}

function registry(records: Record<string, any>, live: Map<string, any>): Record<string, any> {
  return {
    proj: { assets: records },
    assets: { get: (id: string) => live.get(id) },
    Viewer: {},
    bus: { emit: vi.fn() },
    playing: false,
  };
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
    const PM = registry(
      { video: { id: 'video', kind: 'video', name: 'b-roll.mov' } },
      new Map([['video', { url: 'blob:video' }]])
    );
    const preview = installSourcePreview(PM, stage());

    expect(preview.show('video')).toBe(true);
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    expect(root.dataset.open).toBe('true');
    expect(root.querySelector('.sp-badge')?.textContent).toBe('Source \u00b7 Video');
    expect(root.querySelector('.sp-name')?.textContent).toBe('b-roll.mov');
    expect(root.querySelector('.sp-hint')?.textContent).toContain('Esc');
    expect(root.querySelector<HTMLElement>('.sp-foot')?.hidden).toBe(false);

    root.querySelector<HTMLButtonElement>('.sp-close')!.click();
    expect(preview.activeId).toBeNull();
    expect(root.dataset.open).toBe('false');

    preview.dispose();
  });

  it('gives audio a titled card instead of a black stage, with its own transport', () => {
    const PM = registry(
      { audio: { id: 'audio', kind: 'audio', name: 'score.wav' } },
      new Map([['audio', { audioBlob: new Blob(['sound'], { type: 'audio/wav' }) }]])
    );
    const preview = installSourcePreview(PM, stage());

    expect(preview.show('audio')).toBe(true);
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    expect(root.querySelector('.sp-media > audio')).not.toBeNull();
    expect(root.querySelector('.sp-audio b')?.textContent).toBe('score.wav');
    expect(root.querySelector('.sp-media > img, .sp-media > video')).toBeNull();
    expect(root.querySelector<HTMLElement>('.sp-foot')?.hidden).toBe(false);
    expect(root.querySelector('.sp-time')?.textContent).toBe('0:00');

    preview.dispose();
  });

  it('hides the transport for a still image, which has nothing to play', () => {
    const PM = registry(
      { still: { id: 'still', kind: 'image', name: 'plate.png' } },
      new Map([['still', { url: 'blob:image' }]])
    );
    const preview = installSourcePreview(PM, stage());

    expect(preview.show('still')).toBe(true);
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    const foot = root.querySelector<HTMLElement>('.sp-foot')!;
    expect(foot.hidden).toBe(true);
    /* The strip's own display:flex outranks the hidden attribute's UA rule,
       so the stylesheet has to opt out explicitly. */
    expect(window.getComputedStyle(foot).display).toBe('none');
    expect(root.querySelector<HTMLElement>('.sp-status')?.hidden).toBe(true);

    preview.dispose();
  });

  it('pauses the composition once instead of restarting it behind the monitor', () => {
    const PM = registry(
      { video: { id: 'video', kind: 'video' } },
      new Map([['video', { url: 'blob:video' }]])
    );
    PM.playing = true;
    PM.pause = vi.fn(() => { PM.playing = false; });
    PM.play = vi.fn();
    const preview = installSourcePreview(PM, stage());

    expect(preview.show('video')).toBe(true);
    expect(PM.pause).toHaveBeenCalledOnce();
    expect(PM.play).not.toHaveBeenCalled();

    preview.dispose();
  });

  it('restores video preview from the live media element when its cached url field is absent', () => {
    const PM = registry(
      { video: { id: 'video', kind: 'video' } },
      new Map([['video', { el: { currentSrc: 'blob:video-frame' } }]])
    );
    const preview = installSourcePreview(PM, stage());

    expect(preview.show('video')).toBe(true);
    expect(document.querySelector<HTMLVideoElement>('#source-preview video')?.src).toBe('blob:video-frame');

    preview.dispose();
  });

  it('replaces only the preview controller during a hot update', () => {
    const PM = registry({}, new Map());
    const host = stage();
    const first = installSourcePreview(PM, host);
    const firstRoot = host.querySelector('#source-preview');
    const dispose = vi.spyOn(first, 'dispose');

    const second = installSourcePreview(PM, host);

    expect(dispose).toHaveBeenCalledOnce();
    expect(host.querySelectorAll('#source-preview')).toHaveLength(1);
    expect(host.querySelector('#source-preview')).not.toBe(firstRoot);
    expect(PM.Viewer.preview).toBe(second);
    second.dispose();
  });
});
