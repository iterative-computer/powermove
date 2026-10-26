import { ByteBuffer, decodeBinarySchema } from '@open-pencil/kiwi/schema-runtime';
import type { FigmaMessage, NodeChange } from '@open-pencil/kiwi/fig/codec';
import { Decompress } from 'fzstd';
import { readFigmaMessage } from './message';

export type FigmaSelection = { nodes: NodeChange[]; blobs: Uint8Array[] };
export const MAX_CLIPBOARD_CHARS = 32 * 1024 * 1024;
const MAX_EXPANDED_BYTES = 64 * 1024 * 1024;

class SchemaBuffer extends ByteBuffer {
  override readByte(): number {
    if (this.offset >= this.length) throw new Error('Incomplete Figma schema.');
    return super.readByte();
  }
  override readVarUint(): number {
    let value = 0;
    for (let shift = 0; shift <= 28; shift += 7) {
      const byte = this.readByte(); value |= (byte & 127) << shift;
      if (!(byte & 128)) return value >>> 0;
    }
    throw new Error('Invalid Figma schema integer.');
  }
  override readString(): string {
    const end = this.bytes.indexOf(0, this.offset);
    if (end < 0) throw new Error('Incomplete Figma schema text.');
    const value = new TextDecoder().decode(this.bytes.subarray(this.offset, end)); this.offset = end + 1; return value;
  }
}

/** Read only the encoded data. Never insert clipboard HTML into the document. */
export async function decodeFigmaClipboard(html: string): Promise<FigmaSelection> {
  if (html.length > MAX_CLIPBOARD_CHARS) throw new Error('This Figma selection is too large. Copy fewer layers and try again.');
  const encoded = /\(figma\)([\s\S]*?)\(\/figma\)/.exec(html)?.[1]?.replace(/\s/g, '');
  if (!encoded || !/^[A-Za-z0-9+/]*={0,2}$/.test(encoded)) throw new Error('Could not read the copied Figma layers. Copy them again and retry.');
  const bytes = Uint8Array.from(atob(encoded), char => char.charCodeAt(0));
  if (bytes.length < 20 || new TextDecoder().decode(bytes.subarray(0, 8)) !== 'fig-kiwi') throw new Error('This Figma clipboard format is not supported yet.');
  const view = new DataView(bytes.buffer);
  const chunks: Uint8Array[] = [];
  let offset = 12;
  while (offset < bytes.length) {
    if (offset + 4 > bytes.length || chunks.length >= 8) throw new Error('The copied Figma data is incomplete.');
    const size = view.getUint32(offset, true); offset += 4;
    if (!size || offset + size > bytes.length) throw new Error('The copied Figma data is incomplete.');
    chunks.push(bytes.subarray(offset, offset + size)); offset += size;
  }
  if (chunks.length < 2) throw new Error('The copied Figma data is incomplete.');
  const schema = decodeBinarySchema(new SchemaBuffer(await expand(chunks[0]!, 4 * 1024 * 1024)));
  // The embedded schema is data, not code. Use the CSP-safe interpreter and
  // reject object-prototype names before it constructs decoded objects.
  let fields = 0;
  if (schema.definitions.length > 4096) throw new Error('This Figma schema is too large.');
  for (const definition of schema.definitions) {
    fields += definition.fields.length;
    for (const name of [definition.name, ...definition.fields.map(field => field.name)]) {
      if (['__proto__', 'prototype', 'constructor'].includes(name)) throw new Error('Invalid Figma schema.');
    }
  }
  if (fields > 65536) throw new Error('This Figma schema is too large.');
  const message = readFigmaMessage(schema, await expand(chunks[1]!, MAX_EXPANDED_BYTES)) as FigmaMessage;
  if (!Array.isArray(message.nodeChanges) || !message.nodeChanges.length) throw new Error('No layers were found in the Figma selection.');
  if (message.nodeChanges.length > 5000) throw new Error('Copy up to 5,000 Figma layers at a time.');
  return { nodes: message.nodeChanges, blobs: (message.blobs ?? []).map(blob => blob.bytes) };
}

async function expand(bytes: Uint8Array, limit: number): Promise<Uint8Array> {
  const chunks: Uint8Array[] = [];
  let total = 0;
  const add = (chunk: Uint8Array) => {
    total += chunk.length;
    if (total > limit) throw new Error('This Figma selection is too large. Copy fewer layers and try again.');
    chunks.push(chunk);
  };
  if (bytes[0] === 0x28 && bytes[1] === 0xb5 && bytes[2] === 0x2f && bytes[3] === 0xfd) {
    checkZstdFrame(bytes, limit);
    const decoder = new Decompress(add);
    for (let at = 0; at < bytes.length; at += 65536) decoder.push(bytes.subarray(at, at + 65536), at + 65536 >= bytes.length);
  } else {
    const reader = new Blob([new Uint8Array(bytes)]).stream().pipeThrough(new DecompressionStream('deflate-raw')).getReader();
    try {
      while (true) {
        const result = await reader.read();
        if (result.done) break;
        add(result.value);
      }
    } finally { await reader.cancel(); }
  }
  const output = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) { output.set(chunk, at); at += chunk.length; }
  return output;
}

function checkZstdFrame(bytes: Uint8Array, limit: number): void {
  // Bound the history window before fzstd allocates it. Walk block headers to
  // reject extra frames that could introduce a second, unchecked window.
  if (bytes.length < 6) throw new Error('Incomplete Figma compressed data.');
  const descriptor = bytes[4]!, single = !!(descriptor & 32), sizeFlag = descriptor >> 6;
  let at = single ? 5 : 6;
  const windowByte = bytes[5]!;
  const windowSize = single ? 0 : 2 ** (10 + (windowByte >> 3)) * (1 + (windowByte & 7) / 8);
  const dictionaryFlag = descriptor & 3;
  at += dictionaryFlag === 3 ? 4 : dictionaryFlag;
  const sizeBytes = sizeFlag ? 1 << sizeFlag : single ? 1 : 0;
  if (at + sizeBytes > bytes.length) throw new Error('Incomplete Figma compressed data.');
  let contentSize = 0;
  for (let index = 0; index < sizeBytes; index++) contentSize += bytes[at + index]! * 2 ** (8 * index);
  if (sizeFlag === 1) contentSize += 256;
  if (windowSize > limit || contentSize > limit) throw new Error('This Figma selection is too large. Copy fewer layers and try again.');
  at += sizeBytes;
  let last = false;
  while (!last) {
    if (at + 3 > bytes.length) throw new Error('Incomplete Figma compressed data.');
    const block = bytes[at]! | bytes[at + 1]! << 8 | bytes[at + 2]! << 16;
    const type = block >> 1 & 3;
    if (type === 3) throw new Error('Invalid Figma compression block.');
    last = !!(block & 1); at += 3 + (type === 1 ? 1 : block >>> 3);
  }
  if (descriptor & 4) at += 4;
  if (at !== bytes.length) throw new Error('Unsupported Figma compression frame.');
}
