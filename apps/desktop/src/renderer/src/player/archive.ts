const crcTable = Uint32Array.from({ length: 256 }, (_, value) => {
  for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ (0xedb88320 & -(value & 1));
  return value >>> 0;
});

/** Stored ZIP entries: assets are already compressed, and no runtime dependency is needed. */
export function zipFiles(files: Map<string, Uint8Array>): Uint8Array {
  const encoder = new TextEncoder();
  const chunks: Uint8Array[] = [], directory: Uint8Array[] = [];
  let offset = 0, directorySize = 0;
  const crc32 = (data: Uint8Array) => {
    let crc = 0xffffffff;
    for (const byte of data) crc = (crc >>> 8) ^ crcTable[(crc ^ byte) & 255]!;
    return (crc ^ 0xffffffff) >>> 0;
  };
  if (files.size > 65535) throw new Error('Too many files to export');
  for (const [name, data] of files) {
    if (!/^[\w./-]+$/.test(name) || name.startsWith('/') || name.split('/').includes('..')) throw new Error('Invalid export path');
    const path = encoder.encode(name), crc = crc32(data);
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
    if (offset > 256 * 1024 * 1024) throw new Error('Web exports are limited to 256 MB');
  }
  const end = new Uint8Array(22), e = new DataView(end.buffer);
  e.setUint32(0, 0x06054b50, true); e.setUint16(8, files.size, true); e.setUint16(10, files.size, true);
  e.setUint32(12, directorySize, true); e.setUint32(16, offset, true);
  if (offset + directorySize + end.length > 256 * 1024 * 1024) throw new Error('Web exports are limited to 256 MB');
  const output = new Uint8Array(offset + directorySize + end.length);
  let position = 0;
  for (const chunk of [...chunks, ...directory, end]) { output.set(chunk, position); position += chunk.length; }
  return output;
}
