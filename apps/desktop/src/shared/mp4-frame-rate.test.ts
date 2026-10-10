import { readFile } from 'node:fs/promises';
import { describe, expect, it } from 'vitest';
import { mp4FrameRate } from './mp4-frame-rate';

const uint = (n: number) => { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, n); return b; };
function box(type: string, ...parts: Uint8Array[]) {
  const size = 8 + parts.reduce((sum, part) => sum + part.length, 0);
  const bytes = new Uint8Array(size); bytes.set(uint(size)); bytes.set(new TextEncoder().encode(type), 4);
  let at = 8; for (const part of parts) { bytes.set(part, at); at += part.length; }
  return bytes;
}
function track(kind: string, timescale: number, entries: number[][], version = 0) {
  const mdhd = new Uint8Array(version ? 32 : 24);
  mdhd[0] = version; mdhd.set(uint(timescale), version ? 20 : 12);
  const hdlr = new Uint8Array(12); hdlr.set(new TextEncoder().encode(kind), 8);
  const stts = box('stts', uint(0), uint(entries.length), ...entries.flatMap(([count, delta]) => [uint(count!), uint(delta!)]));
  return box('trak', box('mdia', box('hdlr', hdlr), box('mdhd', mdhd), box('minf', box('stbl', stts))));
}

describe('MP4 source frame rate', () => {
  it('reads the video track from the real MP4 fixture', async () => {
    const bytes = await readFile(new URL('../../e2e/fixtures/h264-aac.mp4', import.meta.url));
    expect(await mp4FrameRate(new Blob([bytes]))).toBe(30);
  });
  it.each([0, 1])('preserves fractional timing with mdhd version %s and ignores audio', async version => {
    const blob = new Blob([box('moov', track('soun', 48000, [[100, 1024]]), track('vide', 30000, [[60, 1001]], version))]);
    expect(await mp4FrameRate(blob)).toBe(30000 / 1001);
  });
  it('averages variable timing and skips a large compressed payload', async () => {
    const payload = box('mdat', new Uint8Array(1024 * 1024));
    const source = new Blob([payload, box('moov', track('vide', 1000, [[10, 20], [10, 40]]))]);
    let read = 0;
    const blob = { size: source.size, slice(start: number, end: number) { read += end - start; return source.slice(start, end); } } as Blob;
    expect(await mp4FrameRate(blob)).toBeCloseTo(1000 / 30);
    expect(read).toBeLessThan(1024);
  });
  it('ignores missing, truncated and zero timing', async () => {
    expect(await mp4FrameRate(new Blob(['not a video']))).toBeUndefined();
    const valid = new Blob([box('moov', track('vide', 30, [[10, 1]]))]);
    expect(await mp4FrameRate(valid.slice(0, valid.size - 1))).toBeUndefined();
    expect(await mp4FrameRate(new Blob([box('moov', track('vide', 30, [[10, 0]]))]))).toBeUndefined();
  });
});
