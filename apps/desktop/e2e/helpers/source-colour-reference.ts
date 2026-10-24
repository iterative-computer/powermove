/** Independent double-precision colour reference for source-monitor gates.
 *
 * A decoded Y'CbCr sample becomes display code values through the stream's
 * matrix and range (ITU-T H.273), the section 9 transfer reading (SDR video
 * transfers read as the sRGB curve, extended to all reals; Chromium makes the
 * same choice for BT.709/SMPTE 170M), a linear-light D65 primaries change,
 * Bradford adaptation to the ICC D50 PCS, and the capture's own display ICC
 * matrix/TRC. Superwhites and sub-blacks are kept through the extended
 * curve and the primaries change, as composition keeps them at float
 * depths; only the display encode clips, as the monitor's display does.
 *
 * https://www.itu.int/rec/T-REC-H.273
 * https://www.color.org/specifications/
 */
import { inflateSync } from 'node:zlib';

export type ColourTriple = readonly [number, number, number];
/** The colour reading of one decoded frame: tags, or the section 9 defaults. */
export type VideoTags = { matrix: 'bt601' | 'bt709' | 'bt2020'; fullRange: boolean; primaries: 'smpte170m' | 'bt470bg' | 'bt709' | 'bt2020' };
type Matrix3 = readonly [ColourTriple, ColourTriple, ColourTriple];
type ToneCurve = { readonly decode: (code: number) => number; readonly encode: (linear: number) => number };
export type MatrixTrcProfile = {
  /** RGB linear-light to D50 PCS XYZ, rows ordered X/Y/Z. */
  readonly rgbToXyzD50: Matrix3;
  readonly trc: readonly [ToneCurve, ToneCurve, ToneCurve];
};

const D65: ColourTriple = [0.95047, 1, 1.08883];
const D50: ColourTriple = [0.9642, 1, 0.8249];
const BRADFORD: Matrix3 = [
  [0.8951, 0.2664, -0.1614],
  [-0.7502, 1.7135, 0.0367],
  [0.0389, -0.0685, 1.0296]
];
type Chromaticities = readonly [readonly [number, number], readonly [number, number], readonly [number, number]];
const PRIMARIES: Record<VideoTags['primaries'], Chromaticities> = {
  bt709: [[0.640, 0.330], [0.300, 0.600], [0.150, 0.060]],
  smpte170m: [[0.630, 0.340], [0.310, 0.595], [0.155, 0.070]],
  bt470bg: [[0.640, 0.330], [0.290, 0.600], [0.150, 0.060]],
  bt2020: [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]]
};
const MATRIX: Record<VideoTags['matrix'], { kr: number; kb: number }> = {
  bt709: { kr: 0.2126, kb: 0.0722 },
  bt601: { kr: 0.299, kb: 0.114 },
  bt2020: { kr: 0.2627, kb: 0.0593 }
};
const MAX_ICC_BYTES = 8 * 1024 * 1024;

function multiply(m: Matrix3, v: ColourTriple): ColourTriple {
  return [m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
    m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
    m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2]];
}

function inverse(m: Matrix3): Matrix3 {
  const [a, b, c] = m;
  const cof: Matrix3 = [
    [b[1] * c[2] - b[2] * c[1], a[2] * c[1] - a[1] * c[2], a[1] * b[2] - a[2] * b[1]],
    [b[2] * c[0] - b[0] * c[2], a[0] * c[2] - a[2] * c[0], a[2] * b[0] - a[0] * b[2]],
    [b[0] * c[1] - b[1] * c[0], a[1] * c[0] - a[0] * c[1], a[0] * b[1] - a[1] * b[0]]
  ];
  const det = a[0] * cof[0][0] + a[1] * cof[1][0] + a[2] * cof[2][0];
  if (!Number.isFinite(det) || Math.abs(det) < 1e-12) throw new Error('Singular ICC colourant matrix');
  return cof.map(row => row.map(value => value / det) as unknown as ColourTriple) as unknown as Matrix3;
}

function deriveRgbToXyz(xy: Chromaticities, white: ColourTriple): Matrix3 {
  const primaries = xy.map(([x, y]) => [x / y, 1, (1 - x - y) / y] as ColourTriple);
  const unscaled: Matrix3 = [
    [primaries[0]![0], primaries[1]![0], primaries[2]![0]],
    [1, 1, 1],
    [primaries[0]![2], primaries[1]![2], primaries[2]![2]]
  ];
  const scale = multiply(inverse(unscaled), white);
  return unscaled.map(row => row.map((value, column) => value * scale[column]!) as unknown as ColourTriple) as unknown as Matrix3;
}

function adaptBradford(xyz: ColourTriple): ColourTriple {
  const sourceWhite = multiply(BRADFORD, D65);
  const destWhite = multiply(BRADFORD, D50);
  const lms = multiply(BRADFORD, xyz);
  return multiply(inverse(BRADFORD), [
    lms[0] * destWhite[0] / sourceWhite[0],
    lms[1] * destWhite[1] / sourceWhite[1],
    lms[2] * destWhite[2] / sourceWhite[2]
  ]);
}

