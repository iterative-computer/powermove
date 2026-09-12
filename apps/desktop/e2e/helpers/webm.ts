const MASTER_IDS = new Set([
  0x1a45dfa3, // EBML
  0x18538067, // Segment
  0x1549a966, // Info
  0x1654ae6b, // Tracks
  0xae, // TrackEntry
  0xe0, // Video
  0xe1, // Audio
  0x1f43b675 // Cluster
]);

type Element = { id: number; dataStart: number; dataEnd: number };

function vintLength(first: number): number {
  for (let length = 1; length <= 8; length++) {
    if (first & (1 << (8 - length))) return length;
  }
  throw new Error('Invalid EBML vint');
}

function readId(bytes: Uint8Array, offset: number): { value: number; length: number } {
  const length = vintLength(bytes[offset] ?? 0);
  if (offset + length > bytes.length || length > 4) throw new Error('Invalid EBML id');
  let value = 0;
  for (let index = 0; index < length; index++) value = value * 256 + bytes[offset + index]!;
  return { value, length };
}

function readSize(bytes: Uint8Array, offset: number): { value: number; length: number } {
  const first = bytes[offset] ?? 0;
  const length = vintLength(first);
  if (offset + length > bytes.length) throw new Error('Invalid EBML size');
  let value = first & ((1 << (8 - length)) - 1);
  let unknown = value === ((1 << (8 - length)) - 1);
  for (let index = 1; index < length; index++) {
    const byte = bytes[offset + index]!;
    value = value * 256 + byte;
    unknown = unknown && byte === 0xff;
  }
  return { value: unknown ? Number.POSITIVE_INFINITY : value, length };
}

function children(bytes: Uint8Array, start: number, end: number): Element[] {
  const found: Element[] = [];
  let offset = start;
  while (offset < end) {
    const id = readId(bytes, offset);
    const size = readSize(bytes, offset + id.length);
    const dataStart = offset + id.length + size.length;
    const dataEnd = Number.isFinite(size.value) ? dataStart + size.value : end;
    if (dataEnd > end || dataEnd < dataStart) throw new Error('EBML element exceeds its parent');
    found.push({ id: id.value, dataStart, dataEnd });
    offset = dataEnd;
  }
  return found;
}

export type WebMInspection = {
  hasTracks: boolean;
  videoCodecs: string[];
  audioCodecs: string[];
  simpleBlocks: number;
};

export function inspectWebM(bytes: Uint8Array): WebMInspection {
  const decoder = new TextDecoder();
  const result: WebMInspection = {
    hasTracks: false,
    videoCodecs: [],
    audioCodecs: [],
    simpleBlocks: 0
  };

  const walk = (start: number, end: number, trackType?: number): void => {
    for (const element of children(bytes, start, end)) {
      if (element.id === 0x1654ae6b) result.hasTracks = true;
      if (element.id === 0xa3) result.simpleBlocks++;

      let nextTrackType = trackType;
      if (element.id === 0xae) {
        const entry = children(bytes, element.dataStart, element.dataEnd);
        const type = entry.find(child => child.id === 0x83);
        if (type) {
          nextTrackType = 0;
          for (let offset = type.dataStart; offset < type.dataEnd; offset++) {
            nextTrackType = nextTrackType * 256 + bytes[offset]!;
          }
        }
      }
      if (element.id === 0x86) {
        const codec = decoder.decode(bytes.subarray(element.dataStart, element.dataEnd));
        if (trackType === 1) result.videoCodecs.push(codec);
        if (trackType === 2) result.audioCodecs.push(codec);
      }
      if (MASTER_IDS.has(element.id)) walk(element.dataStart, element.dataEnd, nextTrackType);
    }
  };

  walk(0, bytes.length);
  return result;
}
