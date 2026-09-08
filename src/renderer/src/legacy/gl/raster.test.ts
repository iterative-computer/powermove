import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { makePM } from '../__tests__/make-pm';
import { install, videoImportFailureMessage, waitForPresentedVideoFrame } from './raster';

function rasterRegistry(): PMRegistry {
  const context2d: any = {
    font: '', letterSpacing: '', textBaseline: '', textAlign: '', fillStyle: '',
    scale() {}, fillText() {},
    measureText(text: string) {
      const width = Math.max(1, text.length * 48);
      const left = this.textAlign === 'center' ? width / 2 : this.textAlign === 'right' ? width : 0;
      return {
        width,
        actualBoundingBoxLeft: left,
        actualBoundingBoxRight: width - left,
        actualBoundingBoxAscent: 78,
        actualBoundingBoxDescent: 18,
      };
    },
  };
  vi.stubGlobal('window', {
    document: {
      createElement(_tag: string) {
        return { width: 0, height: 0, getContext: () => context2d };
      },
    },
  });
  const PM: PMRegistry = {
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
  };
  install(PM);
  return PM;
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('legacy raster install', () => {
  it('selects actionable codec warnings for mov and mp4 decode failures', () => {
    const mov = videoImportFailureMessage('prores-4444.MOV', {
      code: 4,
      message: ''
    });
    expect(mov).toContain("file's codec is not supported by this build");
    expect(mov).toMatch(/ProRes.*transcod/i);

    const mp4 = videoImportFailureMessage('unsupported.mp4', { code: 3, message: '' });
    expect(mp4).toContain("file's codec is not supported by this build");
    expect(mp4).toMatch(/transcode.*H\.264/i);
    expect(videoImportFailureMessage('unsupported.mp4', new Error('DEMUXER_ERROR_NO_SUPPORTED_STREAMS')))
      .toBe(mp4);
  });

  it('keeps the generic video failure for other formats and non-codec errors', () => {
    expect(videoImportFailureMessage('clip.webm', { code: 3, message: 'decode failed' }))
      .toBe('Could not read this video file');
    expect(videoImportFailureMessage('clip.mov', { code: 2, message: 'network error' }))
      .toBe('Could not read this video file');
  });

  it('distinguishes a real presented video frame from an audio-only advancing MOV', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('window', { setTimeout, clearTimeout });
    let presented: (() => void) | undefined;
    const playable = {
      requestVideoFrameCallback(callback: () => void) { presented = callback; return 7; },
      cancelVideoFrameCallback: vi.fn(),
      play: vi.fn().mockResolvedValue(undefined),
      pause: vi.fn(),
    };
    const success = waitForPresentedVideoFrame(playable, 1000);
    presented?.();
    await expect(success).resolves.toBe(true);
    expect(playable.pause).toHaveBeenCalledOnce();

    const audioOnly = {
      requestVideoFrameCallback: vi.fn(() => 9),
      cancelVideoFrameCallback: vi.fn(),
      play: vi.fn().mockResolvedValue(undefined),
      pause: vi.fn(),
    };
    const failure = waitForPresentedVideoFrame(audioOnly, 1000);
    await vi.advanceTimersByTimeAsync(1000);
    await expect(failure).resolves.toBe(false);
    expect(audioOnly.cancelVideoFrameCallback).toHaveBeenCalledWith(9);
  });

  it('keeps padded raster textures and tight text selection bounds', () => {
    const PM = rasterRegistry();
    const text = { text: 'Powermove', font: 'SF Pro Display', weight: 650, size: 100, tracking: 0, leading: 1, color: '#fff', align: 'center', italic: false };
    const raster = PM.raster({ type: 'text', d: text });

    expect(raster.h).toBeGreaterThan(250);
    expect(raster.selection.h).toBeLessThan(raster.h * .5);
    expect(raster.selection.w).toBeLessThan(raster.w);
    expect(raster.selection.w).toBeGreaterThan(350);
    expect(Math.abs(raster.selection.x0 + raster.selection.x1)).toBeLessThan(1);

    const left = PM.raster({ type: 'text', d: { ...text, align: 'left' } }).selection;
    const right = PM.raster({ type: 'text', d: { ...text, align: 'right' } }).selection;
    expect(left.x0 < 0 && left.x0 > -12 && left.x1 > 350).toBe(true);
    expect(right.x1 > 0 && right.x1 < 12 && right.x0 < -350).toBe(true);

    const layout = PM.textLayout({ ...text, text: 'AB\nC', leading: 1.1 });
    expect(layout.characters.map((piece: any) => piece.text)).toEqual(['A', 'B', 'C']);
  });

  it('wraps paragraph text inside the Type tool box and keeps the box selectable', () => {
    const PM = rasterRegistry();
    const paragraph = PM.raster({
      type: 'text',
      d: {
        text: 'one two three four', font: 'SF Pro Display', weight: 500, size: 40,
        tracking: 0, leading: 1, color: '#fff', align: 'left', italic: false,
        boxWidth: { v: 130, kf: [], expr: null }, boxHeight: { v: 100, kf: [], expr: null },
      },
    });

    expect(paragraph.selection.x0).toBeLessThan(0);
    expect(paragraph.selection.x1).toBeGreaterThan(130);
    expect(paragraph.selection.y1).toBeGreaterThan(100);
    expect(paragraph.w).toBeGreaterThan(130);
    expect(paragraph.h).toBeGreaterThan(100);

    const centered = PM.raster({
      type: 'text',
      d: {
        text: 'centered', font: 'SF Pro Display', weight: 500, size: 40,
        tracking: 0, leading: 1, color: '#fff', align: 'center', italic: false,
        boxWidth: { v: 130, kf: [], expr: null }, boxHeight: { v: 100, kf: [], expr: null },
      },
    }).selection;
    expect(Math.abs(centered.x0 + centered.x1)).toBeLessThan(1);
  });

  it('atomically replaces media in place and restores metadata plus runtime with one Undo and Redo', async () => {
    const disposed: any[] = [];
    const oldRuntime = { id: 'asset-1', name: 'old.wav', kind: 'audio', marker: 'old' };
    const newRuntime = { id: 'asset-1', name: 'new.wav', kind: 'audio', marker: 'new', persistBlob: new Blob(['new']) };
    vi.stubGlobal('window', {
      document: { createElement: () => ({ getContext: () => ({}) }) },
      navigator: { hardwareConcurrency: 4 },
      URL: { createObjectURL: () => 'blob:test', revokeObjectURL: vi.fn() },
      powermove: { media: { sourcePath: () => '/replacement/new.wav' } },
      setTimeout, clearTimeout,
    });
    const PM = makePM('core/history', 'gl/raster');
    const animatedLayer = {
      id: 'layer-1', type: 'audio', from: 4, dur: 8,
      d: { asset: 'asset-1', trim: 1.5 },
      p: { opacity: { v: 100, kf: [{ i: 'key-1', t: 0, v: 35 }], expr: null } },
    };
    const layerBefore = JSON.stringify(animatedLayer);
    const projectA = PM.proj = {
      id: 'project-1', revision: 2,
      assets: { 'asset-1': { id: 'asset-1', name: 'old.wav', kind: 'audio', storageKey: 'media:old', size: 3, persisted: true } },
      layers: [animatedLayer], comps: {},
    };
    PM.assets.map.set('asset-1', oldRuntime);
    PM.MediaImport = { fingerprint: vi.fn(async () => 'new-fingerprint'), storageKeyFor: (value: string) => `media:${value}` };
    PM.MediaStore = { put: vi.fn(async () => true) };
    PM.Audio = {
      accepts: () => true,
      prepareAsset: vi.fn(async () => newRuntime),
      disposeAsset: vi.fn((asset: any) => disposed.push(asset)),
      pause: vi.fn(), rebalanceCache: vi.fn(),
    };
    PM.touch = vi.fn();
    PM.autosave = vi.fn();
    PM.Viewer = { preview: { clear: vi.fn() } };
    PM.sel = { layers: [], keys: [], chan: null };
    PM.replaceProject = (project: any) => { PM.proj = project; };

    const file = { name: 'new.wav', type: 'audio/wav', size: 3 };
    await PM.assets.replace('asset-1', file);

    expect(PM.proj.assets['asset-1']).toMatchObject({ id: 'asset-1', name: 'new.wav', kind: 'audio', sourcePath: '/replacement/new.wav', persisted: true });
    expect(PM.assets.get('asset-1')).toBe(newRuntime);
    expect(JSON.stringify(PM.proj.layers[0])).toBe(layerBefore);
    expect(PM.hist.list()).toEqual(['Replace old.wav']);
    PM.hist.do('Move existing layer', () => { PM.proj.layers[0].from = 7; });
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.layers[0].from).toBe(4);
    PM.proj = JSON.parse(JSON.stringify(PM.proj));
    expect(PM.proj).not.toBe(projectA);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.assets['asset-1']).toMatchObject({ id: 'asset-1', name: 'old.wav', storageKey: 'media:old' });
    expect(PM.assets.get('asset-1')).toBe(oldRuntime);
    expect(JSON.stringify(PM.proj.layers[0])).toBe(layerBefore);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.proj.assets['asset-1'].name).toBe('new.wav');
    expect(PM.assets.get('asset-1')).toBe(newRuntime);

    const thirdRuntime = { id: 'asset-1', name: 'third.wav', kind: 'audio', marker: 'third', persistBlob: new Blob(['third']) };
    PM.Audio.prepareAsset.mockResolvedValueOnce(thirdRuntime);
    await PM.assets.replace('asset-1', { name: 'third.wav', type: 'audio/wav', size: 5 });
    expect(PM.assets.get('asset-1')).toBe(thirdRuntime);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.assets.get('asset-1')).toBe(newRuntime);
    PM.hist.do('New edit discards replacement redo', () => { PM.proj.layers[0].from = 6; });
    expect(disposed).toEqual([thirdRuntime]);

    PM.hist.clear();
    expect(disposed).toEqual([thirdRuntime, oldRuntime]);
    expect(PM.assets.get('asset-1')).toBe(newRuntime);
  });

  it('rejects incompatible, failed, and stale replacements without changing the original asset', async () => {
    let resolvePreparation!: (value: any) => void;
    const preparation = new Promise<any>((resolve) => { resolvePreparation = resolve; });
    const disposed: any[] = [];
    vi.stubGlobal('window', {
      document: { createElement: () => ({ getContext: () => ({}) }) },
      navigator: { hardwareConcurrency: 4 },
      URL: { createObjectURL: () => 'blob:test', revokeObjectURL: vi.fn() },
      powermove: { media: { sourcePath: () => '' } },
      setTimeout, clearTimeout,
    });
    const PM = makePM('core/history', 'gl/raster');
    const oldRuntime = { id: 'asset-1', name: 'old.wav', kind: 'audio' };
    const projectA: any = {
      id: 'A', assets: { 'asset-1': { id: 'asset-1', name: 'old.wav', kind: 'audio', storageKey: 'media:old' } }, layers: [], comps: {},
    };
    PM.proj = projectA;
    PM.assets.map.set('asset-1', oldRuntime);
    PM.MediaImport = { fingerprint: vi.fn(async () => 'replacement'), storageKeyFor: (value: string) => `media:${value}` };
    PM.MediaStore = { put: vi.fn(async () => true) };
    PM.Audio = {
      accepts: (file: any) => String(file.type).startsWith('audio/'),
      prepareAsset: vi.fn(() => preparation),
      disposeAsset: vi.fn((asset: any) => disposed.push(asset)),
      pause: vi.fn(), rebalanceCache: vi.fn(),
    };
    PM.touch = vi.fn();

    await expect(PM.assets.replace('asset-1', { name: 'wrong.png', type: 'image/png', size: 4 }))
      .rejects.toThrow('Choose an audio file');
    expect(PM.Audio.prepareAsset).not.toHaveBeenCalled();
    expect(PM.hist.list()).toEqual([]);

    const replacing = PM.assets.replace('asset-1', { name: 'new.wav', type: 'audio/wav', size: 4 });
    await Promise.resolve();
    const projectB = { id: 'B', assets: {}, layers: [], comps: {} };
    PM.proj = projectB;
    PM.assets.clear();
    const prepared = { id: 'asset-1', name: 'new.wav', kind: 'audio' };
    resolvePreparation(prepared);

    await expect(replacing).rejects.toThrow(/switched projects/);
    expect(projectA.assets['asset-1'].name).toBe('old.wav');
    expect(projectB.assets).toEqual({});
    expect(PM.hist.list()).toEqual([]);
    expect(disposed).toContain(prepared);
  });
});
