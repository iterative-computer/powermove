/**
 * Still images for the render parity cases of pm-render's image classes
 * (docs/engine/pm-render.md, "Image classes"): detailed content (smooth
 * gradients, saturated hard edges that stress chroma upsampling, fine
 * high-frequency detail) in every container and encoding the native
 * renderer claims to draw, so the comparison against the legacy renderer
 * exercises decoding (IDCT, chroma upsampling, 16-bit reduction, palette and
 * color-key transparency) and color management (Display P3), not only flat
 * colors.
 *
 * PNG and JPEG are encoded here, dependency-free and deterministic. WebP,
 * AVIF, and HEIC are derived from a PNG generated here with the system
 * encoders (`cwebp` from libwebp; `/usr/bin/sips`), which are deterministic
 * for a given encoder version; the capture records each file's hash and the
 * Rust parity test reads the captured bytes.
 */
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { deflateSync } from 'node:zlib';
import { crc32 } from './png';

// ---------------------------------------------------------------------------
// Content

export interface Raster { width: number; height: number; channels: 1 | 2 | 3 | 4; data: Uint8Array }

const clamp8 = (v: number) => v < 0 ? 0 : v > 255 ? 255 : Math.round(v);
/** A triangle wave in [0, 255] with the given integer period. */
const tri = (v: number, period: number) => { const m = ((v % period) + period) % period; return Math.round(510 * Math.min(m, period - m) / period); };

/**
 * A photo-like RGB test card: a smooth two-axis gradient (banding and IDCT
 * rounding), a saturated checker (chroma edges at every 4:2:0 phase), fine
 * one-pixel lines and a zone-plate-like ring pattern (high frequencies),
 * and a skin-tone ramp.
 */
export function photo(width: number, height: number): Raster {
  const data = new Uint8Array(width * height * 3);
  const third = Math.floor(width / 3);
  const swatches = [[230, 30, 40], [20, 70, 220], [40, 200, 60], [250, 220, 30], [200, 40, 200], [20, 200, 210]];
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    let r: number, g: number, b: number;
    if (y >= height - 24) {
      // Skin-tone ramp with a soft vertical falloff.
      const t = x / (width - 1), f = (height - y) / 24;
      r = 120 + 125 * t; g = 80 + 100 * t * f; b = 60 + 70 * t * f;
    } else if (x < third) {
      r = 255 * x / third; g = 255 * y / (height - 25); b = 128 + 100 * (x - y) / (third + height);
    } else if (x < 2 * third) {
      // 5-by-3 cells: odd sizes put edges at every chroma sampling phase.
      const cx = Math.floor((x - third) / 5), cy = Math.floor(y / 3);
      const s = swatches[(cx + 2 * cy) % swatches.length]!;
      [r, g, b] = (cx + cy) % 3 === 0 ? [235, 235, 235] : s as [number, number, number];
    } else {
      const lx = x - 2 * third, ly = y;
      if (ly < 30) {
        // One-pixel lines: alternating colors per column.
        r = lx % 2 ? 250 : 10; g = (lx >> 1) % 2 ? 240 : 20; b = ly % 2 ? 200 : 60;
      } else {
        const ring = tri((lx - 40) * (lx - 40) + (ly - 70) * (ly - 70), 97);
        r = ring; g = 255 - ring; b = tri(lx * 7 + ly * 3, 41);
      }
    }
    const i = (y * width + x) * 3;
    data[i] = clamp8(r); data[i + 1] = clamp8(g); data[i + 2] = clamp8(b);
  }
  return { width, height, channels: 3, data };
}

/** The photo card with an alpha channel: a horizontal ramp, a fully
 *  transparent band, and a hard-edged opaque disc. */
export function photoAlpha(width: number, height: number): Raster {
  const rgb = photo(width, height).data;
  const data = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const i = y * width + x;
    const dx = x - width * .7, dy = y - height * .5;
    let a = clamp8(255 * x / (width - 1));
    if (y < 12) a = 0;
    if (dx * dx + dy * dy < (height * .3) ** 2) a = 255;
    data.set(rgb.subarray(i * 3, i * 3 + 3), i * 4); data[i * 4 + 3] = a;
  }
  return { width, height, channels: 4, data };
}

