import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './exporter';

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
