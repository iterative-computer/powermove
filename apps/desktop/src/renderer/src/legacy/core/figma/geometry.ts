import { parseSvgPathData } from '../svg-import';

/** Figma's geometry blobs contain byte opcodes and little-endian float points. */
export function readGeometry(bytes: Uint8Array) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let at = 0, commands = 0;
  const parts: string[] = [];
  while (at < bytes.length) {
    if (++commands > 100000) throw new Error('Vector geometry is too large');
    const opcode = bytes[at++]!;
    const command = ({ 0: 'Z', 1: 'M', 2: 'L', 3: 'Q', 4: 'C' } as Record<number, string>)[opcode];
    const count = ({ 0: 0, 1: 2, 2: 2, 3: 4, 4: 6 } as Record<number, number>)[opcode];
    if (!command || count == null || at + count * 4 > bytes.length) throw new Error('Unsupported vector geometry');
    parts.push(command);
    for (let i = 0; i < count; i++) {
      const value = view.getFloat32(at, true); at += 4;
      if (!Number.isFinite(value) || Math.abs(value) > 1e7) throw new Error('Invalid vector coordinates');
      parts.push(String(value));
    }
  }
  return parseSvgPathData(parts.join(' '));
}