/** Display P3 code values: the P3 primaries and secondaries (outside sRGB),
 *  in-gamut patches, a neutral ramp, and a P3 red to white ramp. */
export function p3Card(width: number, height: number): Raster {
  const patches = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [0, 255, 255], [255, 0, 255],
    [200, 60, 50], [60, 160, 90], [70, 90, 190], [230, 180, 150], [128, 128, 128], [30, 30, 30]];
  const data = new Uint8Array(width * height * 3);
  const rows = 2, cols = 6, ph = Math.floor(height * .6 / rows), pw = Math.floor(width / cols);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    let c: number[];
    if (y < rows * ph) c = patches[Math.min(cols - 1, Math.floor(x / pw)) + cols * Math.floor(y / ph)]!;
    else if (y < rows * ph + (height - rows * ph) / 2) { const v = 255 * x / (width - 1); c = [v, v, v]; }
    else { const t = x / (width - 1); c = [255, 255 * t, 255 * t]; }
    const i = (y * width + x) * 3;
    data[i] = clamp8(c[0]!); data[i + 1] = clamp8(c[1]!); data[i + 2] = clamp8(c[2]!);
  }
  return { width, height, channels: 3, data };
}

// ---------------------------------------------------------------------------
// ICC

const s15 = (v: number) => { const b = new Uint8Array(4); new DataView(b.buffer).setInt32(0, Math.round(v * 65536)); return b; };
const concat = (parts: Uint8Array[]) => { const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0)); let o = 0; for (const p of parts) { out.set(p, o); o += p.length; } return out; };
const ascii = (s: string) => new TextEncoder().encode(s);
const u32 = (v: number) => { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, v); return b; };
const u16 = (v: number) => new Uint8Array([v >> 8, v & 0xff]);

/** A minimal ICC v4 display profile with Display P3 colorants (D50-adapted,
 *  as Apple's "Display P3" profile) and the sRGB parametric curve. */
export function displayP3Icc(): Uint8Array {
  const xyz = (v: [number, number, number]) => concat([ascii('XYZ '), new Uint8Array(4), ...v.map(s15)]);
  const mluc = (text: string) => {
    const utf16 = new Uint8Array(text.length * 2);
    for (let i = 0; i < text.length; i++) { utf16[2 * i] = 0; utf16[2 * i + 1] = text.charCodeAt(i); }
    return concat([ascii('mluc'), new Uint8Array(4), u32(1), u32(12), ascii('enUS'), u32(utf16.length), u32(28), utf16]);
  };
  const para = concat([ascii('para'), new Uint8Array(4), u16(3), u16(0), ...[2.4, 1 / 1.055, .055 / 1.055, 1 / 12.92, .04045].map(s15)]);
  const tags: Array<[string, Uint8Array]> = [
    ['desc', mluc('Corpus Display P3')], ['cprt', mluc('No copyright, use freely')],
    ['wtpt', xyz([.9642029, 1, .8249054])],
    ['rXYZ', xyz([.5151215, .2411957, -.0010529])], ['gXYZ', xyz([.2919769, .6922455, .0418854])], ['bXYZ', xyz([.1571045, .0665741, .7840729])],
    ['rTRC', para], ['gTRC', para], ['bTRC', para],
  ];
  const pad4 = (b: Uint8Array) => b.length % 4 ? concat([b, new Uint8Array(4 - b.length % 4)]) : b;
  let offset = 128 + 4 + 12 * tags.length;
  const table: Uint8Array[] = [u32(tags.length)], data: Uint8Array[] = [];
  for (const [sig, body] of tags) {
    table.push(ascii(sig), u32(offset), u32(body.length));
    const padded = pad4(body); data.push(padded); offset += padded.length;
  }
  const header = new Uint8Array(128);
  header.set(u32(offset), 0);
  header.set(u32(0x04300000), 8);
  header.set(ascii('mntrRGB XYZ '), 12);
  header.set(u32(2026), 24); header.set(u16(1), 28); header.set(u16(1), 30);
  header.set(ascii('acsp'), 36);
  header.set(ascii('APPL'), 40);
  header.set(s15(.9642029), 68); header.set(s15(1), 72); header.set(s15(.8249054), 76);
  return concat([header, ...table, ...data]);
}

