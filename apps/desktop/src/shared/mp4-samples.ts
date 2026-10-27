import { mp4BoxReader } from './mp4-boxes';

/** Where every compressed frame of an MP4/MOV video track lives and when it is
 * shown, for decoding a clip in order instead of seeking for every frame.
 * Arrays are in decode order. `pts` is in seconds on the same media timeline
 * Chromium's `<video>` uses: composition offsets and a single edit applied. */
export type Mp4VideoSamples = {
  codec: string;
  description?: Uint8Array;
  width: number;
  height: number;
  colorSpace?: VideoColorSpaceInit;
  offsets: Float64Array;
  sizes: Uint32Array;
  pts: Float64Array;
  keys: Uint8Array;
};

const MAX_MOOV_BYTES = 64 * 1024 * 1024;
const VISUAL_SAMPLE_ENTRY_BYTES = 78;

type Box = { type: string; start: number; end: number };
function children(view: DataView, start: number, end: number): Box[] {
  const boxes: Box[] = [];
  while (start + 8 <= end) {
    let size = view.getUint32(start), header = 8;
    const type = String.fromCharCode(view.getUint8(start + 4), view.getUint8(start + 5), view.getUint8(start + 6), view.getUint8(start + 7));
    if (size === 1) {
      if (start + 16 > end) break;
      size = Number(view.getBigUint64(start + 8)); header = 16;
    } else if (size === 0) size = end - start;
    if (!Number.isSafeInteger(size) || size < header || start + size > end) break;
    boxes.push({ type, start: start + header, end: start + size });
    start += size;
  }
  return boxes;
}
const find = (view: DataView, parent: Box | undefined, type: string) => parent && children(view, parent.start, parent.end).find(box => box.type === type);
const hex = (value: number) => value.toString(16).padStart(2, '0');

function hevcCodec(config: Uint8Array): string | undefined {
  if (config.length < 13) return;
  const space = config[1]! >> 6, tier = (config[1]! >> 5) & 1, profile = config[1]! & 31;
  let compatibility = 0;
  // The string carries the 32 compatibility flags in reverse bit order.
  const flags = (config[2]! << 24 | config[3]! << 16 | config[4]! << 8 | config[5]!) >>> 0;
  for (let bit = 0; bit < 32; bit++) if (flags & (1 << bit)) compatibility |= 1 << (31 - bit);
  const constraints = [...config.subarray(6, 12)];
  while (constraints.length && constraints[constraints.length - 1] === 0) constraints.pop();
  return `.${['', 'A', 'B', 'C'][space]}${profile}.${(compatibility >>> 0).toString(16)}.${tier ? 'H' : 'L'}${config[12]}`
    + constraints.map(value => '.' + value.toString(16).toUpperCase()).join('');
}

/* SDR tags only. HDR and wide-gamut sources import through a playback proxy
   (video-color.ts); any other tagged color keeps the seeking path. Code 2 is
   "unspecified", which leaves the choice to the stream like <video> does. */
const PRIMARIES: Record<number, VideoColorPrimaries> = { 1: 'bt709', 5: 'bt470bg', 6: 'smpte170m' };
const TRANSFERS: Record<number, VideoTransferCharacteristics> = { 1: 'bt709', 6: 'smpte170m', 13: 'iec61966-2-1' };
const MATRICES: Record<number, VideoMatrixCoefficients> = { 0: 'rgb', 1: 'bt709', 5: 'bt470bg', 6: 'smpte170m' };
const UNSPECIFIED = 2;

/** Read the first video track's sample table and decoder configuration.
 * Returns undefined for anything a plain in-order decode cannot reproduce
 * exactly like `<video>` (fragmented files, rotation, non-square pixels,
 * multiple edits, unsupported codecs); callers then keep seeking. */
export async function mp4VideoSamples(blob: Blob): Promise<Mp4VideoSamples | undefined> {
  try {
    const moovBox = (await mp4BoxReader(blob)(0, blob.size)).find(box => box.type === 'moov');
    if (!moovBox || moovBox.end - moovBox.start > MAX_MOOV_BYTES) return;
    const view = new DataView(await blob.slice(moovBox.start, moovBox.end).arrayBuffer());
    const bytes = new Uint8Array(view.buffer);
    const moov = children(view, 0, view.byteLength);
    if (moov.some(box => box.type === 'mvex')) return;
    const mvhd = moov.find(box => box.type === 'mvhd');
    if (!mvhd) return;
    for (const trak of moov.filter(box => box.type === 'trak')) {
      const mdia = find(view, trak, 'mdia'), hdlr = find(view, mdia, 'hdlr');
      if (!hdlr || hdlr.end - hdlr.start < 12 || String.fromCharCode(...bytes.subarray(hdlr.start + 8, hdlr.start + 12)) !== 'vide') continue;
      // Only the first video track plays; anything unusual about it keeps seeking.
      return readTrack(view, bytes, trak, mdia!);
    }
  } catch { /* Damaged or unexpected structure keeps the seeking path. */ }
}

