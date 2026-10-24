import { deflateSync } from 'node:zlib';
import { describe, expect, it } from 'vitest';
import { displayCodeToXyzD50, extractPngIcc, parseMatrixTrcIcc, readPngMatrixTrcProfile,
  referenceDisplayCodes, srgbEotf } from './source-colour-reference';

type Triple = readonly [number, number, number];
type Matrix = readonly [Triple, Triple, Triple];
const SRGB_D50: Matrix = [
  [0.4360747, 0.3850649, 0.1430804],
  [0.2225045, 0.7168786, 0.0606169],
  [0.0139322, 0.0971045, 0.7141733]
];
const DISPLAY_P3_D50: Matrix = [
  [0.515102, 0.291965, 0.157153],
  [0.241182, 0.692236, 0.0665819],
  [-0.001049, 0.041881, 0.784378]
];

function fixed(value: number): Buffer {
  const bytes = Buffer.alloc(4);
  bytes.writeInt32BE(Math.round(value * 65536));
  return bytes;
}

function xyzTag(xyz: Triple): Buffer {
  return Buffer.concat([Buffer.from('XYZ '), Buffer.alloc(4), ...xyz.map(fixed)]);
}

function parametricSrgbTrc(): Buffer {
  const header = Buffer.alloc(12);
  header.write('para', 0, 'ascii');
  header.writeUInt16BE(4, 8);
  return Buffer.concat([header, ...[2.4, 1 / 1.055, 0.055 / 1.055, 1 / 12.92, 0.04045, 0, 0].map(fixed)]);
}

function sampledLinearTrc(): Buffer {
  const header = Buffer.alloc(12);
  header.write('curv', 0, 'ascii');
  header.writeUInt32BE(3, 8);
  const samples = Buffer.alloc(6);
  samples.writeUInt16BE(0, 0);
  samples.writeUInt16BE(32768, 2);
  samples.writeUInt16BE(65535, 4);
  return Buffer.concat([header, samples]);
}

function gammaTrc(): Buffer {
  const tag = Buffer.alloc(14);
  tag.write('curv', 0, 'ascii');
  tag.writeUInt32BE(1, 8);
  tag.writeUInt16BE(2 * 256, 12);
  return tag;
}

function syntheticIcc(matrix: Matrix, trc = parametricSrgbTrc()): Buffer {
  const colourant = (column: number): Triple => [matrix[0][column]!, matrix[1][column]!, matrix[2][column]!];
  const tags: Array<[string, Buffer]> = [
    ['rXYZ', xyzTag(colourant(0))], ['gXYZ', xyzTag(colourant(1))], ['bXYZ', xyzTag(colourant(2))],
    ['rTRC', trc], ['gTRC', trc], ['bTRC', trc]
  ];
  const table = Buffer.alloc(4 + tags.length * 12);
  table.writeUInt32BE(tags.length, 0);
  let offset = 128 + table.length;
  const payload: Buffer[] = [];
  for (const [index, [signature, data]] of tags.entries()) {
    table.write(signature, 4 + index * 12, 'ascii');
    table.writeUInt32BE(offset, 8 + index * 12);
    table.writeUInt32BE(data.length, 12 + index * 12);
    payload.push(data);
    offset += data.length;
  }
  const header = Buffer.alloc(128);
  header.writeUInt32BE(offset, 0);
  header.write('mntr', 12, 'ascii');
  header.write('RGB ', 16, 'ascii');
  header.write('XYZ ', 20, 'ascii');
  header.write('acsp', 36, 'ascii');
  fixed(0.9642).copy(header, 68);
  fixed(1).copy(header, 72);
  fixed(0.8249).copy(header, 76);
  return Buffer.concat([header, table, ...payload]);
}

function pngChunk(type: string, payload: Buffer): Buffer {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(payload.length);
  // The parser reads colour metadata rather than pixels; a valid PNG CRC is
  // still supplied so this fixture is independently inspectable.
  let crc = 0xffffffff;
  for (const byte of Buffer.concat([Buffer.from(type), payload])) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
  return Buffer.concat([length, Buffer.from(type), payload, checksum]);
}

function syntheticPng(icc: Buffer): Buffer {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(1, 0); ihdr.writeUInt32BE(1, 4); ihdr[8] = 8; ihdr[9] = 6;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    pngChunk('IHDR', ihdr),
    pngChunk('iCCP', Buffer.concat([Buffer.from('synthetic\0\0'), deflateSync(icc)])),
    pngChunk('IDAT', deflateSync(Buffer.from([0, 0, 0, 0, 255]))),
    pngChunk('IEND', Buffer.alloc(0))]);
}