// ---------------------------------------------------------------------------
// PNG

function pngChunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, data.length);
  out.set(ascii(type), 4);
  out.set(data, 8);
  view.setUint32(8 + data.length, (crc32(out.subarray(4, 8 + data.length)) ^ 0xffffffff) >>> 0);
  return out;
}

export interface PngOptions {
  /** PNG color type: 0 gray, 2 RGB, 3 indexed, 4 gray and alpha, 6 RGBA. */
  colorType: 0 | 2 | 3 | 4 | 6;
  bitDepth: 8 | 16;
  /** Samples, top-down: bytes for 8-bit and palette indices, words for 16-bit. */
  samples: Uint8Array | Uint16Array;
  palette?: Uint8Array; trns?: Uint8Array; interlace?: boolean;
  /** Ancillary chunks before IDAT. */
  chunks?: Array<[string, Uint8Array]>;
}

const ADAM7 = [[0, 0, 8, 8], [4, 0, 8, 8], [0, 4, 4, 8], [2, 0, 4, 4], [0, 2, 2, 4], [1, 0, 2, 2], [0, 1, 1, 2]] as const;

/** Encode a PNG of any 8 or 16-bit color type, optionally Adam7-interlaced. */
export function encodePngWith(width: number, height: number, o: PngOptions): Uint8Array {
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[o.colorType];
  const bytes = o.bitDepth / 8;
  if (o.samples.length !== width * height * channels) throw new Error('PNG sample count mismatch');
  const header = new Uint8Array(13);
  const view = new DataView(header.buffer);
  view.setUint32(0, width); view.setUint32(4, height);
  header[8] = o.bitDepth; header[9] = o.colorType; header[12] = o.interlace ? 1 : 0;
  const rows: number[] = [];
  const emit = (xs: number[], y: number) => {
    rows.push(0);
    for (const x of xs) for (let c = 0; c < channels; c++) {
      const v = o.samples[(y * width + x) * channels + c]!;
      if (bytes === 2) rows.push(v >> 8, v & 0xff); else rows.push(v);
    }
  };
  const all = (n: number, start: number, step: number) => { const out: number[] = []; for (let i = start; i < n; i += step) out.push(i); return out; };
  if (o.interlace) {
    for (const [x0, y0, dx, dy] of ADAM7) {
      const xs = all(width, x0, dx);
      if (!xs.length) continue;
      for (const y of all(height, y0, dy)) emit(xs, y);
    }
  } else for (let y = 0; y < height; y++) emit(all(width, 0, 1), y);
  return concat([
    new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk('IHDR', header),
    ...(o.chunks ?? []).map(([t, d]) => pngChunk(t, d)),
    ...(o.palette ? [pngChunk('PLTE', o.palette)] : []),
    ...(o.trns ? [pngChunk('tRNS', o.trns)] : []),
    pngChunk('IDAT', deflateSync(new Uint8Array(rows), { level: 9 })),
    pngChunk('IEND', new Uint8Array()),
  ]);
}

/** An `iCCP` chunk body. */
export const iccpChunk = (name: string, icc: Uint8Array): [string, Uint8Array] => ['iCCP', concat([ascii(name), new Uint8Array([0, 0]), deflateSync(icc)])];

const rgbPng = (r: Raster, chunks: Array<[string, Uint8Array]> = []) =>
  encodePngWith(r.width, r.height, { colorType: r.channels === 4 ? 6 : 2, bitDepth: 8, samples: r.data, chunks });

/** 16-bit RGBA: smooth ramps whose values are not multiples of 257, so the
 *  16 to 8-bit reduction (rounding versus truncation) shows. */