/** The sRGB curve, odd-extended to all reals (section 9's display encoding E). */
export function srgbEotf(signal: number): number {
  const magnitude = Math.abs(signal);
  const linear = magnitude <= 0.04045 ? magnitude / 12.92 : ((magnitude + 0.055) / 1.055) ** 2.4;
  return Math.sign(signal) * linear;
}

function ascii(bytes: Uint8Array, at: number, length: number): string {
  if (at < 0 || length < 0 || at + length > bytes.length) throw new Error('Truncated ICC or PNG');
  return String.fromCharCode(...bytes.subarray(at, at + length));
}

function be16(bytes: Uint8Array, at: number): number {
  if (at < 0 || at + 2 > bytes.length) throw new Error('Truncated ICC');
  return (bytes[at]! << 8) | bytes[at + 1]!;
}

function be32(bytes: Uint8Array, at: number): number {
  if (at < 0 || at + 4 > bytes.length) throw new Error('Truncated ICC or PNG');
  return ((bytes[at]! * 0x1000000) + (bytes[at + 1]! << 16) + (bytes[at + 2]! << 8) + bytes[at + 3]!) >>> 0;
}

function signedFixed16(bytes: Uint8Array, at: number): number {
  const value = be32(bytes, at);
  return (value > 0x7fffffff ? value - 0x100000000 : value) / 65536;
}

/** Unpacks only an embedded PNG iCCP profile; colour metadata is never guessed. */
export function extractPngIcc(png: Uint8Array): Uint8Array {
  if (png.length < 8 || ![137, 80, 78, 71, 13, 10, 26, 10].every((byte, at) => png[at] === byte))
    throw new Error('Invalid PNG signature');
  let profile: Uint8Array | null = null;
  for (let at = 8; at + 12 <= png.length;) {
    const length = be32(png, at), type = ascii(png, at + 4, 4);
    const start = at + 8, end = start + length;
    if (end + 4 > png.length) throw new Error('Truncated PNG chunk');
    if (type === 'iCCP') {
      if (profile) throw new Error('Multiple PNG iCCP profiles');
      let separator = start;
      while (separator < end && png[separator] !== 0) separator++;
      if (separator === start || separator + 2 > end || png[separator + 1] !== 0)
        throw new Error('Invalid PNG iCCP compression header');
      profile = inflateSync(png.subarray(separator + 2, end), { maxOutputLength: MAX_ICC_BYTES });
    }
    at = end + 4;
    if (type === 'IEND') break;
  }
  if (!profile) throw new Error('PNG has no embedded ICC profile');
  return profile;
}

function curveFromDecode(decode: (x: number) => number): ToneCurve {
  let previous = decode(0);
  if (!Number.isFinite(previous)) throw new Error('Invalid ICC tone curve');
  for (let i = 1; i <= 1024; i++) {
    const next = decode(i / 1024);
    if (!Number.isFinite(next) || next + 1e-9 < previous) throw new Error('Non-monotonic ICC tone curve');
    previous = next;
  }
  return {
    decode,
    encode(linear) {
      if (linear <= decode(0)) return 0;
      if (linear >= decode(1)) return 1;
      let lo = 0, hi = 1;
      for (let i = 0; i < 52; i++) {
        const mid = (lo + hi) / 2;
        if (decode(mid) < linear) lo = mid; else hi = mid;
      }
      return (lo + hi) / 2;
    }
  };
}

function parseToneCurve(bytes: Uint8Array): ToneCurve {
  if (bytes.length < 12) throw new Error('Truncated ICC tone curve');
  const kind = ascii(bytes, 0, 4);
  if (kind === 'curv') {
    const count = be32(bytes, 8);
    if (count === 0) return curveFromDecode(x => x);
    if (count === 1) {
      if (bytes.length < 14) throw new Error('Truncated ICC gamma curve');
      const gamma = be16(bytes, 12) / 256;
      if (gamma <= 0) throw new Error('Invalid ICC gamma curve');
      return curveFromDecode(x => x ** gamma);
    }
    if (count > 65536 || 12 + count * 2 > bytes.length) throw new Error('Truncated ICC sampled curve');
    const samples = Array.from({ length: count }, (_, i) => be16(bytes, 12 + i * 2) / 65535);
    return curveFromDecode(x => {
      const at = x * (count - 1), left = Math.min(count - 2, Math.floor(at));
      return samples[left]! + (samples[left + 1]! - samples[left]!) * (at - left);
    });
  }
  if (kind !== 'para') throw new Error(`Unsupported ICC tone curve type ${kind}`);
  const fn = be16(bytes, 8);
  const sizes = [1, 3, 4, 5, 7];
  const count = sizes[fn];
  if (count === undefined || 12 + 4 * count > bytes.length) throw new Error('Invalid ICC parametric curve');
  const p = Array.from({ length: count }, (_, i) => signedFixed16(bytes, 12 + 4 * i));
  const [g, a = 1, b = 0, c = 0, d = 0, e = 0, f = 0] = p;
  if (!(g! > 0) || a <= 0) throw new Error('Invalid ICC parametric curve parameters');
  const decode = (x: number): number => {
    switch (fn) {
      case 0: return x ** g!;
      case 1: return x >= -b / a ? (a * x + b) ** g! : 0;
      case 2: return x >= -b / a ? (a * x + b) ** g! + c : c;
      case 3: return x >= d ? (a * x + b) ** g! : c * x;
      case 4: return x >= d ? (a * x + b) ** g! + e : c * x + f;
      default: throw new Error('Invalid ICC parametric curve');
    }
  };
  return curveFromDecode(decode);
}

