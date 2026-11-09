/** One binary allocation per upload, preserving byte offsets of typed-array views. */
export class StudioBinaryWriter {
  private parts: Uint8Array[] = [];
  private length = 0;
  get byteLength():number { return this.length; }
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
  /** Bounded IPC packets keep structured cloning out of long editor tasks.
   * The original geometry stays resident; no second scene-sized array exists. */
  *chunks(size=1024*1024):Generator<{offset:number;bytes:Uint8Array}> {
    if(!Number.isInteger(size) || size<1)throw new Error('Invalid Studio chunk size');
    let offset=0;
    for(const part of this.parts)for(let at=0;at<part.length;at+=size){
      const bytes=part.subarray(at,Math.min(part.length,at+size));
      yield {offset,bytes};offset+=bytes.length;
    }
  }
}
