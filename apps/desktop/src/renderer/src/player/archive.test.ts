import { crc32 } from 'node:zlib';
import { describe, expect, it } from 'vitest';
import { zipFiles } from './archive';

describe('stored ZIP integrity', () => {
  it('preserves entry bytes, CRCs, and directory offsets across CRC block boundaries', () => {
    const source = new Uint8Array(1024 * 1024 + 31);
    let state = 0x12345678;
    for (let index = 0; index < source.length; index++) {
      state ^= state << 13; state ^= state >>> 17; state ^= state << 5;
      source[index] = state;
    }
    // Unaligned views exercise nonzero byteOffset as well as every tail length.
    const lengths = [...Array.from({ length: 33 }, (_, length) => length), 127, 128, 129, 255, 256, 257, 1023, 1024, 1025];
    const files = new Map<string, Uint8Array>(lengths.map(length => [`entry-${length}.bin`, source.subarray(3, 3 + length)] as const));
    for (const offset of [1, 7, 15]) {
      for (let tail = 0; tail < 16; tail++) {
        files.set(`offset-${offset}-tail-${tail}.bin`, source.subarray(offset, offset + 1024 + tail));
      }
    }
    files.set('large.bin', source.subarray(7));
    files.set('known.txt', new TextEncoder().encode('123456789'));
    const archive = zipFiles(files), view = new DataView(archive.buffer);
    const end = archive.length - 22;
    expect(view.getUint32(end, true)).toBe(0x06054b50);
    expect(view.getUint16(end + 8, true)).toBe(files.size);
    let local = 0, directory = view.getUint32(end + 16, true);
    const directoryStart = directory;
    for (const [name, bytes] of files) {
      const path = new TextEncoder().encode(name), checksum = crc32(bytes);
      expect(view.getUint32(local, true)).toBe(0x04034b50);
      expect(view.getUint32(local + 14, true)).toBe(checksum);
      expect(view.getUint32(local + 18, true)).toBe(bytes.length);
      expect(view.getUint16(local + 26, true)).toBe(path.length);
      expect(archive.subarray(local + 30, local + 30 + path.length)).toEqual(path);
      const dataStart = local + 30 + path.length;
      expect(Buffer.compare(archive.subarray(dataStart, dataStart + bytes.length), bytes)).toBe(0);
      expect(view.getUint32(directory, true)).toBe(0x02014b50);
      expect(view.getUint32(directory + 16, true)).toBe(checksum);
      expect(view.getUint32(directory + 24, true)).toBe(bytes.length);
      expect(view.getUint32(directory + 42, true)).toBe(local);
      local = dataStart + bytes.length;
      directory += 46 + path.length;
    }
    expect(local).toBe(directoryStart);
    expect(directory).toBe(end);
    expect(view.getUint32(end + 12, true)).toBe(directory - directoryStart);
  });
});