/** Parses an RGB matrix/TRC display profile in the ICC D50 XYZ PCS. */
export function parseMatrixTrcIcc(icc: Uint8Array): MatrixTrcProfile {
  if (icc.length < 132 || icc.length > MAX_ICC_BYTES || be32(icc, 0) > icc.length || ascii(icc, 36, 4) !== 'acsp'
      || ascii(icc, 16, 4) !== 'RGB ' || ascii(icc, 20, 4) !== 'XYZ ')
    throw new Error('Unsupported ICC profile header or PCS');
  const illuminant: ColourTriple = [signedFixed16(icc, 68), signedFixed16(icc, 72), signedFixed16(icc, 76)];
  if (illuminant.some((value, i) => Math.abs(value - D50[i]!) > 0.002)) throw new Error('ICC PCS illuminant is not D50');
  const count = be32(icc, 128);
  if (count > 4096 || 132 + count * 12 > icc.length) throw new Error('Invalid ICC tag table');
  const tags = new Map<string, Uint8Array>();
  for (let i = 0; i < count; i++) {
    const at = 132 + i * 12, signature = ascii(icc, at, 4);
    const offset = be32(icc, at + 4), length = be32(icc, at + 8);
    if (offset < 128 || !length || offset + length > icc.length) throw new Error('Invalid ICC tag span');
    if (tags.has(signature)) throw new Error(`Duplicate ICC tag ${signature}`);
    tags.set(signature, icc.subarray(offset, offset + length));
  }
  const xyz = (key: string): ColourTriple => {
    const tag = tags.get(key);
    if (!tag || tag.length < 20 || ascii(tag, 0, 4) !== 'XYZ ') throw new Error(`Missing ICC ${key} colourant`);
    return [signedFixed16(tag, 8), signedFixed16(tag, 12), signedFixed16(tag, 16)];
  };
  const r = xyz('rXYZ'), g = xyz('gXYZ'), b = xyz('bXYZ');
  const trc = (key: string): ToneCurve => {
    const tag = tags.get(key);
    if (!tag) throw new Error(`Missing ICC ${key} tone curve`);
    return parseToneCurve(tag);
  };
  const rgbToXyzD50: Matrix3 = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
  inverse(rgbToXyzD50); // Reject singular profiles before a gate compares pixels.
  return { rgbToXyzD50, trc: [trc('rTRC'), trc('gTRC'), trc('bTRC')] };
}

export const readPngMatrixTrcProfile = (png: Uint8Array): MatrixTrcProfile => parseMatrixTrcIcc(extractPngIcc(png));

export function displayCodeToXyzD50(profile: MatrixTrcProfile, codes: ColourTriple): ColourTriple {
  if (codes.some(code => !Number.isFinite(code) || code < 0 || code > 255)) throw new Error('Invalid display RGB code');
  return multiply(profile.rgbToXyzD50, [
    profile.trc[0].decode(codes[0] / 255),
    profile.trc[1].decode(codes[1] / 255),
    profile.trc[2].decode(codes[2] / 255)
  ]);
}

/** Unrounded display code values (0–255) for one Y'CbCr sample. */
export function referenceDisplayCodes(profile: MatrixTrcProfile, yuv: ColourTriple, tags: VideoTags, bits: 8 | 10 = 8): ColourTriple {
  const top = 2 ** bits - 1, scale = 2 ** (bits - 8);
  if (yuv.some(code => !Number.isInteger(code) || code < 0 || code > top)) throw new Error('Invalid YUV code');
  const [y, cb, cr] = tags.fullRange
    ? [yuv[0] / top, (yuv[1] - 128 * scale) / top, (yuv[2] - 128 * scale) / top]
    : [(yuv[0] - 16 * scale) / (219 * scale), (yuv[1] - 128 * scale) / (224 * scale), (yuv[2] - 128 * scale) / (224 * scale)];
  const { kr, kb } = MATRIX[tags.matrix];
  const kg = 1 - kr - kb;
  const signal: ColourTriple = [
    y + 2 * (1 - kr) * cr,
    y - 2 * kb * (1 - kb) / kg * cb - 2 * kr * (1 - kr) / kg * cr,
    y + 2 * (1 - kb) * cb
  ];
  const xyz = adaptBradford(multiply(deriveRgbToXyz(PRIMARIES[tags.primaries], D65), signal.map(srgbEotf) as unknown as ColourTriple));
  const display = multiply(inverse(profile.rgbToXyzD50), xyz);
  return display.map((channel, index) => {
    const trc = profile.trc[index]!;
    return channel <= trc.decode(0) ? 0 : channel >= trc.decode(1) ? 255 : 255 * trc.encode(channel);
  }) as unknown as ColourTriple;
}
