type Box = { type: string; start: number; end: number };

/** Read MP4/MOV sample timing without loading the compressed video into memory.
 * Fractional rates (such as 30000/1001) stay fractional. For variable frame
 * timing, a composition's single frame rate is the source's average rate. */
export async function mp4FrameRate(blob: Blob): Promise<number | undefined> {
  let remaining = 4096;
  async function boxes(start: number, end: number): Promise<Box[]> {
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
  }
  try {
    const root = await boxes(0, blob.size);
    const moov = root.find(box => box.type === 'moov');
    if (!moov) return;
    for (const track of (await boxes(moov.start, moov.end)).filter(box => box.type === 'trak')) {
      const mdia = (await boxes(track.start, track.end)).find(box => box.type === 'mdia');
      if (!mdia) continue;
      const media = await boxes(mdia.start, mdia.end);
      const handler = media.find(box => box.type === 'hdlr');
      if (!handler || handler.end - handler.start < 12) continue;
      const kind = await blob.slice(handler.start + 8, handler.start + 12).text();
      if (kind !== 'vide') continue;
      const mdhd = media.find(box => box.type === 'mdhd');
      const minf = media.find(box => box.type === 'minf');
      if (!mdhd || !minf) continue;
      const header = new DataView(await blob.slice(mdhd.start, Math.min(mdhd.start + 32, mdhd.end)).arrayBuffer());
      const version = header.getUint8(0);
      if (version !== 0 && version !== 1) continue;
      const timescale = header.getUint32(version === 1 ? 20 : 12);
      const stbl = (await boxes(minf.start, minf.end)).find(box => box.type === 'stbl');
      if (!stbl) continue;
      const stts = (await boxes(stbl.start, stbl.end)).find(box => box.type === 'stts');
      if (!stts || stts.end - stts.start > 4 * 1024 * 1024) continue;
      const timing = new DataView(await blob.slice(stts.start, stts.end).arrayBuffer());
      const count = timing.getUint32(4);
      if (count > (timing.byteLength - 8) / 8) continue;
      let frames = 0, ticks = 0;
      for (let i = 0; i < count; i++) {
        const samples = timing.getUint32(8 + i * 8), delta = timing.getUint32(12 + i * 8);
        frames += samples; ticks += samples * delta;
      }
      const fps = frames * timescale / ticks;
      if (Number.isFinite(fps) && fps > 0) return fps;
    }
  } catch { /* Missing or damaged timing must not prevent media import. */ }
}
