export type Box = { type: string; start: number; end: number };

/** Walk MP4/MOV boxes by reading only their headers. Each reader carries one
 * header budget across all of its calls so a damaged file cannot loop forever. */
export function mp4BoxReader(blob: Blob, budget = 4096): (start: number, end: number) => Promise<Box[]> {
  let remaining = budget;
  return async (start, end) => {
    const result: Box[] = [];
    while (start + 8 <= end && remaining-- > 0) {
      const bytes = await blob.slice(start, Math.min(start + 16, end)).arrayBuffer();
      const view = new DataView(bytes);
      let size = view.getUint32(0), header = 8;
      const type = new TextDecoder().decode(new Uint8Array(bytes, 4, 4));
      if (size === 1) {
        if (bytes.byteLength < 16) return [];
        size = Number(view.getBigUint64(8)); header = 16;
      } else if (size === 0) size = end - start;
      if (!Number.isSafeInteger(size) || size < header || start + size > end) return [];
      result.push({ type, start: start + header, end: start + size });
      start += size;
    }
    return result;
  };
}

/** The `mdia` box of every video track, in file order. */
export async function mp4VideoTracks(blob: Blob, boxes: (start: number, end: number) => Promise<Box[]>): Promise<Box[][]> {
  const moov = (await boxes(0, blob.size)).find(box => box.type === 'moov');
  if (!moov) return [];
  const tracks: Box[][] = [];
  for (const track of (await boxes(moov.start, moov.end)).filter(box => box.type === 'trak')) {
    const mdia = (await boxes(track.start, track.end)).find(box => box.type === 'mdia');
    if (!mdia) continue;
    const media = await boxes(mdia.start, mdia.end);
    const handler = media.find(box => box.type === 'hdlr');
    if (!handler || handler.end - handler.start < 12) continue;
    if (await blob.slice(handler.start + 8, handler.start + 12).text() === 'vide') tracks.push(media);
  }
  return tracks;
}
