const MAGIC = new Uint8Array([0x50, 0x4d, 0x56, 0x33, 0x0a]); // PMV3\n
export interface ProjectContainerMedia {
  id: string;
  type: string;
  data: Uint8Array;
}

export interface DecodedProjectContainer {
  document: any;
  media: ProjectContainerMedia[];
  binary: boolean;
}

const bytes = (value: Uint8Array | ArrayBuffer): Uint8Array =>
  value instanceof Uint8Array ? value : new Uint8Array(value);

export function isProjectContainer(value: Uint8Array): boolean {
  return value.byteLength >= MAGIC.length && MAGIC.every((byte, index) => value[index] === byte);
}

/** PMV3 stores media as raw bytes, avoiding base64's 33% size and transient string copies. */
export function encodeProjectContainer(document: any, media: ProjectContainerMedia[]): Uint8Array {
  let offset = 0;
  const descriptors = media.map(item => {
    const descriptor = { id: item.id, type: item.type, offset, length: item.data.byteLength };
    offset += item.data.byteLength;
    return descriptor;
  });
  const header = new TextEncoder().encode(JSON.stringify({ document, media: descriptors }));
  const result = new Uint8Array(MAGIC.length + 4 + header.byteLength + offset);
  result.set(MAGIC, 0);
  new DataView(result.buffer).setUint32(MAGIC.length, header.byteLength, true);
  result.set(header, MAGIC.length + 4);
  let cursor = MAGIC.length + 4 + header.byteLength;
  for (const item of media) {
    result.set(item.data, cursor);
    cursor += item.data.byteLength;
  }
  return result;
}

export function decodeProjectContainer(input: string | Uint8Array | ArrayBuffer): DecodedProjectContainer {
  if (typeof input === 'string') return { document: JSON.parse(input), media: [], binary: false };
  const data = bytes(input);
  if (!isProjectContainer(data)) {
    return { document: JSON.parse(new TextDecoder().decode(data)), media: [], binary: false };
  }
  if (data.byteLength < MAGIC.length + 4) throw new Error('The project container is truncated.');
  const headerLength = new DataView(data.buffer, data.byteOffset, data.byteLength).getUint32(MAGIC.length, true);
  const bodyStart = MAGIC.length + 4 + headerLength;
  if (headerLength < 2 || bodyStart > data.byteLength) throw new Error('The project container header is invalid.');
  const header = JSON.parse(new TextDecoder().decode(data.subarray(MAGIC.length + 4, bodyStart)));
  if (!header || typeof header !== 'object' || !header.document || !Array.isArray(header.media)) {
    throw new Error('The project container header is invalid.');
  }
  const media: ProjectContainerMedia[] = header.media.map((item: any) => {
    if (!item || typeof item.id !== 'string' || typeof item.type !== 'string'
      || !Number.isSafeInteger(item.offset) || item.offset < 0
      || !Number.isSafeInteger(item.length) || item.length < 0
      || bodyStart + item.offset + item.length > data.byteLength) {
      throw new Error('The project container media index is invalid.');
    }
    return { id: item.id, type: item.type, data: data.slice(bodyStart + item.offset, bodyStart + item.offset + item.length) };
  });
  return { document: header.document, media, binary: true };
}