export function png16(): Uint8Array {
  const width = 200, height = 120, s = new Uint16Array(width * height * 4);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const i = (y * width + x) * 4;
    s[i] = Math.round(65535 * x / (width - 1));
    s[i + 1] = Math.round(65535 * y / (height - 1));
    s[i + 2] = (x * 977 + y * 331) % 65536;
    s[i + 3] = y < 20 ? 65535 : Math.round(65535 * (0.15 + 0.85 * ((x + y) % 101) / 100));
  }
  return encodePngWith(width, height, { colorType: 6, bitDepth: 16, samples: s });
}

/** 8-bit gray and alpha, Adam7-interlaced. */
export function pngGrayAlpha(): Uint8Array {
  const width = 181, height = 103, s = new Uint8Array(width * height * 2);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const i = (y * width + x) * 2;
    s[i] = x < 60 ? Math.round(255 * x / 59) : tri(x * x + y * 5, 53);
    s[i + 1] = y < 30 ? 255 : clamp8(255 * (y - 30) / (height - 31)) | (x % 9 === 0 ? 255 : 0);
  }
  return encodePngWith(width, height, { colorType: 4, bitDepth: 8, samples: s, interlace: true });
}

/** Indexed color with a `tRNS` alpha table (as palette-quantized web art). */
export function pngIndexed(): Uint8Array {
  const width = 160, height = 100, entries = 64;
  const palette = new Uint8Array(entries * 3), trns = new Uint8Array(entries);
  for (let i = 0; i < entries; i++) {
    palette[3 * i] = (i * 37) & 255; palette[3 * i + 1] = (i * 91 + 40) & 255; palette[3 * i + 2] = 255 - ((i * 13) & 255);
    trns[i] = i < 4 ? 0 : i < 16 ? i * 16 : 255;
  }
  const s = new Uint8Array(width * height);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) s[y * width + x] = (Math.floor(x / 10) + 4 * Math.floor(y / 10) + (x === y ? 7 : 0)) % entries;
  return encodePngWith(width, height, { colorType: 3, bitDepth: 8, samples: s, palette, trns });
}

/** The P3 card as a PNG with an embedded Display P3 profile. */
export const pngP3 = () => rgbPng(p3Card(240, 135), [iccpChunk('Display P3', displayP3Icc())]);

// ---------------------------------------------------------------------------
// JPEG (baseline or spectral-selection progressive, Huffman, 8-bit)

const ZIGZAG = [0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28,
  35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63];

const LUMA_Q = [16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62,
  18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99];
const CHROMA_Q = [17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99,
  ...new Array<number>(32).fill(99)];

const DC_LUMA = { bits: [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], values: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] };
const DC_CHROMA = { bits: [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0], values: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] };
const AC_LUMA = {
  bits: [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d],
  values: [0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08,
    0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6,
    0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa],
};
const AC_CHROMA = {
  bits: [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77],
  values: [0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91,
    0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58,
    0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4,
    0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa],
};
type HuffSpec = { bits: number[]; values: number[] };

/** Canonical codes: symbol to [code, length]. */
function huffCodes(spec: HuffSpec): Map<number, [number, number]> {
  const codes = new Map<number, [number, number]>();
  let code = 0, k = 0;
  for (let len = 1; len <= 16; len++) {
    for (let i = 0; i < spec.bits[len - 1]!; i++) codes.set(spec.values[k++]!, [code++, len]);
    code <<= 1;
  }
  return codes;
}

/** cos(n·π/16) from exact-literal constants (no libm), for a reproducible FDCT. */
const COS16 = [1, .9807852804032304, .9238795325112867, .8314696123025452, .7071067811865476, .5555702330196022, .3826834323650898, .19509032201612825, 0];
function cos16(n: number): number {
  n %= 32;
  if (n > 16) n = 32 - n;
  return n > 8 ? -COS16[16 - n]! : COS16[n]!;
}
/** DCT basis T[u][x] = C(u)/2 · cos((2x + 1)uπ/16). */
const DCT = Array.from({ length: 8 }, (_, u) => Array.from({ length: 8 }, (_, x) => (u === 0 ? COS16[4]! : 1) / 2 * cos16((2 * x + 1) * u)));

