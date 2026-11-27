// Minimal PNG encoder and decoder for corpus textures and reference renders.
// 8-bit and 16-bit RGBA, no interlacing; enough for deterministic test images.
import { deflateSync, inflateSync } from 'node:zlib';

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(bytes: Uint8Array): number {
  let c = 0xffffffff;
  for (const b of bytes) c = CRC_TABLE[(c ^ b) & 0xff]! ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, data.length);
  for (let i = 0; i < 4; i++) out[4 + i] = type.charCodeAt(i);
  out.set(data, 8);
  view.setUint32(8 + data.length, crc32(out.subarray(4, 8 + data.length)));
  return out;
}

/** Encodes RGBA pixels (top row first). `depth` 16 expects a Uint16Array. */
export function encodePng(width: number, height: number, pixels: Uint8Array | Uint16Array, depth: 8 | 16 = 8): Uint8Array {
  const bpp = depth === 8 ? 4 : 8;
  const raw = new Uint8Array((width * bpp + 1) * height);
  for (let y = 0; y < height; y++) {
    const row = y * (width * bpp + 1);
    raw[row] = 0;
    for (let x = 0; x < width * 4; x++) {
      const v = pixels[y * width * 4 + x]!;
      if (depth === 8) raw[row + 1 + x] = v;
      else { raw[row + 1 + x * 2] = v >> 8; raw[row + 2 + x * 2] = v & 0xff; }
    }
  }
  const ihdr = new Uint8Array(13);
  const view = new DataView(ihdr.buffer);
  view.setUint32(0, width); view.setUint32(4, height);
  ihdr[8] = depth; ihdr[9] = 6; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
  const parts = [
    new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', new Uint8Array()),
  ];
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let offset = 0;
  for (const p of parts) { out.set(p, offset); offset += p.length; }
  return out;
}

/** Decodes 8 or 16-bit RGB/RGBA PNGs (filter types 0-4). 16-bit returns Uint16Array. */
export function decodePng(bytes: Uint8Array): { width: number; height: number; depth: 8 | 16; channels: number; pixels: Uint8Array | Uint16Array; chunks: string[] } {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let offset = 8, width = 0, height = 0, depth = 8, colorType = 6;
  const idat: Uint8Array[] = [], chunks: string[] = [];
  while (offset < bytes.length) {
    const length = view.getUint32(offset);
    const type = String.fromCharCode(...bytes.subarray(offset + 4, offset + 8));
    const data = bytes.subarray(offset + 8, offset + 8 + length);
    chunks.push(type);
    if (type === 'IHDR') { width = view.getUint32(offset + 8); height = view.getUint32(offset + 12); depth = data[8]!; colorType = data[9]!; if (data[12]) throw new Error('Interlaced PNG'); }
    if (type === 'IDAT') idat.push(data);
    offset += 12 + length;
  }
  const channels = colorType === 6 ? 4 : colorType === 2 ? 3 : colorType === 4 ? 2 : colorType === 0 ? 1 : 0;
  if (!channels) throw new Error(`Unsupported PNG color type ${colorType}`);
  const inflated = inflateSync(Buffer.concat(idat));
  const bpp = channels * (depth / 8), stride = width * bpp;
  const out = new Uint8Array(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = inflated[y * (stride + 1)]!;
    const src = inflated.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? out[y * stride + x - bpp]! : 0;
      const b = y > 0 ? out[(y - 1) * stride + x]! : 0;
      const c = x >= bpp && y > 0 ? out[(y - 1) * stride + x - bpp]! : 0;
      let v = src[x]!;
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c; }
      out[y * stride + x] = v & 0xff;
    }
  }
  if (depth === 8) return { width, height, depth: 8, channels, pixels: out, chunks };
  const wide = new Uint16Array(width * height * channels);
  for (let i = 0; i < wide.length; i++) wide[i] = (out[i * 2]! << 8) | out[i * 2 + 1]!;
  return { width, height, depth: 16, channels, pixels: wide, chunks };
}
