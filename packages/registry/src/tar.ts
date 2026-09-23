import { REGISTRY_LIMITS, type RegistryLimits } from './limits';
import { normalizePath, type SnapshotInput } from './snapshot';

export type TarErrorCode = 'path_too_long' | 'bad_header' | 'unsupported_entry' | 'truncated' |
  'too_many_files' | 'file_too_large' | 'tree_too_large' | 'bad_path' | 'inflate_failed';
export class TarError extends Error {
  constructor(public readonly code: TarErrorCode, message: string = code) { super(message); this.name = 'TarError'; }
}

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });
const BLOCK = 512;

function compare(a: string, b: string): number {
  const x = encoder.encode(a), y = encoder.encode(b);
  for (let i = 0; i < Math.min(x.length, y.length); i++) if (x[i] !== y[i]) return x[i]! - y[i]!;
  return x.length - y.length;
}

function putAscii(block: Uint8Array, offset: number, length: number, value: string): void {
  const bytes = encoder.encode(value);
  if (bytes.length > length) throw new TarError('path_too_long');
  block.set(bytes, offset);
}

function putOctal(block: Uint8Array, offset: number, length: number, value: number): void {
  if (!Number.isSafeInteger(value) || value < 0) throw new TarError('bad_header');
  const digits = value.toString(8);
  if (digits.length > length - 1) throw new TarError('bad_header');
  putAscii(block, offset, length, digits.padStart(length - 1, '0') + '\0');
}

function splitPath(path: string): { name: string; prefix: string } {
  if (encoder.encode(path).length <= 100) return { name: path, prefix: '' };
  for (let at = path.lastIndexOf('/'); at > 0; at = path.lastIndexOf('/', at - 1)) {
    const prefix = path.slice(0, at), name = path.slice(at + 1);
    if (encoder.encode(prefix).length <= 155 && encoder.encode(name).length <= 100) return { name, prefix };
  }
  throw new TarError('path_too_long');
}

function checksum(block: Uint8Array): number {
  let sum = 0;
  for (let i = 0; i < BLOCK; i++) sum += i >= 148 && i < 156 ? 32 : block[i]!;
  return sum;
}

export function writeTar(files: SnapshotInput[]): Uint8Array {
  const ordered = files.map((file) => {
    try { return { path: normalizePath(file.path), bytes: file.bytes }; }
    catch { throw new TarError('bad_path'); }
  }).sort((a, b) => compare(a.path, b.path));
  const chunks: Uint8Array[] = [];
  let length = BLOCK * 2;
  for (const file of ordered) {
    const { name, prefix } = splitPath(file.path);
    const header = new Uint8Array(BLOCK);
    putAscii(header, 0, 100, name);
    putOctal(header, 100, 8, 0o644);
    putOctal(header, 108, 8, 0);
    putOctal(header, 116, 8, 0);
    putOctal(header, 124, 12, file.bytes.length);
    putOctal(header, 136, 12, 0);
    header[156] = 48;
    putAscii(header, 257, 6, 'ustar\0');
    putAscii(header, 263, 2, '00');
    putAscii(header, 345, 155, prefix);
    putOctal(header, 148, 8, checksum(header));
    chunks.push(header, file.bytes);
    const padding = (BLOCK - file.bytes.length % BLOCK) % BLOCK;
    if (padding) chunks.push(new Uint8Array(padding));
    length += BLOCK + file.bytes.length + padding;
  }
  const result = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) { result.set(chunk, offset); offset += chunk.length; }
  return result;
}

function field(block: Uint8Array, offset: number, length: number): string {
  const bytes = block.subarray(offset, offset + length);
  const end = bytes.indexOf(0);
  try { return decoder.decode(end < 0 ? bytes : bytes.subarray(0, end)); }
  catch { throw new TarError('bad_header'); }
}

