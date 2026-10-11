/** On-disk corpus formats. Shapes are documented in docs/engine/corpus.md. */
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { encodePng } from './png';

export const FLOAT_FORMAT = 'powermove-corpus-rgba32f';

export interface FileRecord { file: string; bytes: number; sha256: string }

export interface FloatHeader {
  format: typeof FLOAT_FORMAT;
  version: 1;
  width: number;
  height: number;
  channels: 4;
  sampleType: 'f32';
  endianness: 'little';
  rowOrder: 'top-down';
  alpha: 'premultiplied';
  encoding: string;
  source: string;
  time: number;
  dataFile: string;
  dataBytes: number;
  dataSha256: string;
}

export const sha256 = (bytes: Uint8Array | string): string => createHash('sha256').update(bytes).digest('hex');

export function writeFile(root: string, relative: string, bytes: Uint8Array | string): FileRecord {
  const file = path.join(root, relative);
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, bytes);
  const length = typeof bytes === 'string' ? Buffer.byteLength(bytes) : bytes.length;
  return { file: relative, bytes: length, sha256: sha256(bytes) };
}

export function writePng(root: string, relative: string, width: number, height: number, rgba: Uint8Array, text: Record<string, string>): FileRecord {
  return writeFile(root, relative, encodePng({ width, height, channels: 4, data: rgba }, text));
}

/** Raw little-endian f32 RGBA plus a JSON header beside it (`<name>.rgba32f`, `<name>.rgba32f.json`). */
export function writeFloat(root: string, relative: string, header: Omit<FloatHeader, 'format' | 'version' | 'channels' | 'sampleType' | 'endianness' | 'dataFile' | 'dataBytes' | 'dataSha256'>, data: Uint8Array): { data: FileRecord; header: FileRecord } {
  if (data.length !== header.width * header.height * 16) throw new Error(`Float data is ${data.length} bytes, expected ${header.width * header.height * 16}`);
  if (new Uint8Array(new Uint16Array([1]).buffer)[0] !== 1) throw new Error('Float capture assumes a little-endian host');
  const record = writeFile(root, relative, data);
  const full: FloatHeader = {
    format: FLOAT_FORMAT, version: 1, channels: 4, sampleType: 'f32', endianness: 'little', ...header,
    dataFile: path.basename(relative), dataBytes: record.bytes, dataSha256: record.sha256,
  };
  return { data: record, header: writeFile(root, relative + '.json', JSON.stringify(full, null, 1) + '\n') };
}

/** Read a float capture back (tests and downstream tools). */
export function readFloat(headerPath: string): { header: FloatHeader; pixels: Float32Array } {
  const header = JSON.parse(readFileSync(headerPath, 'utf8')) as FloatHeader;
  if (header.format !== FLOAT_FORMAT || header.version !== 1) throw new Error(`${headerPath} is not a ${FLOAT_FORMAT} v1 header`);
  const bytes = readFileSync(path.join(path.dirname(headerPath), header.dataFile));
  if (bytes.length !== header.dataBytes || sha256(bytes) !== header.dataSha256) throw new Error(`${header.dataFile} does not match its header`);
  const copy = new Uint8Array(bytes.length); copy.set(bytes);
  return { header, pixels: new Float32Array(copy.buffer) };
}

/** Straight-alpha conversion of a bottom-up premultiplied RGBA8 buffer, as the exporter's `alphaFrame`. */
export function straightTopDown(px: Uint8Array, width: number, height: number): Uint8Array {
  const d = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++) {
    let si = (height - 1 - y) * width * 4, di = y * width * 4;
    for (let x = 0; x < width; x++, si += 4, di += 4) {
      const a = px[si + 3]!;
      if (a === 255) { d[di] = px[si]!; d[di + 1] = px[si + 1]!; d[di + 2] = px[si + 2]!; d[di + 3] = 255; }
      else if (a === 0) { d[di] = d[di + 1] = d[di + 2] = d[di + 3] = 0; }
      else {
        d[di] = Math.min(255, (px[si]! * 255 / a) | 0);
        d[di + 1] = Math.min(255, (px[si + 1]! * 255 / a) | 0);
        d[di + 2] = Math.min(255, (px[si + 2]! * 255 / a) | 0);
        d[di + 3] = a;
      }
    }
  }
  return d;
}

export const timeLabel = (t: number): string => `t${t.toFixed(6)}`;
