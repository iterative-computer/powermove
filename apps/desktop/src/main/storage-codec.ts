import { gzip, gunzipSync } from 'node:zlib';
import { promisify } from 'node:util';

const compress = promisify(gzip);
export const STORAGE_COMPRESSION_THRESHOLD = 32 * 1024;
const MARKER = '__powermoveDiskValue';

/** A JSON envelope keeps the existing filenames and atomic write protocol.
 * Compression runs on Node's worker pool; renderer/IPC values stay ordinary JSON. */
export async function encodeStorageJSON(serialized: string): Promise<string> {
  const bytes = Buffer.byteLength(serialized);
  if (bytes < STORAGE_COMPRESSION_THRESHOLD) return serialized;
  const compressed = await compress(serialized);
  const envelope = JSON.stringify({ [MARKER]: 1, encoding: 'gzip', bytes, data: compressed.toString('base64') });
  return Buffer.byteLength(envelope) < bytes * .9 ? envelope : serialized;
}

export function decodeStorageJSON(serialized: string, limit: number): { value: unknown; compressed: boolean; bytes: number } {
  let value: unknown = JSON.parse(serialized);
  if (value && typeof value === 'object' && Object.hasOwn(value, MARKER)) {
    const envelope = value as { __powermoveDiskValue: unknown; encoding: unknown; bytes: number; data: string };
    if (envelope.__powermoveDiskValue !== 1 || envelope.encoding !== 'gzip' || typeof envelope.data !== 'string'
      || !Number.isSafeInteger(envelope.bytes) || envelope.bytes < 2 || envelope.bytes > limit) throw new Error('Invalid compressed recovery record.');
    const bytes = gunzipSync(Buffer.from(envelope.data, 'base64'), { maxOutputLength: envelope.bytes });
    if (bytes.length !== envelope.bytes) throw new Error('The compressed recovery record is incomplete.');
    value = JSON.parse(bytes.toString('utf8'));
    return { value, compressed: true, bytes: envelope.bytes };
  }
  return { value, compressed: false, bytes: Buffer.byteLength(serialized) };
}
