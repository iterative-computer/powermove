import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './exporter';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';

afterEach(resetBridgeForTests);

function exporter(overrides: any = {}): any {
  (globalThis as any).window = globalThis;
  const PM: PMRegistry = {
    version: 'test',
    h: () => ({}),
    proj: { name: 't', fps: 30 },
    store: { get: () => ({}), set() {} },
    toast() {}, download() {}, modal() {}, row() { return {}; },
    selectField() {}, toggleField() {},
    round: (value: number) => value,
    tc: () => '0:00',
    renderFrameTo: () => null,
    GL: { canvas: { width: 2, height: 2 }, resize() {}, render() {} },
  };
  Object.assign(PM, overrides);
  install(PM);
  return PM.Export;
}

describe('legacy exporter install', () => {
  it('emits an EBML header and Segment for video-only input', async () => {
    const X = exporter();
    const blob = X.muxWebM(
      [{ ts: 0, key: true, data: new Uint8Array([1, 2, 3]) }, { ts: 33333, key: false, data: new Uint8Array([4]) }],
      { width: 1920, height: 1080, fps: 30 },
    );
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let segment = -1;
    for (let i = 30; i < bytes.length - 4; i++) {
      if (bytes[i] === 0x18 && bytes[i + 1] === 0x53 && bytes[i + 2] === 0x80 && bytes[i + 3] === 0x67) {
        segment = i;
        break;
      }
    }

    expect([...bytes.slice(0, 4)]).toEqual([0x1a, 0x45, 0xdf, 0xa3]);
    expect(segment).toBeGreaterThan(0);
  });

  it('interleaves Opus audio with the video track', async () => {
    const X = exporter();
    const audio = {
      chunks: [{ ts: 0, data: new Uint8Array([9]) }, { ts: 20000, data: new Uint8Array([8]) }],
      priv: new Uint8Array([0x4f, 0x70]), rate: 48000, channels: 2,
    };
    const blob = X.muxWebM(
      [{ ts: 0, key: true, data: new Uint8Array([1]) }, { ts: 10000, key: false, data: new Uint8Array([2]) }],
      { width: 64, height: 64, fps: 30, audio },
    );
    const bytes = new Uint8Array(await blob.arrayBuffer());
    const text = new TextDecoder().decode(bytes);
    let videoBlocks = 0;
    let audioBlocks = 0;
    for (let i = 0; i < bytes.length - 4; i++) {
      if (bytes[i] === 0xa3 && bytes[i + 2] === 0x81) videoBlocks++;
      if (bytes[i] === 0xa3 && bytes[i + 2] === 0x82) audioBlocks++;
    }

    expect(text).toContain('A_OPUS');
    expect(text).toContain('V_VP9');
    expect(videoBlocks).toBe(2);
    expect(audioBlocks).toBe(2);
  });

  it('starts another cluster after a two-second window', async () => {
    const X = exporter();
    const frames = [];
    for (let i = 0; i < 90; i++) frames.push({ ts: i * 33333, key: i % 60 === 0, data: new Uint8Array([i & 0xff]) });
    const blob = X.muxWebM(frames, { width: 16, height: 16, fps: 30 });
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let clusters = 0;
    for (let i = 0; i < bytes.length - 4; i++) {
      if (bytes[i] === 0x1f && bytes[i + 1] === 0x43 && bytes[i + 2] === 0xb6 && bytes[i + 3] === 0x75) clusters++;
    }

    expect(clusters).toBeGreaterThanOrEqual(2);
  });
});


describe('export lifecycle', () => {
  it('releases the export lock when progress setup throws, allowing retry', async () => {
    let attempts = 0;
    const X = exporter({
      proj: { name: 'Retry', w: 64, h: 64, fps: 30, dur: 1 },
      pause() {}, setTime() {},
      h() { attempts++; throw new Error('Preview setup failed'); }
    });
    const first = await X.run({ format: 'webm' });
    expect(first.error).toBe('Preview setup failed');
    expect(X.busy).toBe(false);
    const retry = await X.run({ format: 'webm' });
    expect(retry.error).toBe('Preview setup failed');
    expect(attempts).toBe(2);
    expect(X.busy).toBe(false);
  });
});


it('awaits the media binding acknowledgement before an export is submitted', async () => {
  let acknowledge!: () => void;
  const flushBindings = vi.fn(() => new Promise<void>(resolve => { acknowledge = resolve; }));
  const exportVideo = vi.fn(async () => ({ status: 'cancelled' }));
  installBridgeForTests({ nativeEngine: {
    status: async () => ({ enabled: true, processes: { job: { state: 'ready' } } }),
    onExportStatus: () => () => {}, exportVideo
  } } as any);
  const X = exporter({ HostMedia: { flushBindings }, proj: { id: 'P1', name: 'test', fps: 30, w: 64, h: 64, dur: 1, work: [0, 1] } });
  const exported = X.run({ format: 'still' });
  await vi.waitFor(() => expect(flushBindings).toHaveBeenCalled());
  expect(exportVideo).not.toHaveBeenCalled();
  acknowledge();
  expect(await exported).toEqual({ cancelled: true });
  expect(exportVideo).toHaveBeenCalledOnce();
});

