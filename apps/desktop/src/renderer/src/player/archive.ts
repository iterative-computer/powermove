const crcTable = Uint32Array.from({ length: 256 }, (_, value) => {
  for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ (0xedb88320 & -(value & 1));
  return value >>> 0;
});
// Process sixteen bytes per iteration, then an eight-byte block and byte tail.
// Large media buffers dominate stored ZIPs; 16 KiB of lookup tables shorten
// the dependency chain without changing the short-file path.
const crcSlices = Array.from({ length: 16 }, () => new Uint32Array(256));
crcSlices[0]!.set(crcTable);
for (let slice = 1; slice < crcSlices.length; slice++) {
  for (let byte = 0; byte < 256; byte++) {
    const previous = crcSlices[slice - 1]![byte]!;
    crcSlices[slice]![byte] = (previous >>> 8) ^ crcTable[previous & 255]!;
  }
}

function crc32(data: Uint8Array): number {
  let crc = 0xffffffff, offset = 0;
  const [t0, t1, t2, t3, t4, t5, t6, t7] = crcSlices as [Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array];
  if (data.length >= 16) {
    const t8 = crcSlices[8]!, t9 = crcSlices[9]!, t10 = crcSlices[10]!, t11 = crcSlices[11]!, t12 = crcSlices[12]!, t13 = crcSlices[13]!, t14 = crcSlices[14]!, t15 = crcSlices[15]!;
    for (; offset + 16 <= data.length; offset += 16) {
      const first = crc ^ (data[offset]! | data[offset + 1]! << 8 | data[offset + 2]! << 16 | data[offset + 3]! << 24);
      crc = t15[first & 255]! ^ t14[(first >>> 8) & 255]! ^ t13[(first >>> 16) & 255]! ^ t12[first >>> 24]!
        ^ t11[data[offset + 4]!]! ^ t10[data[offset + 5]!]! ^ t9[data[offset + 6]!]! ^ t8[data[offset + 7]!]!
        ^ t7[data[offset + 8]!]! ^ t6[data[offset + 9]!]! ^ t5[data[offset + 10]!]! ^ t4[data[offset + 11]!]!
        ^ t3[data[offset + 12]!]! ^ t2[data[offset + 13]!]! ^ t1[data[offset + 14]!]! ^ t0[data[offset + 15]!]!;
    }
  }
  for (; offset + 8 <= data.length; offset += 8) {
    const first = crc ^ (data[offset]! | data[offset + 1]! << 8 | data[offset + 2]! << 16 | data[offset + 3]! << 24);
    crc = t7[first & 255]! ^ t6[(first >>> 8) & 255]! ^ t5[(first >>> 16) & 255]! ^ t4[first >>> 24]!
      ^ t3[data[offset + 4]!]! ^ t2[data[offset + 5]!]! ^ t1[data[offset + 6]!]! ^ t0[data[offset + 7]!]!;
  }
  for (; offset < data.length; offset++) crc = (crc >>> 8) ^ crcTable[(crc ^ data[offset]!) & 255]!;
  return (crc ^ 0xffffffff) >>> 0;
}

/** Stored ZIP entries: assets are already compressed, and no runtime dependency is needed. */
export function zipFiles(files: Map<string, Uint8Array>): Uint8Array<ArrayBuffer> {
  const encoder = new TextEncoder();
  const chunks: Uint8Array[] = [], directory: Uint8Array[] = [];
  let offset = 0, directorySize = 0;
  // Classic ZIP uses 16-bit counts and 32-bit sizes/offsets; larger values need ZIP64.
  if (files.size >= 0xffff) throw new Error('This export requires ZIP64 (too many files).');
  for (const [name, data] of files) {
    if (!/^[\w./-]+$/.test(name) || name.startsWith('/') || name.split('/').includes('..')) throw new Error('Invalid export path');
    const path = encoder.encode(name);
    if (path.length > 0xffff) throw new Error('Export path is too long for ZIP.');
    if (data.length >= 0xffffffff || offset + 30 + path.length + data.length >= 0xffffffff) {
      throw new Error('This export requires ZIP64 (4 GB or larger).');
    }
    const crc = crc32(data);
    const header = new Uint8Array(30 + path.length), h = new DataView(header.buffer);
    h.setUint32(0, 0x04034b50, true); h.setUint16(4, 20, true); h.setUint16(6, 0x800, true);
    h.setUint16(12, 33, true); h.setUint32(14, crc, true); h.setUint32(18, data.length, true); h.setUint32(22, data.length, true);
    h.setUint16(26, path.length, true); header.set(path, 30);
    chunks.push(header, data);
    const entry = new Uint8Array(46 + path.length), e = new DataView(entry.buffer);
    e.setUint32(0, 0x02014b50, true); e.setUint16(4, 20, true); e.setUint16(6, 20, true); e.setUint16(8, 0x800, true);
    e.setUint16(14, 33, true); e.setUint32(16, crc, true); e.setUint32(20, data.length, true); e.setUint32(24, data.length, true);
    e.setUint16(28, path.length, true); e.setUint32(42, offset, true); entry.set(path, 46);
    directory.push(entry); directorySize += entry.length; offset += header.length + data.length;
  }
  const end = new Uint8Array(22), e = new DataView(end.buffer);
  e.setUint32(0, 0x06054b50, true); e.setUint16(8, files.size, true); e.setUint16(10, files.size, true);
  e.setUint32(12, directorySize, true); e.setUint32(16, offset, true);
  if (offset + directorySize + end.length >= 0xffffffff) throw new Error('This export requires ZIP64 (4 GB or larger).');
  const output = new Uint8Array(offset + directorySize + end.length);
  let position = 0;
  for (const chunk of [...chunks, ...directory, end]) { output.set(chunk, position); position += chunk.length; }
  return output;
}