describe('independent PNG ICC and video colour reference', () => {
  it('extracts matrix/TRC profiles from a PNG and maps known sRGB/P3 colourants into D50 PCS', () => {
    for (const matrix of [SRGB_D50, DISPLAY_P3_D50]) {
      const png = syntheticPng(syntheticIcc(matrix));
      const profile = readPngMatrixTrcProfile(png);
      expect(extractPngIcc(png)).toEqual(syntheticIcc(matrix));
      const red = displayCodeToXyzD50(profile, [255, 0, 0]);
      for (let axis = 0; axis < 3; axis++) expect(red[axis]).toBeCloseTo(matrix[axis]![0], 4);
      const white = displayCodeToXyzD50(profile, [255, 255, 255]);
      for (let axis = 0; axis < 3; axis++)
        expect(white[axis]).toBeCloseTo(matrix[axis]!.reduce((a, b) => a + b, 0), 4);
    }
  });

  it('reads SD red through its SMPTE 170M primaries and the composition reading through BT.709', () => {
    const p3 = readPngMatrixTrcProfile(syntheticPng(syntheticIcc(DISPLAY_P3_D50)));
    const red = [81, 90, 240] as const;
    const tagged = referenceDisplayCodes(p3, red, { matrix: 'bt601', fullRange: false, primaries: 'smpte170m' });
    const compat = referenceDisplayCodes(p3, red, { matrix: 'bt601', fullRange: false, primaries: 'bt709' });
    // Today's CSS video showed 228,62,34 and the composition path 233,51,35.
    expect(tagged.map(Math.round)).toEqual([228, 62, 33]);
    expect(compat.map(Math.round)).toEqual([233, 51, 35]);
  });

  it('keeps BT.709 neutrals on the sRGB curve on an sRGB display', () => {
    const srgb = readPngMatrixTrcProfile(syntheticPng(syntheticIcc(SRGB_D50)));
    for (const code of [0, 16, 64, 128, 192, 255]) {
      const out = referenceDisplayCodes(srgb, [code, 128, 128], { matrix: 'bt709', fullRange: true, primaries: 'bt709' });
      for (const channel of out) expect(channel).toBeCloseTo(code, 1);
    }
    const ten = referenceDisplayCodes(srgb, [512, 512, 512], { matrix: 'bt709', fullRange: false, primaries: 'bt709' }, 10);
    const eight = referenceDisplayCodes(srgb, [128, 128, 128], { matrix: 'bt709', fullRange: false, primaries: 'bt709' });
    for (let channel = 0; channel < 3; channel++) expect(ten[channel]).toBeCloseTo(eight[channel]!, 6);
    expect(srgbEotf(-0.5)).toBe(-srgbEotf(0.5));
    expect(srgbEotf(0.04045)).toBeCloseTo(0.04045 / 12.92, 12);
  });

  it('decodes and numerically inverts gamma and sampled ICC TRCs', () => {
    const gamma = parseMatrixTrcIcc(syntheticIcc(SRGB_D50, gammaTrc()));
    const sample = parseMatrixTrcIcc(syntheticIcc(SRGB_D50, sampledLinearTrc()));
    expect(gamma.trc[0].decode(0.5)).toBeCloseTo(0.25, 8);
    expect(gamma.trc[0].encode(0.25)).toBeCloseTo(0.5, 8);
    expect(sample.trc[0].decode(0.5)).toBeCloseTo(0.5000076, 5);
    expect(sample.trc[0].encode(sample.trc[0].decode(0.37))).toBeCloseTo(0.37, 8);
  });

  it('rejects absent ICC profiles, corrupt tag bounds and non-XYZ PCS', () => {
    expect(() => extractPngIcc(Buffer.from('not a PNG'))).toThrow(/PNG signature/);
    const missingIcc = Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), pngChunk('IEND', Buffer.alloc(0))]);
    expect(() => extractPngIcc(missingIcc)).toThrow(/no embedded ICC/);
    const badSpan = syntheticIcc(SRGB_D50);
    badSpan.writeUInt32BE(badSpan.length + 1, 132 + 4);
    expect(() => parseMatrixTrcIcc(badSpan)).toThrow(/tag span/);
    const badPcs = syntheticIcc(SRGB_D50);
    badPcs.write('Lab ', 20, 'ascii');
    expect(() => parseMatrixTrcIcc(badPcs)).toThrow(/header or PCS/);
  });
});
