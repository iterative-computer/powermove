import type { FigmaSelection } from './clipboard';
export const imageHash = (value: unknown): string =>
  typeof value === 'string'
    ? value.toLowerCase()
    : value instanceof Uint8Array || Array.isArray(value)
      ? Array.from(value)
          .map((v) => Number(v).toString(16).padStart(2, '0'))
          .join('')
      : '';
const imageBlob = (b: Uint8Array) =>
  (b[0] === 137 && b[1] === 80) ||
  (b[0] === 255 && b[1] === 216) ||
  (b[0] === 71 && b[1] === 73) ||
  (b[0] === 82 && b[1] === 73 && b[8] === 87 && b[9] === 69);
/** DataBlob is authoritative. Older payloads sometimes supply only a hash;
 * recover those bytes only after a digest matches, never by guessing from the
 * number of images or substituting the whole-selection preview. */
export async function clipboardImages(selection: FigmaSelection): Promise<Record<string, Uint8Array>> {
  const images: Record<string, Uint8Array> = {},
    wanted = new Set<string>(),
    queue: unknown[] = [selection.nodes];
  let visited = 0;
  while (queue.length) {
    const value = queue.pop();
    if (++visited > 300000) throw new Error('Figma document is too complex');
    if (!value || typeof value !== 'object' || value instanceof Uint8Array) continue;
    const paint = value as any;
    if (paint.type === 'IMAGE' && paint.image) {
      const hash = imageHash(paint.image.hash),
        bytes = selection.blobs[paint.image.dataBlob];
      if (hash && bytes?.length && imageBlob(bytes)) images[hash] = bytes;
      else if (hash) wanted.add(hash);
    }
    for (const item of Object.values(value)) if (item && typeof item === 'object') queue.push(item);
  }
  for (const hash of Object.keys(images)) wanted.delete(hash);
  if (!wanted.size || !globalThis.crypto?.subtle) return images;
  for (const bytes of selection.blobs) {
    if (!imageBlob(bytes)) continue;
    for (const [length, algorithm] of [
      [40, 'SHA-1'],
      [64, 'SHA-256'],
    ] as const) {
      if (![...wanted].some((h) => h.length === length)) continue;
      const hash = imageHash(new Uint8Array(await crypto.subtle.digest(algorithm, new Uint8Array(bytes))));
      if (wanted.has(hash)) {
        images[hash] = bytes;
        wanted.delete(hash);
      }
    }
    if (!wanted.size) break;
  }
  return images;
}
