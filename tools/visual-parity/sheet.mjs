// The visual parity contact sheet (README.md).
//
//   node tools/visual-parity/sheet.mjs --dir <out> [--title Paper] [--notes notes.json] [--summary summary.txt]
//
// Reads <dir>/v2/manifest.json and, when present, the delivered video's frames
// (ref/, with ref-prev/ and ref-next/), v1's (v1/) and the paused viewer's
// (v2viewer/). Writes a heat map per sample to <dir>/diff/ and <dir>/index.html:
// rows are times, columns are video | v1 | V2 | V2 viewer | difference, then
// every frame ranked by perceptible difference. V2 is judged against the
// video when there is one, else against v1.
//
// Perceptible: both frames are averaged over 2×2 then 3×3 (compression noise
// and anti-aliasing go), converted to CIELAB, and a pixel counts when ΔE76 >
// 6. "Visible" (the older measure: 3×3 averaged, a channel off by more than
// 16/255) is kept beside it. Timing: V2 is also compared with the video's
// frames one before and one after; when one of them is clearly closer, the row
// says so (a timing difference, not a content one).
// notes.json maps a time label ("t13_458") to the cause or the accepted reason.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, encodePng } from './lib.mjs';

const argv = process.argv.slice(2);
const opt = {};
for (let i = 0; i < argv.length; i += 2) opt[argv[i].replace(/^--/, '')] = argv[i + 1];
const dir = path.resolve(opt.dir);
const title = opt.title ?? path.basename(dir);
const notes = opt.notes && existsSync(opt.notes) ? JSON.parse(readFileSync(opt.notes, 'utf8')) : {};
// Paragraphs (blank-line separated) shown under the title: the findings.
const summary = opt.summary && existsSync(opt.summary) ? readFileSync(opt.summary, 'utf8').trim().split(/\n\s*\n/) : [];
const manifest = (name) => (existsSync(path.join(dir, name, 'manifest.json')) ? JSON.parse(readFileSync(path.join(dir, name, 'manifest.json'), 'utf8')) : null);
const v1 = manifest('v1');
const v2 = manifest('v2') ?? { samples: [] };
const viewer = manifest('v2viewer');
const ref = manifest('ref');
const prev = manifest('ref-prev');
const next = manifest('ref-next');
const byTime = (m, dirName) => new Map((m?.samples ?? []).map((s) => [s.time.toFixed(3), `${dirName}/${s.file}`]));
const viewerByTime = byTime(viewer, 'v2viewer');
const refByTime = byTime(ref, 'ref');
const prevByTime = byTime(prev, 'ref-prev');
const nextByTime = byTime(next, 'ref-next');
const v1ByTime = byTime(v1, 'v1');
const project = (v1 ?? v2).project;
mkdirSync(path.join(dir, 'diff'), { recursive: true });

