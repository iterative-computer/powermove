import { createHash } from 'node:crypto';
import { runInNewContext } from 'node:vm';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { muxWebM } from './webm-mux';

beforeEach(() => vi.stubGlobal('window', globalThis));
afterEach(() => vi.unstubAllGlobals());

function payload(length: number, seed = 1): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(length + 11);
  for (let index = 0; index < bytes.length; index++) bytes[index] = (index * 13 + seed) & 255;
  return bytes.subarray(7, 7 + length);
}
const digest = async (blob: Blob) => createHash('sha256').update(new Uint8Array(await blob.arrayBuffer())).digest('hex');

// These complete-stream hashes were captured from the previous muxer before
// changing its byte assembly. They lock headers, sizes, track ordering, timing,
// cluster boundaries, and every payload byte independently of the new tree.
const legacyStreams: Array<[number, string]> = [
  [0, 'bce02263934b04c3aeec7c6dbb282739b78f70cee338e1f8922ad90378516da9'],
  [1, '9d540ceba7bd831c4a896814bba0cba6e1bc1329207b2bdc0e0f29b81917543b'],
  [122, '46e3976596146ddb5c55a9acc76374548faee9627ed378c081fbb7f630f3517b'],
  [123, '7515c3391596736c83146093d736a799f3f20db4f34167aa8e3eb82a56f08778'],
  [124, '785634e6d3d1d2a17f80507c69e518a63c3ed93cc351cd37ed468d1af6609ddf'],
  [126, '4237df9ea6a17884898d92a21521eb1b668106b7863b6d9a8985133a8f60ff93'],
  [127, 'a9041e70bf9deaf83576e1df9ecd348b00860946e2d1f563da0705bd2d8c98f1'],
  [128, '9192a2290deac2e12437813c5aa75cf3455a789bfadd2645b75a42489b549496'],
  [16377, 'd93526c0561579fda0828f75ca893aed8b5daf7aea5eefe058303bc00d15f424'],
  [16378, '90bf5bed5ccce2510ce61a5ccfb79e01b69adb7bf2c382d15f22b0e81d6a7d4e'],
  [16379, '16eebb7e21107501a686cdeefd0bf26fd87d0545611b2d5ea1d85fba9f49b84a'],
  [16382, '08dee33dc44bc7969c11bb646ef670bdab4a8763776fe3cdcef09fc2254132b0'],
  [16383, '0f2a73fc3f0dc010b8bb96bc61e7329265e0dbe314a76a1faa6af3066e91277f'],
  [16384, '5be71e43ad910c8d73c43e62951b40a79ac4e7259e7233146f4689c96c1cd971'],
  [2097145, 'c6ff4676562de822e5f95156732cd94f2328e00b6d494d5426b0c1834df7ec77'],
  [2097146, 'c39d878c2621d5aa09c8fd68c61eefbea62965d000706ded92e3a605345d94e9'],
  [2097147, '9310c49bb031b62b8a782d99d40b7758d8646de60821483997589a5f3df84c8e'],
];

describe('WebM assembly', () => {
  it.each(legacyStreams)('preserves the legacy stream for a %i-byte payload across EBML size boundaries', async (length, expected) => {
    const frames = [
      { ts: 0, key: true, data: payload(length) },
      { ts: 2_000_000, key: false, data: payload(3) },
      { ts: 2_001_000, key: true, data: payload(129) },
    ];
    const audio = { rate: 44100, channels: 1, priv: payload(19), chunks: [
      { ts: 2_001_000, data: payload(10) }, { ts: 0, data: payload(1) }, { ts: 2_000_000, data: payload(127) },
    ] };
    const blob = muxWebM(frames, { width: 17, height: 31, fps: 29.97, codecId: 'V_VP8', audio }, 'fixture');
    expect(blob.type).toBe('video/webm');
    expect(await digest(blob)).toBe(expected);
  });

  it('keeps the exact empty default-codec container', async () => {
    expect(await digest(muxWebM([], { width: 1, height: 1, fps: 30 }, 'fixture')))
      .toBe('c5eba4a3361cf2e90c3ecd05a17b40660e9ed66696cd83a1384103935971612c');
  });

  it.each([
    [10000, 8, '34800810f73f66915976fecb2849faa2aa291bd6b31d5f17949c41b9603174aa'],
    [300, 1024, '0342e202fdbd763506b5dc0021a79f5579bb0acf5249981dba0973362fab5d3d'],
  ] as const)('preserves %i small-frame packets when coalescing their headers and bytes', async (count, length, expected) => {
    const frames = Array.from({ length: count }, (_, index) => ({ ts: index * 33333, key: index % 60 === 0, data: payload(length, index) }));
    expect(await digest(muxWebM(frames, { width: 1920, height: 1080, fps: 30 }, 'fixture'))).toBe(expected);
  });

  it('snapshots small and large unaligned video/audio views sharing a backing buffer before returning', async () => {
    const backing = payload(20000);
    const frames = [{ ts: 0, key: true, data: backing.subarray(3, 5003) }, { ts: 33333, key: false, data: backing.subarray(77, 150) }];
    const audio = { chunks: [{ ts: 0, data: backing.subarray(5011, 10011) }, { ts: 20000, data: backing.subarray(11, 33) }], priv: backing.subarray(155, 174) };
    const copiedFrames = frames.map(frame => ({ ...frame, data: frame.data.slice() }));
    const copiedAudio = { ...audio, priv: audio.priv.slice(), chunks: audio.chunks.map(chunk => ({ ...chunk, data: chunk.data.slice() })) };
    const blob = muxWebM(frames, { width: 64, height: 64, fps: 30, audio }, 'fixture');
    backing.fill(0); audio.priv.fill(255); frames[0]!.data.fill(128);
    const expected = muxWebM(copiedFrames, { width: 64, height: 64, fps: 30, audio: copiedAudio }, 'fixture');
    expect(Buffer.compare(new Uint8Array(await blob.arrayBuffer()), new Uint8Array(await expected.arrayBuffer()))).toBe(0);
  });

  it('accepts byte views from another realm without losing their exact range', async () => {
    const foreign = runInNewContext('new Uint8Array([9, 1, 2, 3, 8]).subarray(1, 4)') as Uint8Array;
    expect(foreign instanceof Uint8Array).toBe(false);
    const options = { width: 64, height: 64, fps: 30 };
    const actual = muxWebM([{ ts: 0, key: true, data: foreign }], options, 'fixture');
    const expected = muxWebM([{ ts: 0, key: true, data: new Uint8Array([1, 2, 3]) }], options, 'fixture');
    expect(Buffer.compare(new Uint8Array(await actual.arrayBuffer()), new Uint8Array(await expected.arrayBuffer()))).toBe(0);
  });
});
