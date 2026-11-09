import { STUDIO_MAX_BINARY, STUDIO_MAX_JSON } from '../../shared/studio-native';

export type WirePacket = { json: { id: number; ok: boolean; result?: unknown; error?: string }; binary: Buffer };
/** Incremental decoder: one allocation per packet, including when pipe chunks are tiny. */
export class StudioPacketDecoder {
  private header = Buffer.alloc(12);
  private headerAt = 0;
  private body: Buffer | null = null;
  private bodyAt = 0;
  private jsonBytes = 0;

  push(chunk: Buffer): WirePacket[] {
    const packets: WirePacket[] = [];
    let offset = 0;
    while (offset < chunk.length) {
      if (!this.body) {
        const n = Math.min(12 - this.headerAt, chunk.length - offset);
        chunk.copy(this.header, this.headerAt, offset, offset + n);
        this.headerAt += n; offset += n;
        if (this.headerAt !== 12) continue;
        if (this.header.toString('ascii', 0, 4) !== 'PMS1') throw new Error('Invalid Studio packet magic');
        this.jsonBytes = this.header.readUInt32LE(4);
        const bytes = this.header.readUInt32LE(8);
        if (!this.jsonBytes || this.jsonBytes > STUDIO_MAX_JSON || bytes > STUDIO_MAX_BINARY) throw new Error('Studio packet exceeds size limit');
        this.body = Buffer.allocUnsafe(this.jsonBytes + bytes);
        this.bodyAt = 0;
      }
      const n = Math.min(this.body.length - this.bodyAt, chunk.length - offset);
      chunk.copy(this.body, this.bodyAt, offset, offset + n);
      this.bodyAt += n; offset += n;
      if (this.bodyAt !== this.body.length) continue;
      const json: WirePacket['json'] = JSON.parse(this.body.toString('utf8', 0, this.jsonBytes));
      if (!json || !Number.isSafeInteger(json.id) || typeof json.ok !== 'boolean') throw new Error('Invalid Studio response');
      packets.push({ json, binary: this.body.subarray(this.jsonBytes) });
      this.body = null; this.headerAt = 0;
    }
    return packets;
  }
}
export function studioPacket(id: number, command: object, binary: Uint8Array = new Uint8Array()): Buffer[] {
  const json = Buffer.from(JSON.stringify({ ...command, id }), 'utf8');
  if (json.length > STUDIO_MAX_JSON || binary.byteLength > STUDIO_MAX_BINARY) throw new Error('Studio request exceeds size limit');
  const header = Buffer.allocUnsafe(12);
  header.write('PMS1'); header.writeUInt32LE(json.length, 4); header.writeUInt32LE(binary.byteLength, 8);
  return [header, json, Buffer.from(binary.buffer, binary.byteOffset, binary.byteLength)];
}