function readTrack(view: DataView, bytes: Uint8Array, trak: Box, mdia: Box): Mp4VideoSamples | undefined {
  const tkhd = find(view, trak, 'tkhd');
  if (!tkhd) return;
  // a, b, c, d of the display matrix: anything but identity rotates or flips.
  const matrix = tkhd.start + (view.getUint8(tkhd.start) === 1 ? 52 : 40);
  if (view.getInt32(matrix) !== 0x10000 || view.getInt32(matrix + 4) !== 0 || view.getInt32(matrix + 12) !== 0 || view.getInt32(matrix + 16) !== 0x10000) return;
  const mdhd = find(view, mdia, 'mdhd');
  if (!mdhd) return;
  const timescale = view.getUint32(mdhd.start + (view.getUint8(mdhd.start) === 1 ? 20 : 12));
  if (!timescale) return;
  let mediaStart = 0;
  const elst = find(view, find(view, trak, 'edts'), 'elst');
  if (elst) {
    const version = view.getUint8(elst.start), count = view.getUint32(elst.start + 4);
    if (count !== 1) return;
    const entry = elst.start + 8;
    mediaStart = version === 1 ? Number(view.getBigInt64(entry + 8)) : view.getInt32(entry + 4);
    const rate = view.getInt16(entry + (version === 1 ? 16 : 8));
    if (mediaStart < 0 || rate !== 1) return;
  }
  const stbl = find(view, find(view, mdia, 'minf'), 'stbl');
  const stsd = find(view, stbl, 'stsd');
  if (!stbl || !stsd || view.getUint32(stsd.start + 4) !== 1) return;
  const [entry] = children(view, stsd.start + 8, stsd.end);
  if (!entry || entry.end - entry.start < VISUAL_SAMPLE_ENTRY_BYTES) return;
  const width = view.getUint16(entry.start + 24), height = view.getUint16(entry.start + 26);
  const extensions = children(view, entry.start + VISUAL_SAMPLE_ENTRY_BYTES, entry.end);
  const extension = (type: string) => extensions.find(box => box.type === type);
  const payload = (box: Box | undefined) => box && bytes.slice(box.start, box.end);
  const pasp = extension('pasp');
  if (pasp && view.getUint32(pasp.start) !== view.getUint32(pasp.start + 4)) return;
  let codec: string | undefined, description: Uint8Array | undefined;
  if (entry.type === 'avc1' || entry.type === 'avc3') {
    description = payload(extension('avcC'));
    if (!description || description.length < 4) return;
    codec = `${entry.type}.${hex(description[1]!)}${hex(description[2]!)}${hex(description[3]!)}`;
  } else if (entry.type === 'hvc1' || entry.type === 'hev1') {
    description = payload(extension('hvcC'));
    const suffix = description && hevcCodec(description);
    if (!suffix) return;
    codec = entry.type + suffix;
  } else if (entry.type === 'av01') {
    const config = payload(extension('av1C'));
    if (!config || config.length < 4) return;
    const depth = config[2]! & 0x20 ? 12 : config[2]! & 0x40 ? 10 : 8;
    codec = `av01.${config[1]! >> 5}.${String(config[1]! & 31).padStart(2, '0')}${config[2]! & 0x80 ? 'H' : 'M'}.${String(depth).padStart(2, '0')}`;
  } else if (entry.type === 'vp09') {
    const config = payload(extension('vpcC'));
    if (!config || config.length < 7) return;
    codec = `vp09.${hex(config[4]!)}.${hex(config[5]!)}.${hex(config[6]! >> 4)}`;
  } else return;
  let colorSpace: VideoColorSpaceInit | undefined;
  const colr = extension('colr');
  if (colr && colr.end - colr.start >= 11 && String.fromCharCode(...bytes.subarray(colr.start, colr.start + 4)) === 'nclx') {
    const codes = [view.getUint16(colr.start + 4), view.getUint16(colr.start + 6), view.getUint16(colr.start + 8)] as const;
    const primaries = PRIMARIES[codes[0]], transfer = TRANSFERS[codes[1]], matrix = MATRICES[codes[2]];
    if (codes[0] !== UNSPECIFIED && !primaries || codes[1] !== UNSPECIFIED && !transfer || codes[2] !== UNSPECIFIED && !matrix) return;
    colorSpace = { fullRange: !!(view.getUint8(colr.start + 10) & 0x80),
      ...(primaries ? { primaries } : {}), ...(transfer ? { transfer } : {}), ...(matrix ? { matrix } : {}) };
  }

  const table = (type: string) => find(view, stbl, type);
  const stsz = table('stsz'), stts = table('stts'), stsc = table('stsc'), chunks = table('stco') ?? table('co64');
  if (!stsz || !stts || !stsc || !chunks) return;
  const count = view.getUint32(stsz.start + 8), fixed = view.getUint32(stsz.start + 4);
  if (!count || !fixed && stsz.end - stsz.start < 12 + count * 4) return;
  const sizes = new Uint32Array(count);
  for (let i = 0; i < count; i++) sizes[i] = fixed || view.getUint32(stsz.start + 12 + i * 4);

  const chunkCount = view.getUint32(chunks.start + 4), wide = chunks.type === 'co64';
  if (chunks.end - chunks.start < 8 + chunkCount * (wide ? 8 : 4)) return;
  const chunkOffset = (i: number) => wide ? Number(view.getBigUint64(chunks.start + 8 + i * 8)) : view.getUint32(chunks.start + 8 + i * 4);
  const offsets = new Float64Array(count);
  const runs = view.getUint32(stsc.start + 4);
  if (!runs || stsc.end - stsc.start < 8 + runs * 12) return;
  let sample = 0;
  for (let run = 0; run < runs && sample < count; run++) {
    const at = stsc.start + 8 + run * 12;
    const first = view.getUint32(at) - 1, perChunk = view.getUint32(at + 4);
    if (view.getUint32(at + 8) !== 1) return;
    const last = run + 1 < runs ? view.getUint32(at + 12) - 1 : chunkCount;
    for (let chunk = first; chunk < last && sample < count; chunk++) {
      let offset = chunkOffset(chunk);
      for (let i = 0; i < perChunk && sample < count; i++) { offsets[sample] = offset; offset += sizes[sample++]!; }
    }
  }
  if (sample !== count) return;

  const pts = new Float64Array(count);
  const timings = view.getUint32(stts.start + 4);
  if (stts.end - stts.start < 8 + timings * 8) return;
  let dts = 0; sample = 0;
  for (let i = 0; i < timings; i++) {
    const samples = view.getUint32(stts.start + 8 + i * 8), delta = view.getUint32(stts.start + 12 + i * 8);
    for (let j = 0; j < samples && sample < count; j++) { pts[sample++] = dts; dts += delta; }
  }
  if (sample !== count) return;
  const ctts = table('ctts');
  if (ctts) {
    const runs = view.getUint32(ctts.start + 4);
    if (ctts.end - ctts.start < 8 + runs * 8) return;
    sample = 0;
    for (let i = 0; i < runs; i++) {
      // Version 0 is unsigned on paper, but encoders write negative offsets in
      // it too; FFmpeg (and so Chromium) reads both versions as signed.
      const samples = view.getUint32(ctts.start + 8 + i * 8), offset = view.getInt32(ctts.start + 12 + i * 8);
      for (let j = 0; j < samples && sample < count; j++) pts[sample++]! += offset;
    }
  }
  for (let i = 0; i < count; i++) pts[i] = (pts[i]! - mediaStart) / timescale;

  const keys = new Uint8Array(count);
  const stss = table('stss');
  if (stss) {
    const entries = view.getUint32(stss.start + 4);
    if (stss.end - stss.start < 8 + entries * 4) return;
    for (let i = 0; i < entries; i++) {
      const index = view.getUint32(stss.start + 8 + i * 4) - 1;
      if (index >= 0 && index < count) keys[index] = 1;
    }
  } else keys.fill(1);
  if (!keys[0]) return;
  return { codec, ...(description ? { description } : {}), width, height, ...(colorSpace ? { colorSpace } : {}), offsets, sizes, pts, keys };
}
