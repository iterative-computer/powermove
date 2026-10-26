import { mp4BoxReader, mp4VideoTracks } from './mp4-boxes';

/* Chromium color-manages video before WebGL ever sees a pixel. For ordinary
   Rec.709 footage that is invisible. HDR footage (iPhone HLG, PQ masters) is
   tone mapped by the browser into a dim, desaturated SDR picture, and footage
   tagged BT.2020 (camera log, wide-gamut ProRes) has its code values remapped
   to sRGB primaries before any LUT or grade can read them. Both import through
   a playback proxy that settles the color once, so the viewer, previews and
   exports all read the same Rec.709 pixels. Names follow FFmpeg's. */

export interface VideoColor {
  range?: string;
  matrix?: string;
  primaries?: string;
  transfer?: string;
}

/** `tonemap`: HDR becomes display-referred Rec.709.
 *  `preserve`: R'G'B' code values stay untouched and are relabelled Rec.709,
 *  the input camera LUTs and grading effects expect. */
export type VideoColorTreatment = 'tonemap' | 'preserve';

const HDR_TRANSFERS = new Set(['smpte2084', 'arib-std-b67']);
const WIDE_PRIMARIES = new Set(['bt2020']);
const WIDE_MATRICES = new Set(['bt2020nc', 'bt2020c']);

export function videoColorTreatment(color: VideoColor | null | undefined): VideoColorTreatment | null {
  if (!color) return null;
  if (color.transfer && HDR_TRANSFERS.has(color.transfer)) return 'tonemap';
  if ((color.primaries && WIDE_PRIMARIES.has(color.primaries)) || (color.matrix && WIDE_MATRICES.has(color.matrix))) return 'preserve';
  return null;
}

/* ISO/IEC 23091-2 code points, as written in `colr` boxes. */
const PRIMARIES: Record<number, string> = { 1: 'bt709', 4: 'bt470m', 5: 'bt470bg', 6: 'smpte170m', 7: 'smpte240m', 9: 'bt2020', 11: 'smpte431', 12: 'smpte432' };
const TRANSFERS: Record<number, string> = {
  1: 'bt709', 4: 'gamma22', 5: 'gamma28', 6: 'smpte170m', 7: 'smpte240m', 8: 'linear', 13: 'iec61966-2-1',
  14: 'bt2020-10', 15: 'bt2020-12', 16: 'smpte2084', 18: 'arib-std-b67',
};
const MATRICES: Record<number, string> = { 0: 'gbr', 1: 'bt709', 5: 'bt470bg', 6: 'smpte170m', 7: 'smpte240m', 9: 'bt2020nc', 10: 'bt2020c' };

/** Bytes between a visual sample entry's header and its child boxes. */
const VISUAL_SAMPLE_ENTRY_BYTES = 78;

/** Read the first video track's `colr` (nclx/nclc) tags without decoding.
 * Absent or ICC-only tags return undefined; Chromium's defaults then apply. */
export async function mp4VideoColor(blob: Blob): Promise<VideoColor | undefined> {
  const boxes = mp4BoxReader(blob);
  try {
    for (const media of await mp4VideoTracks(blob, boxes)) {
      const minf = media.find(box => box.type === 'minf');
      const stbl = minf && (await boxes(minf.start, minf.end)).find(box => box.type === 'stbl');
      const stsd = stbl && (await boxes(stbl.start, stbl.end)).find(box => box.type === 'stsd');
      // A full box: version/flags, then the entry count, then sample entries.
      if (!stsd || stsd.end - stsd.start < 8) continue;
      const [entry] = await boxes(stsd.start + 8, stsd.end);
      if (!entry || entry.end - entry.start < VISUAL_SAMPLE_ENTRY_BYTES) continue;
      const colr = (await boxes(entry.start + VISUAL_SAMPLE_ENTRY_BYTES, entry.end)).find(box => box.type === 'colr');
      if (!colr || colr.end - colr.start < 10) continue;
      const view = new DataView(await blob.slice(colr.start, Math.min(colr.end, colr.start + 11)).arrayBuffer());
      const kind = String.fromCharCode(view.getUint8(0), view.getUint8(1), view.getUint8(2), view.getUint8(3));
      if (kind !== 'nclx' && kind !== 'nclc') continue;
      const color: VideoColor = {};
      const primaries = PRIMARIES[view.getUint16(4)], transfer = TRANSFERS[view.getUint16(6)], matrix = MATRICES[view.getUint16(8)];
      if (primaries) color.primaries = primaries;
      if (transfer) color.transfer = transfer;
      if (matrix) color.matrix = matrix;
      if (kind === 'nclx' && view.byteLength > 10) color.range = view.getUint8(10) & 0x80 ? 'pc' : 'tv';
      return color;
    }
  } catch { /* Missing or damaged tags must not prevent media import. */ }
  return undefined;
}
