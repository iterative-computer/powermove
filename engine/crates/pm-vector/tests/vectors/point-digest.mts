/** FNV-1a 64 over little-endian coordinate bits. JSON records NaN's numeric
 * class, not its hardware sign/payload. All other bits remain significant. */
export function pointsDigest(points: Array<[number, number]>): string {
  const coordinates = points.flat();
  const bytes = new Uint8Array(coordinates.length * 8);
  const view = new DataView(bytes.buffer);
  coordinates.forEach((value, index) => {
    if (Number.isNaN(value)) view.setBigUint64(index * 8, 0x7ff8000000000000n, true);
    else view.setFloat64(index * 8, value, true);
  });
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 0x100000001b3n);
  return hash.toString(16).padStart(16, '0');
}