const THRESHOLD = 16;
const DELTA_E = 6;
function boxBlur3(src, w, h, channels) {
  const out = new Float32Array(w * h * channels);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const acc = new Float32Array(channels);
    let n = 0;
    for (let dy = -1; dy <= 1; dy++) {
      const yy = y + dy;
      if (yy < 0 || yy >= h) continue;
      for (let dx = -1; dx <= 1; dx++) {
        const xx = x + dx;
        if (xx < 0 || xx >= w) continue;
        const i = (yy * w + xx) * channels;
        for (let c = 0; c < channels; c++) acc[c] += src[i + c];
        n++;
      }
    }
    const o = (y * w + x) * channels;
    for (let c = 0; c < channels; c++) out[o + c] = acc[c] / n;
  }
  return out;
}
const rgbOf = (img) => {
  const { width: w, height: h, rgba } = img;
  const out = new Float32Array(w * h * 3);
  for (let i = 0, j = 0; j < out.length; i += 4, j += 3) { out[j] = rgba[i]; out[j + 1] = rgba[i + 1]; out[j + 2] = rgba[i + 2]; }
  return out;
};
function half(src, w, h) {
  const hw = w >> 1, hh = h >> 1;
  const out = new Float32Array(hw * hh * 3);
  for (let y = 0; y < hh; y++) for (let x = 0; x < hw; x++) for (let c = 0; c < 3; c++) {
    const at = (xx, yy) => src[((y * 2 + yy) * w + x * 2 + xx) * 3 + c];
    out[(y * hw + x) * 3 + c] = (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4;
  }
  return out;
}
const lin = (v) => { v /= 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
const fLab = (t) => (t > 216 / 24389 ? Math.cbrt(t) : (24389 / 27 * t + 16) / 116);
function lab(src) {
  const out = new Float32Array(src.length);
  for (let i = 0; i < src.length; i += 3) {
    const r = lin(src[i]), g = lin(src[i + 1]), b = lin(src[i + 2]);
    const x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    const y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    const z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    const fx = fLab(x), fy = fLab(y), fz = fLab(z);
    out[i] = 116 * fy - 16; out[i + 1] = 500 * (fx - fy); out[i + 2] = 200 * (fy - fz);
  }
  return out;
}
const cache = new Map();
function prepared(file) {
  if (cache.has(file)) return cache.get(file);
  const img = decodePng(path.join(dir, file));
  const rgb = rgbOf(img);
  const p = { w: img.width, h: img.height, visible: boxBlur3(rgb, img.width, img.height, 3), lab: lab(boxBlur3(half(rgb, img.width, img.height), img.width >> 1, img.height >> 1, 3)) };
  if (cache.size > 8) cache.delete(cache.keys().next().value);
  cache.set(file, p);
  return p;
}
function heat(v) {
  // black under the threshold, then blue -> red -> yellow -> white
  if (v <= DELTA_E) return [v * 4, v * 4, v * 4];
  const t = Math.min(1, (v - DELTA_E) / 30);
  if (t < 1 / 3) return [255 * t * 3, 0, 255 * (1 - t * 3) + 64 * t * 3];
  if (t < 2 / 3) return [255, 255 * (t - 1 / 3) * 3, 0];
  return [255, 255, 255 * (t - 2 / 3) * 3];
}
// Perceptible and visible difference of two frames; writes the ΔE heat map
// (at half size) when `diff` is given.
function compare(fileA, fileB, diff) {
  const a = prepared(fileA), b = prepared(fileB);
  if (a.w !== b.w || a.h !== b.h) return { error: `size ${a.w}x${a.h} vs ${b.w}x${b.h}` };
  let visible = 0;
  for (let i = 0; i < a.visible.length; i += 3) {
    const d = Math.max(Math.abs(a.visible[i] - b.visible[i]), Math.abs(a.visible[i + 1] - b.visible[i + 1]), Math.abs(a.visible[i + 2] - b.visible[i + 2]));
    if (d > THRESHOLD) visible++;
  }
  const hw = a.w >> 1, hh = a.h >> 1;
  const px = diff ? Buffer.alloc(hw * hh * 4) : null;
  let perceptible = 0, sum = 0;
  for (let i = 0, j = 0; i < a.lab.length; i += 3, j += 4) {
    const d = Math.hypot(a.lab[i] - b.lab[i], a.lab[i + 1] - b.lab[i + 1], a.lab[i + 2] - b.lab[i + 2]);
    sum += d; if (d > DELTA_E) perceptible++;
    if (px) { const [r, g, bb] = heat(d); px[j] = r; px[j + 1] = g; px[j + 2] = bb; px[j + 3] = 255; }
  }
  if (px) writeFileSync(path.join(dir, diff), encodePng(hw, hh, px));
  return { perceptible: perceptible / (hw * hh), visible: visible / (a.w * a.h), mean: sum / (hw * hh) };
}

const rows = [];
const v2ByTime = new Map(v2.samples.map((s) => [s.time.toFixed(3), s]));
for (const s of v1?.samples ?? v2.samples) {
  const key = s.time.toFixed(3);
  if (ref && !refByTime.has(key)) continue;
  const label = s.file.replace(/^.*\//, '').replace(/\.png$/, '');
  const row = { time: s.time, label, v1: v1ByTime.get(key), ref: refByTime.get(key), viewer: viewerByTime.get(key), note: notes[label] };
  const other = v2ByTime.get(key);
  if (!other) { row.missing = 'no V2 frame'; rows.push(row); continue; }
  row.v2 = `v2/${other.file}`;
  const base = row.ref ?? row.v1;
  const result = compare(base, row.v2, `diff/${label}.png`);
  if (result.error) { row.missing = result.error; rows.push(row); continue; }
  Object.assign(row, result, { diff: `diff/${label}.png` });
  if (row.ref && row.v1) row.v1vs = compare(row.ref, row.v1).perceptible;
  // Timing: the neighbouring video frames.
  const around = [[-1, prevByTime.get(key)], [1, nextByTime.get(key)]].filter(([, f]) => f);
  for (const [shift, file] of around) {
    const p = compare(file, row.v2).perceptible;
    if (p < 0.7 * row.perceptible && row.perceptible - p > 0.001 && (row.timing === undefined || p < row.timing.perceptible)) row.timing = { shift, perceptible: p };
  }
  rows.push(row);
  console.log(`${label} perceptible ${(row.perceptible * 100).toFixed(2)}% visible ${(row.visible * 100).toFixed(2)}%${row.timing ? ` (video frame ${row.timing.shift > 0 ? '+' : ''}${row.timing.shift}: ${(row.timing.perceptible * 100).toFixed(2)}%)` : ''}`);
}

const pct = (v) => (v === undefined ? '—' : `${(v * 100).toFixed(2)} %`);
const esc = (t) => String(t ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const tc = (t, fps) => `${Math.floor(t + 1e-6)}:${String(Math.round((t - Math.floor(t + 1e-6)) * fps)).padStart(2, '0')}`;
const fps = project.fps;
const ranked = rows.filter((r) => r.perceptible !== undefined).sort((x, y) => y.perceptible - x.perceptible);
const BAD = 0.002;
const timingNote = (r) => (r.timing ? `Closer to the video frame ${r.timing.shift > 0 ? 'after' : 'before'} (${pct(r.timing.perceptible)}): timing. ` : '');
const columns = [ref && 'Delivered video', v1 && 'v1.2.0', 'V2 export', viewer && 'V2 viewer, paused (screen)', 'Difference'].filter(Boolean);
const img = (src, alt) => `<td class="img">${src ? `<a href="${src}"><img loading="lazy" src="${src}" alt="${esc(alt)}"></a>` : ''}</td>`;
const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)} parity</title>
<style>
:root{color-scheme:light dark;--bg:#fff;--fg:#1d1d1f;--muted:#6e6e73;--line:#d2d2d7;--bad:#c4302b;--ok:#2a7d36}
@media (prefers-color-scheme:dark){:root{--bg:#1c1c1e;--fg:#f5f5f7;--muted:#a1a1a6;--line:#3a3a3c;--bad:#ff6961;--ok:#5fd27a}}
body{margin:0;padding:16px;background:var(--bg);color:var(--fg);font:13px/1.4 -apple-system,system-ui,sans-serif}
h1{font-size:20px;margin:0 0 4px}p{margin:0 0 12px;color:var(--muted)}
table{border-collapse:collapse;width:100%}td,th{border-top:1px solid var(--line);padding:6px;vertical-align:top;text-align:left}
th{font-weight:600;color:var(--muted)}img{width:100%;display:block;cursor:zoom-in}
.cols td.img{width:${Math.floor(92 / columns.length)}%}.num{white-space:nowrap;font-variant-numeric:tabular-nums}.bad{color:var(--bad)}.ok{color:var(--ok)}
.note{color:var(--muted);max-width:40ch}.summary{color:var(--fg);max-width:90ch}
</style></head><body>
<h1>${esc(title)}: ${columns.slice(0, -1).map(esc).join(' | ')}</h1>
<p>${esc(project.w)}×${esc(project.h)} at ${esc(fps)} fps, ${rows.length} samples. ${ref ? `Reference: ${esc(ref.source)}; the heat map is V2 against it. ` : ''}${v1 ? `v1: ${esc(v1.source)}. ` : ''}V2: ${esc(v2.source ?? 'missing')}.
Perceptible = share of pixels whose colour, averaged over 2×2 then 3×3 (compression noise and anti-aliasing go), differs by ΔE76 > ${DELTA_E}; visible = 3×3 averaged, a channel off by more than ${THRESHOLD}/255. Rows over ${pct(BAD)} perceptible are red. Heat (half size): black = same, blue → red → yellow = larger. Click an image for full size.${viewer ? ` ${esc(viewer.source)}: for looking, not measured.` : ''}</p>
${summary.map((p) => `<p class="summary">${esc(p)}</p>`).join('\n')}
<h2>Ranked by perceptible difference</h2>
<table><tr><th>Time</th><th>Perceptible${ref ? ' (V2 vs video)' : ''}</th><th>Visible</th>${ref && v1 ? '<th>v1 vs video</th>' : ''}<th>Mean ΔE</th><th>Cause or accepted reason</th></tr>
${ranked.map((r) => `<tr><td class="num"><a href="#${r.label}">${tc(r.time, fps)} (${r.time.toFixed(3)} s)</a></td><td class="num ${r.perceptible > BAD ? 'bad' : 'ok'}">${pct(r.perceptible)}</td><td class="num">${pct(r.visible)}</td>${ref && v1 ? `<td class="num">${pct(r.v1vs)}</td>` : ''}<td class="num">${r.mean.toFixed(2)}</td><td class="note">${esc(timingNote(r) + (r.note ?? ''))}</td></tr>`).join('\n')}
${rows.filter((r) => r.missing).map((r) => `<tr><td class="num">${r.time.toFixed(3)} s</td><td class="bad" colspan="5">${esc(r.missing)}</td></tr>`).join('\n')}
</table>
<h2>Frames</h2>
<table class="cols"><tr><th>Time</th>${columns.map((c) => `<th>${esc(c)}</th>`).join('')}</tr>
${rows.map((r) => `<tr id="${r.label}"><td class="num">${tc(r.time, fps)}<br>${r.time.toFixed(3)} s<br><span class="${r.perceptible > BAD ? 'bad' : 'ok'}">${pct(r.perceptible)}</span><div class="note">${esc(timingNote(r) + (r.note ?? r.missing ?? ''))}</div></td>
${ref ? img(r.ref, `video ${r.time}`) : ''}${v1 ? img(r.v1, `v1 ${r.time}`) : ''}${img(r.v2, `V2 ${r.time}`)}${viewer ? img(r.viewer, `V2 viewer ${r.time}`) : ''}${img(r.diff, `difference ${r.time}`)}</tr>`).join('\n')}
</table></body></html>`;
writeFileSync(path.join(dir, 'index.html'), html);
writeFileSync(path.join(dir, 'ranked.json'), JSON.stringify(ranked.map(({ time, label, perceptible, visible, v1vs, mean, timing, note }) => ({ time, label, perceptible, visible, v1vs, mean, timing, note })), null, 1));
console.log(path.join(dir, 'index.html'));
