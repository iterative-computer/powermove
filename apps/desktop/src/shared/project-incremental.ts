import { sha256Hex, sha256HexOf } from './sha256';

/** PMV4 keeps immutable media at stable offsets and publishes a complete tail index.
 * Writers build a sibling clone and atomically replace the file; live files are
 * never appended in place. Unreachable records are removed by compaction. */
export const INCREMENTAL_MAGIC = new TextEncoder().encode('PMV4\n');
const END_MAGIC = new TextEncoder().encode('PMV4END\n');
export const PROJECT_FOOTER_BYTES = 64;
const MAX_INDEX_BYTES = 64 * 1024 * 1024;
export interface ProjectRecord { offset: number; length: number; sha256: string }
export interface IncrementalMedia extends ProjectRecord { id: string; revision: string; type: string }
/** A linked asset (section 21): a record without bytes. The original stays
 *  where it is; `hash` (BLAKE3) is how it is found again if it moves. */
export interface LinkedRecord { id: string; hash: string; size: number; path: string; type?: string }
export interface ProjectIndex { version: 4; document: ProjectRecord; media: IncrementalMedia[]; linked?: LinkedRecord[] }
/** `stored`: the BLAKE3 hash of a host media store object main copies from. */
export interface SaveMedia { id: string; revision: string; type: string; length: number; stored?: string }
export const isIncrementalProject = (bytes: Uint8Array): boolean => INCREMENTAL_MAGIC.every((byte, index) => bytes[index] === byte);
const HASH = /^[a-f0-9]{64}$/;

export function projectFooter(offset: number, length: number, digest: string): Uint8Array {
  if (!Number.isSafeInteger(offset) || offset < 5 || !Number.isSafeInteger(length) || length < 2 || length > MAX_INDEX_BYTES || !HASH.test(digest)) throw new Error('Invalid project index');
  const footer = new Uint8Array(PROJECT_FOOTER_BYTES), view = new DataView(footer.buffer);
  footer.set(END_MAGIC); view.setFloat64(8, offset, true); view.setUint32(16, length, true);
  for (let i = 0; i < 32; i++) footer[32 + i] = parseInt(digest.slice(i * 2, i * 2 + 2), 16);
  return footer;
}
export function readProjectFooter(footer: Uint8Array, size: number): ProjectRecord {
  if (footer.length !== PROJECT_FOOTER_BYTES || !END_MAGIC.every((b, i) => footer[i] === b)) throw new Error('The project commit is truncated or invalid.');
  const view = new DataView(footer.buffer, footer.byteOffset, footer.byteLength);
  const offset = view.getFloat64(8, true), length = view.getUint32(16, true);
  if (!Number.isSafeInteger(size) || !Number.isSafeInteger(offset) || offset < 5 || length < 2 || length > MAX_INDEX_BYTES || offset + length !== size - PROJECT_FOOTER_BYTES) throw new Error('The project index is invalid.');
  return { offset, length, sha256: Array.from(footer.subarray(32), b => b.toString(16).padStart(2, '0')).join('') };
}
export function validateProjectIndex(value: any, indexOffset: number): ProjectIndex {
  const validRange = (r: any) => r && Number.isSafeInteger(r.offset) && r.offset >= 5 && Number.isSafeInteger(r.length) && r.length >= 0
    && r.offset <= indexOffset && r.length <= indexOffset - r.offset && typeof r.sha256 === 'string' && HASH.test(r.sha256);
  if (value?.version !== 4 || !validRange(value.document) || value.document.length < 2 || !Array.isArray(value.media) || value.media.length > 100_000) throw new Error('The project index is invalid.');
  const ids = new Set<string>();
  for (const item of value.media) {
    if (!validRange(item) || typeof item.id !== 'string' || !item.id || item.id.length > 1000 || ids.has(item.id)
      || typeof item.revision !== 'string' || !item.revision || item.revision.length > 200 || typeof item.type !== 'string' || item.type.length > 1000) throw new Error('The project media index is invalid.');
    ids.add(item.id);
  }
  if (value.linked !== undefined) validateLinkedRecords(value.linked);
  const ranges = [value.document, ...value.media].filter(r => r.length).sort((a, b) => a.offset - b.offset);
  for (let i = 1; i < ranges.length; i++) if (ranges[i]!.offset < ranges[i - 1]!.offset + ranges[i - 1]!.length) throw new Error('The project records overlap.');
  return value;
}
const BLAKE3 = /^[a-f0-9]{64}$/;
export function validateLinkedRecords(value: unknown): asserts value is LinkedRecord[] {
  if (!Array.isArray(value) || value.length > 100_000) throw new Error('The project link index is invalid.');
  const ids = new Set<string>();
  for (const item of value) {
    if (!item || typeof item.id !== 'string' || !item.id || item.id.length > 1000 || ids.has(item.id) || typeof item.hash !== 'string' || !BLAKE3.test(item.hash)
      || !Number.isSafeInteger(item.size) || item.size < 0 || typeof item.path !== 'string' || !item.path || item.path.length > 4096 || item.path.includes('\0')
      || (item.type !== undefined && (typeof item.type !== 'string' || item.type.length > 1000))) throw new Error('The project link index is invalid.');
    ids.add(item.id);
  }
}
export async function readIncrementalIndex(size: number, read: (offset: number, length: number) => Promise<Uint8Array>, hash = sha256HexOf): Promise<ProjectIndex> {
  if (size < 5 + PROJECT_FOOTER_BYTES) throw new Error('The project commit is truncated.');
  const tail = readProjectFooter(await read(size - PROJECT_FOOTER_BYTES, PROJECT_FOOTER_BYTES), size);
  const bytes = await read(tail.offset, tail.length);
  if (await hash(bytes) !== tail.sha256) throw new Error('The project index checksum does not match.');
  return validateProjectIndex(JSON.parse(new TextDecoder().decode(bytes)), tail.offset);
}
export function decodeIncrementalProject(bytes: Uint8Array): { document: any; media: Array<IncrementalMedia & { data: Uint8Array }>; binary: true } {
  const tail = readProjectFooter(bytes.subarray(bytes.length - PROJECT_FOOTER_BYTES), bytes.length);
  const raw = bytes.subarray(tail.offset, tail.offset + tail.length);
  if (sha256Hex(raw) !== tail.sha256) throw new Error('The project index checksum does not match.');
  const index = validateProjectIndex(JSON.parse(new TextDecoder().decode(raw)), tail.offset);
  const content = (r: ProjectRecord) => {
    const data = bytes.subarray(r.offset, r.offset + r.length);
    if (sha256Hex(data) !== r.sha256) throw new Error('The project content checksum does not match.');
    return data;
  };
  return { document: JSON.parse(new TextDecoder().decode(content(index.document))), media: index.media.map(r => ({ ...r, data: content(r) })), binary: true };
}
