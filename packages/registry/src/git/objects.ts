export type GitObjectType = 'blob' | 'tree' | 'commit';
export type GitCodecErrorCode = 'inflate_failed' | 'bad_header' | 'too_large' | 'length_mismatch';

export class GitCodecError extends Error {
  constructor(public readonly code: GitCodecErrorCode, message: string = code) {
    super(message);
    this.name = 'GitCodecError';
  }
}

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

export function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 || !/^[0-9a-f]*$/i.test(hex)) throw new GitCodecError('bad_header', 'Invalid hex');
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) bytes[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return bytes;
}

export function isSha1Hex(value: string): boolean { return /^[0-9a-f]{40}$/.test(value); }
export function isSha256Hex(value: string): boolean { return /^[0-9a-f]{64}$/.test(value); }

function concat(parts: Uint8Array[]): Uint8Array {
  const result = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) { result.set(part, offset); offset += part.length; }
  return result;
}

function objectBytes(type: GitObjectType, body: Uint8Array): Uint8Array {
  return concat([encoder.encode(`${type} ${body.length}\0`), body]);
}

async function digest(algorithm: 'SHA-1' | 'SHA-256', bytes: Uint8Array): Promise<string> {
  return bytesToHex(new Uint8Array(await crypto.subtle.digest(algorithm, bytes as BufferSource)));
}

export async function hashObject(type: GitObjectType, body: Uint8Array): Promise<string> {
  return digest('SHA-1', objectBytes(type, body));
}

export async function sha256Hex(bytes: Uint8Array): Promise<string> { return digest('SHA-256', bytes); }

async function compress(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new CompressionStream('deflate');
  const reading = collect(stream.readable);
  const writer = stream.writable.getWriter();
  await writer.write(bytes as BufferSource);
  await writer.close();
  return reading;
}

async function collect(stream: ReadableStream<Uint8Array>, max = Number.MAX_SAFE_INTEGER): Promise<Uint8Array> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > max) {
        await reader.cancel();
        throw new GitCodecError('too_large');
      }
      chunks.push(value);
    }
  } finally { reader.releaseLock(); }
  return concat(chunks);
}

export async function encodeLoose(type: GitObjectType, body: Uint8Array): Promise<Uint8Array> {
  return compress(objectBytes(type, body));
}

export async function decodeLoose(bytes: Uint8Array, maxInflatedBytes: number): Promise<{ type: GitObjectType; body: Uint8Array }> {
  let inflated: Uint8Array;
  try {
    const stream = new DecompressionStream('deflate');
    const reading = collect(stream.readable, maxInflatedBytes);
    const writer = stream.writable.getWriter();
    try { await writer.write(bytes as BufferSource); await writer.close(); }
    catch (error) { try { await reading; } catch (readError) { if (readError instanceof GitCodecError) throw readError; } throw error; }
    inflated = await reading;
  } catch (error) {
    if (error instanceof GitCodecError) throw error;
    throw new GitCodecError('inflate_failed');
  }
  const nul = inflated.indexOf(0);
  if (nul < 0) throw new GitCodecError('bad_header');
  let header: string;
  try { header = decoder.decode(inflated.subarray(0, nul)); }
  catch { throw new GitCodecError('bad_header'); }
  const match = /^(blob|tree|commit) (0|[1-9][0-9]*)$/.exec(header);
  if (!match) throw new GitCodecError('bad_header');
  const length = Number(match[2]);
  if (!Number.isSafeInteger(length)) throw new GitCodecError('bad_header');
  const body = inflated.slice(nul + 1);
  if (body.length !== length) throw new GitCodecError('length_mismatch');
  return { type: match[1] as GitObjectType, body };
}

export interface TreeEntry { mode: '100644' | '40000'; name: string; sha: string }

function treeNameBytes(entry: TreeEntry): Uint8Array {
  return encoder.encode(entry.name + (entry.mode === '40000' ? '/' : ''));
}

function compareBytes(a: Uint8Array, b: Uint8Array): number {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    if (a[i] !== b[i]) return a[i]! - b[i]!;
  }
  return a.length - b.length;
}

function validTreeEntry(entry: TreeEntry): void {
  if ((entry.mode !== '100644' && entry.mode !== '40000') || !entry.name || /[\/\0]/.test(entry.name) || !isSha1Hex(entry.sha)) {
    throw new GitCodecError('bad_header', 'Invalid tree entry');
  }
}