describe('offline media before a current-exporter export (OFFLINE-1)', () => {
  /* The export stops where it starts writing (PM.pause throws 'Proceeded').
     The waiting sheet is the export sheet: its label and its Cancel action are recorded. */
  const setup = (assets: { loaded: string[]; loading: string[] }, options: { choice?: string; ready?: Promise<void> } = {}) => {
    const offlineExport = vi.fn(async () => options.choice ?? 'without');
    installBridgeForTests({ offlineExport } as any);
    const loading = new Set(assets.loading), loaded = new Set(assets.loaded);
    const toast = vi.fn();
    const sheets: Array<{ labels: string[]; cancel: () => void; closed: boolean }> = [];
    const element = (): any => {
      const el: any = { style: {}, hidden: false, children: [] as any[], classList: { add() {} }, setAttribute() {}, querySelector: () => null, getContext: () => null, parentElement: null };
      Object.defineProperty(el, 'textContent', { set(text: string) { sheets.at(-1)?.labels.push(text); }, get: () => '' });
      return el;
    };
    const X = exporter({
      proj: {
        id: 'P1', name: 'Offline', w: 64, h: 64, fps: 30, dur: 1,
        assets: Object.fromEntries(['a1', 'a2'].map(id => [id, { id, kind: 'video', name: `${id}.mov` }])),
        layers: ['a1', 'a2'].map(asset => ({ id: `L-${asset}`, type: 'video', d: { asset } })),
      },
      projectAssetsReady: options.ready ?? Promise.resolve(),
      assets: { loading, get: (id: string) => loaded.has(id) ? { id } : undefined },
      toast,
      h: (tag: string, ...children: any[]) => { const el = element(); for (const child of children) if (child?.style !== undefined) child.parentElement = el; return el; },
      modal: (config: any) => {
        const sheet = { labels: [] as string[], cancel: () => config.actions[0].run(), closed: false };
        sheets.push(sheet);
        return { el: element(), close: () => { sheet.closed = true; config.onClose?.(); } };
      },
      pause() { throw new Error('Proceeded'); }, setTime() {},
    });
    return { X, offlineExport, toast, loading, loaded, sheets };
  };

  it('waits for media that is still loading, includes it, and exports without asking', async () => {
    const { X, offlineExport, toast, loading, loaded, sheets } = setup({ loaded: ['a2'], loading: ['a1'] });
    const exported = X.run({ format: 'webm' });
    await new Promise(resolve => setTimeout(resolve, 250));
    loading.delete('a1'); loaded.add('a1');
    expect((await exported).error).toBe('Proceeded');
    expect(offlineExport).not.toHaveBeenCalled();
    expect(sheets).toHaveLength(1);
    expect(sheets[0]!.labels).toContain('Waiting for media…');
    expect(sheets[0]!.closed).toBe(true);
    expect(toast.mock.calls.map(call => String(call[0])).filter(text => /loading/i.test(text))).toEqual([]);
  });

  it('goes on as before once the wait ends, and asks only about media that is offline', async () => {
    const { X, offlineExport } = setup({ loaded: [], loading: ['a1'] });
    X.mediaWaitMs = 150;
    expect((await X.run({ format: 'webm' })).error).toBe('Proceeded');
    expect(offlineExport).toHaveBeenCalledOnce();
    expect(offlineExport).toHaveBeenCalledWith([{ asset: 'a2', name: 'a2.mov' }]);
  });

  it('bounds the wait for a media restore that never ends, then goes on as today without asking', async () => {
    const { X, offlineExport, sheets } = setup({ loaded: [], loading: [] }, { ready: new Promise<void>(() => {}) });
    X.mediaWaitMs = 200;
    const began = Date.now();
    expect((await X.run({ format: 'webm' })).error).toBe('Proceeded');
    expect(Date.now() - began).toBeLessThan(2000);
    expect(X.busy).toBe(false);
    expect(offlineExport).not.toHaveBeenCalled();
    expect(sheets[0]!.labels).toContain('Waiting for media…');
  });

  it('cancels from the waiting sheet', async () => {
    const { X, sheets } = setup({ loaded: [], loading: [] }, { ready: new Promise<void>(() => {}) });
    const exported = X.run({ format: 'webm' });
    await vi.waitFor(() => expect(sheets).toHaveLength(1));
    sheets[0]!.cancel();
    expect(await exported).toEqual({ cancelled: true });
    expect(X.busy).toBe(false);
    expect(sheets[0]!.closed).toBe(true);
  });

  it('asks about offline media: Export Without goes on, Cancel stops', async () => {
    const without = setup({ loaded: ['a1'], loading: [] });
    expect((await without.X.run({ format: 'webm' })).error).toBe('Proceeded');
    expect(without.offlineExport).toHaveBeenCalledWith([{ asset: 'a2', name: 'a2.mov' }]);
    expect(without.sheets).toHaveLength(0);
    const cancelled = setup({ loaded: ['a1'], loading: [] }, { choice: 'cancel' });
    expect(await cancelled.X.run({ format: 'webm' })).toEqual({ cancelled: true });
  });
});
