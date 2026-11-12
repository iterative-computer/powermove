// Shared helpers for the visual parity tools: arguments, sample times, PNG I/O.
import { readFileSync } from 'node:fs';
import { deflateSync, inflateSync } from 'node:zlib';

export function parseArgs(argv) {
  const out = { step: 2, times: [] };
  for (let i = 0; i < argv.length; i++) {
    const key = argv[i].replace(/^--/, '');
    const value = argv[++i];
    if (value === undefined) throw new Error(`--${key} needs a value`);
    if (key === 'times') out.times = value.split(',').filter(Boolean);
    else if (key === 'step') out.step = Number(value);
    else out[key] = value;
  }
  for (const need of ['project', 'out']) if (!out[need]) throw new Error(`--${need} is required`);
  return out;
}

/** "12.5" is seconds; "13:11" is seconds:frames at the project's rate. */
export function parseTime(text, fps) {
  const m = /^(\d+):(\d+)$/.exec(text);
  return m ? Number(m[1]) + Number(m[2]) / fps : Number(text);
}

/** Every `step` seconds from 0, the last frame, and the named times; sorted, unique. */
export function sampleTimes(info, args) {
  const round = (t) => Math.round(t * 1e6) / 1e6;
  const set = new Set();
  const last = Math.max(0, info.dur - 1 / info.fps);
  if (args.step > 0) {
    for (let t = 0; t < last; t += args.step) set.add(round(t));
    set.add(round(last));
  }
  for (const t of args.times) set.add(round(parseTime(t, info.fps)));
  return [...set].sort((a, b) => a - b);
}

export const timeName = (t) => `t${t.toFixed(3).replace('.', '_')}`;

const CRC = new Int32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c;
});
function crc32(buf) {
  let c = -1;
  for (let i = 0; i < buf.length; i++) c = CRC[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}
function chunk(type, data) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, 'ascii');
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), data])), 0);
  return Buffer.concat([head, data, crc]);
}

/** RGBA8, top-down rows. */
export function encodePng(width, height, rgba) {
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0;
    rgba.copy?.(raw, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4)
      ?? raw.set(rgba.subarray(y * width * 4, (y + 1) * width * 4), y * (width * 4 + 1) + 1);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; ihdr[9] = 6; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw, { level: 6 })), chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** 8-bit RGB or RGBA, non-interlaced PNG to { width, height, rgba }. */
export function decodePng(file) {
  const buf = readFileSync(file);
  let pos = 8, width = 0, height = 0, type = 0, depth = 0;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos), kind = buf.toString('ascii', pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (kind === 'IHDR') { width = data.readUInt32BE(0); height = data.readUInt32BE(4); depth = data[8]; type = data[9]; if (data[12]) throw new Error(`${file}: interlaced`); }
    else if (kind === 'IDAT') idat.push(data);
    pos += 12 + len;
  }
  if (depth !== 8 || (type !== 6 && type !== 2)) throw new Error(`${file}: unsupported PNG (depth ${depth}, type ${type})`);
  const ch = type === 6 ? 4 : 3, stride = width * ch;
  const raw = inflateSync(Buffer.concat(idat));
  const px = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const f = raw[y * (stride + 1)], src = y * (stride + 1) + 1, row = y * stride, prev = row - stride;
    for (let x = 0; x < stride; x++) {
      const a = x >= ch ? px[row + x - ch] : 0, b = y ? px[prev + x] : 0, c = x >= ch && y ? px[prev + x - ch] : 0;
      let v = raw[src + x];
      if (f === 1) v += a; else if (f === 2) v += b; else if (f === 3) v += (a + b) >> 1;
      else if (f === 4) { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c; }
      px[row + x] = v & 0xff;
    }
  }
  if (ch === 4) return { width, height, rgba: px };
  const rgba = Buffer.alloc(width * height * 4);
  for (let i = 0, j = 0; i < px.length; i += 3, j += 4) { rgba[j] = px[i]; rgba[j + 1] = px[i + 1]; rgba[j + 2] = px[i + 2]; rgba[j + 3] = 255; }
  return { width, height, rgba };
}
