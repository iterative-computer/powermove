/** One binary allocation per upload, preserving byte offsets of typed-array views. */
export class StudioBinaryWriter {
  private parts: Uint8Array[] = [];
  private length = 0;
  append(data: ArrayBuffer | ArrayBufferView): { offset: number; length: number } {
    const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    const range = { offset: this.length, length: bytes.byteLength };
    this.parts.push(bytes); this.length += bytes.byteLength;
    return range;
  }
  finish(): Uint8Array {
    const bytes = new Uint8Array(this.length); let at = 0;
    for (const part of this.parts) { bytes.set(part, at); at += part.byteLength; }
    return bytes;
  }
}