function octal(block: Uint8Array, offset: number, length: number): number {
  const value = field(block, offset, length).trim();
  if (!/^[0-7]+$/.test(value)) throw new TarError('bad_header');
  const number = Number.parseInt(value, 8);
  if (!Number.isSafeInteger(number)) throw new TarError('bad_header');
  return number;
}

function allZero(bytes: Uint8Array): boolean { return bytes.every((byte) => byte === 0); }

export function readTar(bytes: Uint8Array, limits: RegistryLimits = REGISTRY_LIMITS): SnapshotInput[] {
  const files: SnapshotInput[] = [];
  let offset = 0, total = 0;
  while (true) {
    if (offset + BLOCK > bytes.length) throw new TarError('truncated');
    const header = bytes.subarray(offset, offset + BLOCK);
    if (allZero(header)) {
      if (offset + 2 * BLOCK > bytes.length || !allZero(bytes.subarray(offset + BLOCK, offset + 2 * BLOCK))) throw new TarError('truncated');
      if (!allZero(bytes.subarray(offset + 2 * BLOCK))) throw new TarError('bad_header');
      return files;
    }
    if (octal(header, 148, 8) !== checksum(header)) throw new TarError('bad_header', 'Invalid tar checksum');
    const magic = field(header, 257, 6);
    if (magic !== '' && magic !== 'ustar') throw new TarError('bad_header');
    const type = header[156];
    if (type !== 0 && type !== 48) throw new TarError('unsupported_entry');
    const name = field(header, 0, 100);
    const prefix = magic === 'ustar' ? field(header, 345, 155) : '';
    let path: string;
    try { path = normalizePath(prefix ? `${prefix}/${name}` : name); }
    catch { throw new TarError('bad_path'); }
    if (encoder.encode(path).length > limits.pathChars) throw new TarError('path_too_long');
    const size = octal(header, 124, 12);
    if (files.length + 1 > limits.files) throw new TarError('too_many_files');
    if (size > limits.fileBytes) throw new TarError('file_too_large');
    total += size;
    if (total > limits.treeBytes) throw new TarError('tree_too_large');
    const dataStart = offset + BLOCK;
    const next = dataStart + Math.ceil(size / BLOCK) * BLOCK;
    if (next > bytes.length) throw new TarError('truncated');
    files.push({ path, bytes: bytes.slice(dataStart, dataStart + size) });
    offset = next;
  }
}

async function collect(stream: ReadableStream<Uint8Array>, max: number): Promise<Uint8Array> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > max) { await reader.cancel(); throw new TarError('tree_too_large'); }
      chunks.push(value);
    }
  } finally { reader.releaseLock(); }
  const result = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { result.set(chunk, offset); offset += chunk.length; }
  return result;
}

export async function gzip(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new CompressionStream('gzip');
  const reading = collect(stream.readable, Number.MAX_SAFE_INTEGER);
  const writer = stream.writable.getWriter();
  await writer.write(bytes as BufferSource);
  await writer.close();
  return reading;
}

export async function gunzip(bytes: Uint8Array, maxBytes: number): Promise<Uint8Array> {
  try {
    const stream = new DecompressionStream('gzip');
    const reading = collect(stream.readable, maxBytes);
    const writer = stream.writable.getWriter();
    try { await writer.write(bytes as BufferSource); await writer.close(); }
    catch (error) { try { await reading; } catch (readError) { if (readError instanceof TarError) throw readError; } throw error; }
    return await reading;
  } catch (error) {
    if (error instanceof TarError) throw error;
    throw new TarError('inflate_failed');
  }
}

export async function writeTarGz(files: SnapshotInput[]): Promise<Uint8Array> { return gzip(writeTar(files)); }
export async function readTarGz(bytes: Uint8Array, limits: RegistryLimits = REGISTRY_LIMITS): Promise<SnapshotInput[]> {
  return readTar(await gunzip(bytes, limits.treeBytes + limits.files * BLOCK * 2 + BLOCK * 2), limits);
}
