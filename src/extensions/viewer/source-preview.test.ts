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
  it('plays audio over a visually empty source monitor with no bottom strip', () => {
    const PM = registry(
      { audio: { id: 'audio', kind: 'audio' } },
      new Map([['audio', { audioBlob: new Blob(['sound'], { type: 'audio/wav' }) }]])
    );
    const preview = installSourcePreview(PM, stage());

    expect(preview.show('audio')).toBe(true);
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    expect(root.dataset.open).toBe('true');
    expect(root.querySelector('.sp-media')?.children).toHaveLength(1);
    expect(root.querySelector('.sp-media > audio')).not.toBeNull();
    expect(root.querySelector('.sp-media > img, .sp-media > video')).toBeNull();
    expect(root.querySelector('.sp-progress')).toBeNull();

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
