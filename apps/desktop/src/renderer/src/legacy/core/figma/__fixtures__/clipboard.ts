import { deflateRawSync, zstdCompressSync } from 'node:zlib';
import { getCompiledSchema, getSchemaBytes, initCodec, type NodeChange } from '@open-pencil/kiwi/fig/codec';

export const guid = (localID: number) => ({ sessionID: 7, localID });
export function figmaNode(localID: number, type = 'RECTANGLE', patch: Partial<NodeChange> = {}): NodeChange {
  return { guid: guid(localID), phase: 'CREATED', type, name: `Layer ${localID}`,
    size: { x: 100, y: 60 }, transform: { m00: 1, m01: 0, m02: 0, m10: 0, m11: 1, m12: 0 },
    fillPaints: [{ type: 'SOLID', color: { r: 1, g: .25, b: 0, a: 1 }, opacity: 1, visible: true }], ...patch };
}

/** Synthetic designs encoded with the actual Figma schema. No user data. */
export async function figmaHtml(nodes = [figmaNode(1)], blobs: Uint8Array[] = [], compression: 'deflate' | 'zstd' = 'deflate'): Promise<string> {
  await initCodec();
  const schema = deflateRawSync(getSchemaBytes());
  const message = getCompiledSchema().encodeMessage({ type: 'NODE_CHANGES', nodeChanges: nodes, blobs: blobs.map(bytes => ({ bytes })) });
  const payload = compression === 'zstd' ? zstdCompressSync(message) : deflateRawSync(message);
  return archiveHtml(schema, payload);
}

export function archiveHtml(schema: Uint8Array, message: Uint8Array): string {
  const header = Buffer.alloc(12); header.write('fig-kiwi'); header.writeUInt32LE(15, 8);
  const chunks = [schema, message].flatMap(bytes => { const length = Buffer.alloc(4); length.writeUInt32LE(bytes.length); return [length, bytes]; });
  return `<meta charset="utf-8"><span data-metadata="<!--(figmeta)e30=(/figmeta)-->"></span><span data-buffer="<!--(figma)${Buffer.concat([header, ...chunks]).toString('base64')}(/figma)-->"></span>`;
}

export function pathBlob(): Uint8Array {
  const bytes = new Uint8Array(28), view = new DataView(bytes.buffer);
  let offset = 0;
  for (const [opcode, x, y] of [[1, 0, 0], [2, 100, 0], [2, 0, 60]]) {
    bytes[offset++] = opcode!; view.setFloat32(offset, x!, true); offset += 4; view.setFloat32(offset, y!, true); offset += 4;
  }
  bytes[offset] = 0;
  return bytes;
}
