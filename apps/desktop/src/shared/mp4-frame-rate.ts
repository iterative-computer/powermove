import { mp4BoxReader, mp4VideoTracks } from './mp4-boxes';

/** Read MP4/MOV sample timing without loading the compressed video into memory.
 * Fractional rates (such as 30000/1001) stay fractional. For variable frame
 * timing, a composition's single frame rate is the source's average rate. */
export async function mp4FrameRate(blob: Blob): Promise<number | undefined> {
  const boxes = mp4BoxReader(blob);
  try {
    for (const media of await mp4VideoTracks(blob, boxes)) {
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