class BitWriter {
  out: number[] = [];
  private acc = 0;
  private n = 0;
  write(code: number, len: number) {
    for (let i = len - 1; i >= 0; i--) {
      this.acc = (this.acc << 1) | ((code >> i) & 1);
      if (++this.n === 8) this.flushByte();
    }
  }
  private flushByte() {
    this.out.push(this.acc);
    if (this.acc === 0xff) this.out.push(0);
    this.acc = 0; this.n = 0;
  }
  /** Pad the last byte with one bits (end of a scan). */
  finish(): number[] {
    while (this.n) this.write(1, 1);
    const out = this.out; this.out = []; return out;
  }
}

const category = (v: number) => { let a = Math.abs(v), n = 0; while (a) { n++; a >>= 1; } return n; };
const valueBits = (v: number, n: number) => v >= 0 ? v : v + (1 << n) - 1;

export interface JpegOptions {
  quality: number;
  /** Luma sampling factors [h, v] over 1×1 chroma ([1, 1] 4:4:4, [2, 1] 4:2:2, [2, 2] 4:2:0), explicit luma and
   *  chroma factors (as FFmpeg writes them: 4:4:4 as 1×2 everywhere, 4:2:2 as 2×2 over 1×2), or 'gray'. */
  sampling: [1, 1] | [2, 1] | [2, 2] | { luma: [number, number]; chroma: [number, number] } | 'gray';
  progressive?: boolean;
  icc?: Uint8Array;
  /** EXIF orientation (1 to 8). */
  orientation?: number;
}