export function encodeTree(entries: TreeEntry[]): Uint8Array {
  const sorted = [...entries];
  for (const entry of sorted) validTreeEntry(entry);
  sorted.sort((a, b) => compareBytes(treeNameBytes(a), treeNameBytes(b)));
  const seen = new Set<string>();
  const parts: Uint8Array[] = [];
  for (const entry of sorted) {
    if (seen.has(entry.name)) throw new GitCodecError('bad_header', 'Duplicate tree entry');
    seen.add(entry.name);
    parts.push(encoder.encode(`${entry.mode} ${entry.name}\0`), hexToBytes(entry.sha));
  }
  return concat(parts);
}

export function parseTree(body: Uint8Array): TreeEntry[] {
  const entries: TreeEntry[] = [];
  const seen = new Set<string>();
  let offset = 0;
  while (offset < body.length) {
    const nul = body.indexOf(0, offset);
    if (nul < 0 || nul + 21 > body.length) throw new GitCodecError('bad_header');
    let header: string;
    try { header = decoder.decode(body.subarray(offset, nul)); }
    catch { throw new GitCodecError('bad_header'); }
    const match = /^(100644|40000) (.+)$/.exec(header);
    if (!match) throw new GitCodecError('bad_header');
    const entry: TreeEntry = { mode: match[1] as TreeEntry['mode'], name: match[2]!, sha: bytesToHex(body.subarray(nul + 1, nul + 21)) };
    validTreeEntry(entry);
    if (seen.has(entry.name) || (entries.length && compareBytes(treeNameBytes(entries[entries.length - 1]!), treeNameBytes(entry)) >= 0)) {
      throw new GitCodecError('bad_header', 'Noncanonical tree order');
    }
    seen.add(entry.name);
    entries.push(entry);
    offset = nul + 21;
  }
  return entries;
}

export interface CommitIdentity { name: string; email: string; time: number; tz: string }
export interface Commit { tree: string; parents: string[]; author: CommitIdentity; committer: CommitIdentity; message: string }

function identityText(identity: CommitIdentity): string {
  if (!identity.name || /[<>\r\n\0]/.test(identity.name) || !identity.email || /[<>\r\n\0 ]/.test(identity.email) ||
      !Number.isSafeInteger(identity.time) || !/^[+-](?:[01][0-9]|2[0-3])[0-5][0-9]$/.test(identity.tz)) {
    throw new GitCodecError('bad_header', 'Invalid commit identity');
  }
  return `${identity.name} <${identity.email}> ${identity.time} ${identity.tz}`;
}

function parseIdentity(value: string): CommitIdentity {
  const match = /^(.+) <([^<>\s]+)> (-?(?:0|[1-9][0-9]*)) ([+-][0-9]{4})$/.exec(value);
  if (!match) throw new GitCodecError('bad_header');
  const identity = { name: match[1]!, email: match[2]!, time: Number(match[3]), tz: match[4]! };
  identityText(identity);
  return identity;
}

export function encodeCommit(commit: Commit): Uint8Array {
  if (!isSha1Hex(commit.tree) || commit.parents.some((sha) => !isSha1Hex(sha))) throw new GitCodecError('bad_header');
  const headers = [`tree ${commit.tree}`, ...commit.parents.map((sha) => `parent ${sha}`), `author ${identityText(commit.author)}`, `committer ${identityText(commit.committer)}`];
  return encoder.encode(`${headers.join('\n')}\n\n${commit.message}`);
}

export function parseCommit(body: Uint8Array): Commit {
  let text: string;
  try { text = decoder.decode(body); } catch { throw new GitCodecError('bad_header'); }
  const blank = text.indexOf('\n\n');
  if (blank < 0) throw new GitCodecError('bad_header');
  const headers = text.slice(0, blank).split('\n');
  const tree = headers.shift()?.match(/^tree ([0-9a-f]{40})$/)?.[1];
  if (!tree) throw new GitCodecError('bad_header');
  const parents: string[] = [];
  while (headers[0]?.startsWith('parent ')) {
    const parent = headers.shift()!.match(/^parent ([0-9a-f]{40})$/)?.[1];
    if (!parent) throw new GitCodecError('bad_header');
    parents.push(parent);
  }
  const author = headers.shift()?.match(/^author (.+)$/)?.[1];
  const committer = headers.shift()?.match(/^committer (.+)$/)?.[1];
  if (!author || !committer || headers.length) throw new GitCodecError('bad_header');
  return { tree, parents, author: parseIdentity(author), committer: parseIdentity(committer), message: text.slice(blank + 2) };
}
