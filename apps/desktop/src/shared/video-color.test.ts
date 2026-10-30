import { readFile } from 'node:fs/promises';
import { describe, expect, it } from 'vitest';
import { mp4VideoColor, videoColorTreatment } from './video-color';

const u16 = (n: number) => { const b = new Uint8Array(2); new DataView(b.buffer).setUint16(0, n); return b; };
const uint = (n: number) => { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, n); return b; };
function box(type: string, ...parts: Uint8Array[]) {
  const size = 8 + parts.reduce((sum, part) => sum + part.length, 0);
  const bytes = new Uint8Array(size); bytes.set(uint(size)); bytes.set(new TextEncoder().encode(type), 4);
  let at = 8; for (const part of parts) { bytes.set(part, at); at += part.length; }
  return bytes;
}
function track(kind: string, ...entryChildren: Uint8Array[]) {
  const hdlr = new Uint8Array(12); hdlr.set(new TextEncoder().encode(kind), 8);
  const entry = box('hvc1', new Uint8Array(78), box('hvcC', new Uint8Array(4)), ...entryChildren);
  const stsd = box('stsd', uint(0), uint(1), entry);
  return box('trak', box('mdia', box('hdlr', hdlr), box('minf', box('stbl', stsd))));
}
const colr = (kind: 'nclx' | 'nclc', primaries: number, transfer: number, matrix: number, full = false) =>
  box('colr', new TextEncoder().encode(kind), u16(primaries), u16(transfer), u16(matrix), ...(kind === 'nclx' ? [new Uint8Array([full ? 0x80 : 0])] : []));

describe('video color tags', () => {
  it('reads iPhone-style HLG and QuickTime nclc tags from the video track only', async () => {
    const hlg = new Blob([box('moov', track('soun', colr('nclx', 1, 1, 1)), track('vide', box('pasp', new Uint8Array(8)), colr('nclx', 9, 18, 9)))]);
    expect(await mp4VideoColor(hlg)).toEqual({ primaries: 'bt2020', transfer: 'arib-std-b67', matrix: 'bt2020nc', range: 'tv' });
    const log = new Blob([box('moov', track('vide', colr('nclc', 9, 2, 9)))]);
    expect(await mp4VideoColor(log)).toEqual({ primaries: 'bt2020', matrix: 'bt2020nc' });
  });
  it('ignores untagged, ICC-only and damaged files', async () => {
    expect(await mp4VideoColor(new Blob([box('moov', track('vide'))]))).toBeUndefined();
    expect(await mp4VideoColor(new Blob([box('moov', track('vide', box('colr', new TextEncoder().encode('prof'), new Uint8Array(16))))]))).toBeUndefined();
    const valid = new Blob([box('moov', track('vide', colr('nclx', 9, 16, 9)))]);
    expect(await mp4VideoColor(valid.slice(0, valid.size - 4))).toBeUndefined();
    expect(await mp4VideoColor(new Blob(['not a video']))).toBeUndefined();
  });
  it('leaves the ordinary Rec.709 fixture to Chromium', async () => {
    const bytes = await readFile(new URL('../../e2e/fixtures/h264-aac.mp4', import.meta.url));
    expect(videoColorTreatment(await mp4VideoColor(new Blob([bytes])))).toBeNull();
  });
  it('tone maps HDR transfers and preserves BT.2020 code values', () => {
    expect(videoColorTreatment({ primaries: 'bt2020', transfer: 'arib-std-b67', matrix: 'bt2020nc' })).toBe('tonemap');
    expect(videoColorTreatment({ transfer: 'smpte2084' })).toBe('tonemap');
    expect(videoColorTreatment({ primaries: 'bt2020', transfer: 'bt709', matrix: 'bt2020nc' })).toBe('preserve');
    expect(videoColorTreatment({ matrix: 'bt2020c' })).toBe('preserve');
    expect(videoColorTreatment({ primaries: 'bt709', transfer: 'bt709', matrix: 'bt709' })).toBeNull();
    expect(videoColorTreatment({ matrix: 'smpte170m' })).toBeNull();
    expect(videoColorTreatment(undefined)).toBeNull();
  });
});