/** Encode an RGB raster (or its luma, for gray) as a JFIF JPEG. */
export function encodeJpeg(image: Raster, o: JpegOptions): Uint8Array {
  if (image.channels !== 3) throw new Error('encodeJpeg needs RGB');
  const { width, height } = image;
  const gray = o.sampling === 'gray';
  const [[hMax, vMax], [hC, vC]] = gray ? [[1, 1], [1, 1]] : Array.isArray(o.sampling) ? [o.sampling, [1, 1]]
    : [(o.sampling as { luma: [number, number] }).luma, (o.sampling as { chroma: [number, number] }).chroma];
  const comps = gray ? [{ id: 1, h: 1, v: 1, q: 0, dc: 0, ac: 0 }]
    : [{ id: 1, h: hMax!, v: vMax!, q: 0, dc: 0, ac: 0 }, { id: 2, h: hC!, v: vC!, q: 1, dc: 1, ac: 1 }, { id: 3, h: hC!, v: vC!, q: 1, dc: 1, ac: 1 }];
  const scale = o.quality < 50 ? 5000 / o.quality : 200 - 2 * o.quality;
  const qt = [LUMA_Q, CHROMA_Q].map(base => base.map(b => Math.min(255, Math.max(1, Math.floor((b * scale + 50) / 100)))));
  const mcuW = 8 * hMax, mcuH = 8 * vMax;
  const mcusX = Math.ceil(width / mcuW), mcusY = Math.ceil(height / mcuH);
  const pw = mcusX * mcuW, ph = mcusY * mcuH;
  // Full-resolution planes, edge-replicated to the MCU grid.
  const planes = (gray ? [0] : [0, 1, 2]).map(() => new Float64Array(pw * ph));
  for (let y = 0; y < ph; y++) for (let x = 0; x < pw; x++) {
    const i = (Math.min(y, height - 1) * width + Math.min(x, width - 1)) * 3;
    const r = image.data[i]!, g = image.data[i + 1]!, b = image.data[i + 2]!;
    const j = y * pw + x;
    planes[0]![j] = .299 * r + .587 * g + .114 * b;
    if (!gray) {
      planes[1]![j] = -.168735892 * r - .331264108 * g + .5 * b + 128;
      planes[2]![j] = .5 * r - .418687589 * g - .081312411 * b + 128;
    }
  }
  // Quantized coefficients per component, in its block grid.
  const blocks = comps.map((c, ci) => {
    const fx = hMax / c.h, fy = vMax / c.v;
    const bw = mcusX * c.h, bh = mcusY * c.v;
    const out: Int32Array[] = [];
    for (let by = 0; by < bh; by++) for (let bx = 0; bx < bw; bx++) {
      const px = new Float64Array(64);
      for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) {
        let sum = 0;
        for (let sy = 0; sy < fy; sy++) for (let sx = 0; sx < fx; sx++) sum += planes[ci]![((by * 8 + y) * fy + sy) * pw + (bx * 8 + x) * fx + sx]!;
        px[y * 8 + x] = sum / (fx * fy) - 128;
      }
      const coef = new Int32Array(64);
      for (let v = 0; v < 8; v++) for (let u = 0; u < 8; u++) {
        let s = 0;
        for (let y = 0; y < 8; y++) { let row = 0; for (let x = 0; x < 8; x++) row += DCT[u]![x]! * px[y * 8 + x]!; s += DCT[v]![y]! * row; }
        coef[v * 8 + u] = Math.round(s / qt[c.q]![v * 8 + u]!);
      }
      out.push(coef);
    }
    return { bw, bh, cw: Math.ceil(Math.ceil(width * c.h / hMax) / 8), ch: Math.ceil(Math.ceil(height * c.v / vMax) / 8), out };
  });
  const dcCodes = [huffCodes(DC_LUMA), huffCodes(DC_CHROMA)], acCodes = [huffCodes(AC_LUMA), huffCodes(AC_CHROMA)];
  const put = (w: BitWriter, table: Map<number, [number, number]>, symbol: number) => { const c = table.get(symbol)!; w.write(c[0], c[1]); };
  const encodeDc = (w: BitWriter, ci: number, coef: Int32Array, pred: number[]) => {
    const diff = coef[0]! - pred[ci]!; pred[ci] = coef[0]!;
    const n = category(diff);
    put(w, dcCodes[comps[ci]!.dc]!, n); if (n) w.write(valueBits(diff, n), n);
  };
  const encodeAc = (w: BitWriter, ci: number, coef: Int32Array, ss: number, se: number) => {
    const table = acCodes[comps[ci]!.ac]!;
    let run = 0;
    for (let k = ss; k <= se; k++) {
      const v = coef[ZIGZAG[k]!]!;
      if (!v) { run++; continue; }
      while (run > 15) { put(w, table, 0xf0); run -= 16; }
      const n = category(v);
      put(w, table, (run << 4) | n); w.write(valueBits(v, n), n);
      run = 0;
    }
    if (run) put(w, table, 0x00);
  };
  /** Blocks of one MCU, in scan order. */
  const mcuBlocks = (mx: number, my: number) => comps.flatMap((c, ci) => {
    const list: Array<[number, Int32Array]> = [];
    for (let y = 0; y < c.v; y++) for (let x = 0; x < c.h; x++) list.push([ci, blocks[ci]!.out[(my * c.v + y) * blocks[ci]!.bw + mx * c.h + x]!]);
    return list;
  });
  const segment = (marker: number, body: Uint8Array | number[]) => [0xff, marker, (body.length + 2) >> 8, (body.length + 2) & 0xff, ...body];
  const sos = (ids: number[], ss: number, se: number) => segment(0xda, [ids.length, ...ids.flatMap(ci => [comps[ci]!.id, (comps[ci]!.dc << 4) | comps[ci]!.ac]), ss, se, 0]);
  const bytes: number[] = [0xff, 0xd8];
  bytes.push(...segment(0xe0, [...ascii('JFIF\0'), 1, 1, 0, 0, 1, 0, 1, 0, 0]));
  if (o.orientation) {
    // Exif, big-endian TIFF, IFD0 with one entry: Orientation (SHORT).
    const tiff = [0x4d, 0x4d, 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, o.orientation, 0, 0, 0, 0, 0, 0];
    bytes.push(...segment(0xe1, [...ascii('Exif\0\0'), ...tiff]));
  }
  if (o.icc) bytes.push(...segment(0xe2, [...ascii('ICC_PROFILE\0'), 1, 1, ...o.icc]));
  qt.slice(0, gray ? 1 : 2).forEach((t, id) => bytes.push(...segment(0xdb, [id, ...ZIGZAG.map(z => t[z]!)])));
  bytes.push(...segment(o.progressive ? 0xc2 : 0xc0, [8, height >> 8, height & 0xff, width >> 8, width & 0xff, comps.length,
    ...comps.flatMap(c => [c.id, (c.h << 4) | c.v, c.q])]));
  const dht: number[] = [];
  for (const [cls, id, spec] of [[0, 0, DC_LUMA], [1, 0, AC_LUMA], [0, 1, DC_CHROMA], [1, 1, AC_CHROMA]] as Array<[number, number, HuffSpec]>) {
    if (gray && id) continue;
    dht.push((cls << 4) | id, ...spec.bits, ...spec.values);
  }
  bytes.push(...segment(0xc4, dht));
  const all = comps.map((_, ci) => ci);
  if (!o.progressive) {
    const w = new BitWriter(), pred = comps.map(() => 0);
    if (gray) for (let by = 0; by < blocks[0]!.ch; by++) for (let bx = 0; bx < blocks[0]!.cw; bx++) {
      const coef = blocks[0]!.out[by * blocks[0]!.bw + bx]!; encodeDc(w, 0, coef, pred); encodeAc(w, 0, coef, 1, 63);
    } else for (let my = 0; my < mcusY; my++) for (let mx = 0; mx < mcusX; mx++) for (const [ci, coef] of mcuBlocks(mx, my)) {
      encodeDc(w, ci, coef, pred); encodeAc(w, ci, coef, 1, 63);
    }
    bytes.push(...sos(all, 0, 63), ...w.finish());
  } else {
    // DC first (interleaved), then two AC bands per component (non-interleaved,
    // over each component's own block grid).
    const w = new BitWriter(), pred = comps.map(() => 0);
    if (gray) for (let by = 0; by < blocks[0]!.ch; by++) for (let bx = 0; bx < blocks[0]!.cw; bx++) encodeDc(w, 0, blocks[0]!.out[by * blocks[0]!.bw + bx]!, pred);
    else for (let my = 0; my < mcusY; my++) for (let mx = 0; mx < mcusX; mx++) for (const [ci, coef] of mcuBlocks(mx, my)) encodeDc(w, ci, coef, pred);
    bytes.push(...sos(all, 0, 0), ...w.finish());
    for (const [ss, se] of [[1, 5], [6, 63]] as const) for (const ci of all) {
      const b = blocks[ci]!;
      for (let by = 0; by < b.ch; by++) for (let bx = 0; bx < b.cw; bx++) encodeAc(w, ci, b.out[by * b.bw + bx]!, ss, se);
      bytes.push(...sos([ci], ss, se), ...w.finish());
    }
  }
  bytes.push(0xff, 0xd9);
  return new Uint8Array(bytes);
}

