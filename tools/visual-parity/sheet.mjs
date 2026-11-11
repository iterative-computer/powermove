// The visual parity contact sheet (README.md).
//
//   node tools/visual-parity/sheet.mjs --dir <out> [--title Paper] [--notes notes.json] [--summary summary.txt]
//
// Reads <dir>/v1/manifest.json and <dir>/v2/manifest.json, writes a heat map
// per sample to <dir>/diff/ and <dir>/index.html: rows are times, columns are
// v1 | V2 | difference, then every frame ranked by visible difference.
// "Visible" ignores anti-aliasing: each image is box-filtered over 3x3 first,
// and a pixel counts when a channel still differs by more than 16/255.
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
const v1 = JSON.parse(readFileSync(path.join(dir, 'v1/manifest.json'), 'utf8'));
const v2 = existsSync(path.join(dir, 'v2/manifest.json')) ? JSON.parse(readFileSync(path.join(dir, 'v2/manifest.json'), 'utf8')) : { samples: [] };
const viewer = existsSync(path.join(dir, 'v2viewer/manifest.json')) ? JSON.parse(readFileSync(path.join(dir, 'v2viewer/manifest.json'), 'utf8')) : null;
const viewerByTime = new Map((viewer?.samples ?? []).map((s) => [s.time.toFixed(3), `v2viewer/${s.file}`]));
mkdirSync(path.join(dir, 'diff'), { recursive: true });

const THRESHOLD = 16;
function blur3(img) {
  const { width: w, height: h, rgba } = img;
  const out = new Float32Array(w * h * 3);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    let r = 0, g = 0, b = 0, n = 0;
    for (let dy = -1; dy <= 1; dy++) {
      const yy = y + dy;
      if (yy < 0 || yy >= h) continue;
      for (let dx = -1; dx <= 1; dx++) {
        const xx = x + dx;
        if (xx < 0 || xx >= w) continue;
        const i = (yy * w + xx) * 4;
        r += rgba[i]; g += rgba[i + 1]; b += rgba[i + 2]; n++;
      }
    }
    const o = (y * w + x) * 3;
    out[o] = r / n; out[o + 1] = g / n; out[o + 2] = b / n;
  }
  return out;
}
function heat(v) {
  // black under the threshold, then blue -> red -> yellow -> white
  if (v <= THRESHOLD) return [v * 2, v * 2, v * 2];
  const t = Math.min(1, (v - THRESHOLD) / 96);
  if (t < 1 / 3) return [255 * t * 3, 0, 255 * (1 - t * 3) + 64 * t * 3];
  if (t < 2 / 3) return [255, 255 * (t - 1 / 3) * 3, 0];
  return [255, 255, 255 * (t - 2 / 3) * 3];
}

const rows = [];
const v2ByTime = new Map(v2.samples.map((s) => [s.time.toFixed(3), s]));
for (const s of v1.samples) {
  const key = s.time.toFixed(3);
  const label = s.file.replace(/\.png$/, '');
  const row = { time: s.time, label, v1: `v1/${s.file}`, viewer: viewerByTime.get(key), note: notes[label] };
  const other = v2ByTime.get(key);
  if (!other) { row.missing = 'no V2 frame'; rows.push(row); continue; }
  row.v2 = `v2/${other.file}`;
  const a = decodePng(path.join(dir, row.v1)), b = decodePng(path.join(dir, row.v2));
  if (a.width !== b.width || a.height !== b.height) { row.missing = `size ${a.width}x${a.height} vs ${b.width}x${b.height}`; rows.push(row); continue; }
  const fa = blur3(a), fb = blur3(b);
  const px = Buffer.alloc(a.width * a.height * 4);
  let visible = 0, sum = 0, max = 0;
  for (let i = 0, j = 0; i < fa.length; i += 3, j += 4) {
    const d = Math.max(Math.abs(fa[i] - fb[i]), Math.abs(fa[i + 1] - fb[i + 1]), Math.abs(fa[i + 2] - fb[i + 2]));
    sum += d; if (d > max) max = d; if (d > THRESHOLD) visible++;
    const [r, g, bb] = heat(d);
    px[j] = r; px[j + 1] = g; px[j + 2] = bb; px[j + 3] = 255;
  }
  row.diff = `diff/${label}.png`;
  writeFileSync(path.join(dir, row.diff), encodePng(a.width, a.height, px));
  row.visible = visible / (a.width * a.height);
  row.mean = sum / (a.width * a.height);
  row.max = max;
  rows.push(row);
  console.log(`${label} visible ${(row.visible * 100).toFixed(2)}% mean ${row.mean.toFixed(2)} max ${max.toFixed(0)}`);
}

