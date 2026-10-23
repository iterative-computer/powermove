/** Minimal EBML/WebM muxer. The element tree tracks lengths without repeatedly
 * concatenating compressed media at block, cluster, and segment boundaries. */
function vint(n: any, len?: any) {
  if (!len) { len = 1; while (n >= 2 ** (7 * len) - 1) len++; }
  const b: any = new Uint8Array(len);
  for (let i: any = len - 1; i >= 0; i--) { b[i] = n & 0xff; n = Math.floor(n / 256); }
  b[0] |= 1 << (8 - len);
  return b;
}
function uintBytes(n: any) {
  if (n === 0) return new Uint8Array([0]);
  const out: any = [];
  while (n > 0) { out.unshift(n & 0xff); n = Math.floor(n / 256); }
  return new Uint8Array(out);
}
function floatBytes(f: any) {
  const b: any = new Uint8Array(8); new DataView(b.buffer).setFloat64(0, f); return b;
}
function idBytes(id: any) {
  const out: any = [];
  let n: any = id;
  while (n > 0) { out.unshift(n & 0xff); n = Math.floor(n / 256); }
  return new Uint8Array(out);
}
function el(id: any, payload: any) {
  /* ArrayBuffer.isView is realm-safe (unlike instanceof), keeping the muxer testable headlessly */
  const data: any = ArrayBuffer.isView(payload) ? payload : join(payload);
  return join([idBytes(id), vint(data.length ?? data.byteLength), data]);
}

const str: any = (s: any) => new window.TextEncoder().encode(s);

export function muxWebM(frames: any, { width, height, fps, codecId = 'V_VP9', audio }: any, version: string | undefined) {
  const scale: any = 1e6; // 1ms ticks
  const header: any = el(0x1A45DFA3, [
    el(0x4286, uintBytes(1)), el(0x42F7, uintBytes(1)),
    el(0x42F2, uintBytes(4)), el(0x42F3, uintBytes(8)),
    el(0x4282, str('webm')), el(0x4287, uintBytes(2)), el(0x4285, uintBytes(2)),
  ]);
  /* merged, time-ordered block stream so clusters interleave video + audio */
  const blocks: any = frames.map((f: any) => ({ ms: Math.round(f.ts / 1000), track: 1, key: f.key, data: f.data }));
  if (audio && audio.chunks.length) {
    audio.chunks.forEach((b: any) => blocks.push({ ms: Math.round(b.ts / 1000), track: 2, key: false, data: b.data }));
    blocks.sort((a: any, b: any) => a.ms - b.ms || a.track - b.track);
  }
  const lastMs: any = blocks.length ? blocks[blocks.length - 1].ms : 0;
  const durMs: any = lastMs + 1000 / fps;
  const info: any = el(0x1549A966, [
    el(0x2AD7B1, uintBytes(scale)),
    el(0x4D80, str('Powermove')), el(0x5741, str('Powermove ' + version)),
    el(0x4489, floatBytes(durMs)),
  ]);
  const tracks: any = [
    el(0xAE, [
      el(0xD7, uintBytes(1)), el(0x73C5, uintBytes(1)), el(0x83, uintBytes(1)),
      el(0x536E, str('Video')), el(0x86, str(codecId)),
      el(0xE0, [el(0xB0, uintBytes(width)), el(0xBA, uintBytes(height))]),
    ]),
  ];
  if (audio && audio.chunks.length) {
    tracks.push(el(0xAE, [
      el(0xD7, uintBytes(2)), el(0x73C5, uintBytes(2)), el(0x83, uintBytes(2)),
      el(0x536E, str('Audio')), el(0x86, str('A_OPUS')),
      ...(audio.priv ? [el(0x63A2, audio.priv)] : []),
      el(0xE1, [
        el(0xB5, floatBytes(audio.rate || 48000)),
        el(0x9F, uintBytes(audio.channels || 2)),
      ]),
    ]));
  }
  const tracksEl: any = el(0x1654AE6B, tracks);
  /* one cluster per ~2s keeps block-relative timecodes inside their vint */
  const clusters: any = [];
  let cur: any = null, curStart: any = 0;
  for (const b of blocks) {
    if (!cur || b.ms - curStart > 2000) {
      if (cur) clusters.push(el(0x1F43B675, [el(0xE7, uintBytes(curStart)), ...cur]));
      cur = []; curStart = b.ms;
    }
    const rel: any = b.ms - curStart;
    const bh: any = new Uint8Array(4);
    bh[0] = 0x80 | b.track;          // track number, single-byte vint
    bh[1] = (rel >> 8) & 0xff; bh[2] = rel & 0xff;
    bh[3] = b.key ? 0x80 : 0x00;
    cur.push(el(0xA3, [bh, b.data]));
  }
  if (cur) clusters.push(el(0x1F43B675, [el(0xE7, uintBytes(curStart)), ...cur]));
  const segment: any = el(0x18538067, [info, tracksEl, ...clusters]);
  return new window.Blob(flatten([header, segment]), { type: 'video/webm' });
}


type Part = Uint8Array<ArrayBuffer> | { parts: Part[]; length: number };
function join(parts: Part[]) { return { parts, length: parts.reduce((sum, part) => sum + part.length, 0) }; }
/** Keep payload views intact until Blob snapshots them. Coalesce small leaves
 * so tiny frames do not turn into thousands of individual Blob parts. */
function flatten(parts: Part[]): BlobPart[] {
  const output: BlobPart[] = [], pending: Uint8Array<ArrayBuffer>[] = [];
  let length = 0;
  const flush = () => {
    if (!length) return;
    const bytes = new Uint8Array(length);
    let offset = 0;
    for (const part of pending) { bytes.set(part, offset); offset += part.length; }
    output.push(bytes); pending.length = 0; length = 0;
  };
  const visit = (part: Part) => {
    if (ArrayBuffer.isView(part)) {
      if (part.length >= 4096) { flush(); output.push(part); }
      else {
        if (length + part.length > 16 * 1024) flush();
        pending.push(part); length += part.length;
      }
    } else for (const child of part.parts) visit(child);
  };
  for (const part of parts) visit(part);
  flush();
  return output;
}