// ---------------------------------------------------------------------------
// TIFF

/** A baseline TIFF: 8-bit RGB, chunky, uncompressed, one strip, no ICC
 *  profile and no orientation tag (the TIFF class pm-render draws). */
export function encodeTiff(image: Raster): Uint8Array {
  if (image.channels !== 3) throw new Error('encodeTiff writes RGB');
  const { width, height, data } = image;
  // Header, IFD (count, 11 entries, next offset), BitsPerSample values, pixels.
  const entries = 11, ifdAt = 8, bitsAt = ifdAt + 2 + entries * 12 + 4, pixelsAt = bitsAt + 6;
  const out = new Uint8Array(pixelsAt + data.length), view = new DataView(out.buffer);
  out.set([0x49, 0x49, 42, 0]);
  view.setUint32(4, ifdAt, true);
  view.setUint16(ifdAt, entries, true);
  const tags: Array<[number, 3 | 4, number, number]> = [
    [256, 4, 1, width], [257, 4, 1, height], [258, 3, 3, bitsAt], [259, 3, 1, 1], [262, 3, 1, 2],
    [273, 4, 1, pixelsAt], [277, 3, 1, 3], [278, 4, 1, height], [279, 4, 1, data.length], [284, 3, 1, 1], [296, 3, 1, 1],
  ];
  tags.forEach(([tag, type, count, value], i) => {
    const at = ifdAt + 2 + i * 12;
    view.setUint16(at, tag, true); view.setUint16(at + 2, type, true); view.setUint32(at + 4, count, true);
    if (type === 3 && count === 1) view.setUint16(at + 8, value, true); else view.setUint32(at + 8, value, true);
  });
  for (let i = 0; i < 3; i++) view.setUint16(bitsAt + 2 * i, 8, true);
  out.set(data, pixelsAt);
  return out;
}