const pct = (v) => (v === undefined ? '—' : `${(v * 100).toFixed(2)} %`);
const esc = (t) => String(t ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const tc = (t, fps) => `${Math.floor(t + 1e-6)}:${String(Math.round((t - Math.floor(t + 1e-6)) * fps)).padStart(2, '0')}`;
const fps = v1.project.fps;
const ranked = rows.filter((r) => r.visible !== undefined).sort((x, y) => y.visible - x.visible);
const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)} parity</title>
<style>
:root{color-scheme:light dark;--bg:#fff;--fg:#1d1d1f;--muted:#6e6e73;--line:#d2d2d7;--bad:#c4302b;--ok:#2a7d36}
@media (prefers-color-scheme:dark){:root{--bg:#1c1c1e;--fg:#f5f5f7;--muted:#a1a1a6;--line:#3a3a3c;--bad:#ff6961;--ok:#5fd27a}}
body{margin:0;padding:16px;background:var(--bg);color:var(--fg);font:13px/1.4 -apple-system,system-ui,sans-serif}
h1{font-size:20px;margin:0 0 4px}p{margin:0 0 12px;color:var(--muted)}
table{border-collapse:collapse;width:100%}td,th{border-top:1px solid var(--line);padding:6px;vertical-align:top;text-align:left}
th{font-weight:600;color:var(--muted)}img{width:100%;display:block;cursor:zoom-in}
.cols td.img{width:${viewer ? 23 : 31}%}.num{white-space:nowrap;font-variant-numeric:tabular-nums}.bad{color:var(--bad)}.ok{color:var(--ok)}
.note{color:var(--muted);max-width:40ch}.summary{color:var(--fg);max-width:90ch}
</style></head><body>
<h1>${esc(title)}: v1.2.0 | V2</h1>
<p>${esc(v1.project.w)}×${esc(v1.project.h)} at ${esc(fps)} fps, ${rows.length} samples. v1: ${esc(v1.source)}. V2: ${esc(v2.source ?? 'missing')}.
Visible = share of pixels whose 3×3-averaged colour still differs by more than ${THRESHOLD}/255 in a channel. Heat: black = same, blue → red → yellow = larger. Click an image for full size.${viewer ? ` The last column is ${esc(viewer.source)}: for looking, not measured.` : ''}</p>
${summary.map((p) => `<p class="summary">${esc(p)}</p>`).join('\n')}
<h2>Ranked by visible difference</h2>
<table><tr><th>Time</th><th>Visible</th><th>Mean</th><th>Cause or accepted reason</th></tr>
${ranked.map((r) => `<tr><td class="num"><a href="#${r.label}">${tc(r.time, fps)} (${r.time.toFixed(3)} s)</a></td><td class="num ${r.visible > 0.002 ? 'bad' : 'ok'}">${pct(r.visible)}</td><td class="num">${r.mean.toFixed(2)}</td><td class="note">${esc(r.note ?? '')}</td></tr>`).join('\n')}
${rows.filter((r) => r.missing).map((r) => `<tr><td class="num">${r.time.toFixed(3)} s</td><td class="bad" colspan="3">${esc(r.missing)}</td></tr>`).join('\n')}
</table>
<h2>Frames</h2>
<table class="cols"><tr><th>Time</th><th>v1.2.0</th><th>V2</th><th>Difference</th>${viewer ? '<th>V2 viewer, paused (screen)</th>' : ''}</tr>
${rows.map((r) => `<tr id="${r.label}"><td class="num">${tc(r.time, fps)}<br>${r.time.toFixed(3)} s<br><span class="${r.visible > 0.002 ? 'bad' : 'ok'}">${pct(r.visible)}</span><div class="note">${esc(r.note ?? r.missing ?? '')}</div></td>
<td class="img"><a href="${r.v1}"><img loading="lazy" src="${r.v1}" alt="v1 ${r.time}"></a></td>
<td class="img">${r.v2 ? `<a href="${r.v2}"><img loading="lazy" src="${r.v2}" alt="V2 ${r.time}"></a>` : ''}</td>
<td class="img">${r.diff ? `<a href="${r.diff}"><img loading="lazy" src="${r.diff}" alt="difference ${r.time}"></a>` : ''}</td>${viewer ? `<td class="img">${r.viewer ? `<a href="${r.viewer}"><img loading="lazy" src="${r.viewer}" alt="V2 viewer ${r.time}"></a>` : ''}</td>` : ''}</tr>`).join('\n')}
</table></body></html>`;
writeFileSync(path.join(dir, 'index.html'), html);
writeFileSync(path.join(dir, 'ranked.json'), JSON.stringify(ranked.map(({ time, label, visible, mean, max, note }) => ({ time, label, visible, mean, max, note })), null, 1));
console.log(path.join(dir, 'index.html'));