// ---------------------------------------------------------------------------
// System encoders

function viaTool(source: Uint8Array, extension: string, command: (input: string, output: string) => [string, string[]]): Uint8Array {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'pm-corpus-still-'));
  try {
    const input = path.join(dir, 'source.png'), output = path.join(dir, `out.${extension}`);
    writeFileSync(input, source);
    const [binary, args] = command(input, output);
    try { execFileSync(binary, args, { stdio: ['ignore', 'ignore', 'pipe'] }); } catch (e) {
      throw new Error(`${binary} failed (${(e as Error).message}); the corpus needs ${binary === 'cwebp' ? 'libwebp (brew install webp)' : 'macOS sips'} to derive ${extension} stills`);
    }
    return new Uint8Array(readFileSync(output));
  } finally { rmSync(dir, { recursive: true, force: true }); }
}

const cwebp = (source: Uint8Array, args: string[]) => viaTool(source, 'webp', (i, out) => ['cwebp', ['-quiet', '-metadata', 'none', ...args, i, '-o', out]]);
const sips = (source: Uint8Array, format: 'heic' | 'avif') => viaTool(source, format, (i, out) => ['/usr/bin/sips', ['-s', 'format', format, i, '--out', out]]);

/** Generated stills by file name (see media.ts GENERATED_MEDIA). */
export const GENERATED_STILLS: Record<string, () => Uint8Array> = {
  'corpus-photo-420.jpg': () => encodeJpeg(photo(251, 147), { quality: 85, sampling: [2, 2] }),
  'corpus-photo-422.jpg': () => encodeJpeg(photo(250, 150), { quality: 90, sampling: [2, 1] }),
  'corpus-photo-444.jpg': () => encodeJpeg(photo(250, 150), { quality: 75, sampling: [1, 1] }),
  'corpus-photo-444-tall-mcu.jpg': () => encodeJpeg(photo(251, 147), { quality: 85, sampling: { luma: [1, 2], chroma: [1, 2] } }),
  'corpus-photo-422-tall-mcu.jpg': () => encodeJpeg(photo(251, 147), { quality: 85, sampling: { luma: [2, 2], chroma: [1, 2] } }),
  'corpus-photo-progressive.jpg': () => encodeJpeg(photo(251, 147), { quality: 85, sampling: [2, 2], progressive: true }),
  'corpus-photo-gray.jpg': () => encodeJpeg(photo(250, 150), { quality: 85, sampling: 'gray' }),
  'corpus-photo-orientation-6.jpg': () => encodeJpeg(photo(250, 150), { quality: 85, sampling: [2, 2], orientation: 6 }),
  'corpus-p3.jpg': () => encodeJpeg(p3Card(240, 135), { quality: 95, sampling: [1, 1], icc: displayP3Icc() }),
  'corpus-p3.png': pngP3,
  'corpus-16bit.png': png16,
  'corpus-gray-alpha.png': pngGrayAlpha,
  'corpus-indexed.png': pngIndexed,
  'corpus-photo-lossy.webp': () => cwebp(rgbPng(photo(250, 150)), ['-q', '80']),
  'corpus-photo-lossless.webp': () => cwebp(rgbPng(photoAlpha(250, 150)), ['-lossless', '-exact']),
  'corpus-photo-lossless-opaque.webp': () => cwebp(rgbPng(photo(250, 150)), ['-lossless']),
  'corpus-photo-alpha.webp': () => cwebp(rgbPng(photoAlpha(250, 150)), ['-q', '80', '-exact']),
  'corpus-photo.avif': () => sips(rgbPng(photo(250, 150)), 'avif'),
  'corpus-photo.heic': () => sips(rgbPng(photo(250, 150)), 'heic'),
  'corpus-p3.heic': () => sips(pngP3(), 'heic'),
  'corpus-photo.tiff': () => encodeTiff(photo(250, 150)),
};
