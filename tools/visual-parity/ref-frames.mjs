// Reference frames from a delivered video (README.md): what was actually
// exported and posted, at the sheet's sample times.
//
//   node tools/visual-parity/ref-frames.mjs --video <file> --out <dir> [--offset 0]
//
// Sample times come from <out>/v1/manifest.json; project time t is video
// time t + offset, taken as the video frame round((t + offset) * rate).
// Samples past the video's end are left out. Decoding assumes BT.709 video
// range (the tags of an untagged H.264 file). Writes <out>/ref/<time>.png and
// <out>/ref/manifest.json.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const argv = process.argv.slice(2);
const opt = {};
for (let i = 0; i < argv.length; i += 2) opt[argv[i].replace(/^--/, '')] = argv[i + 1];
const offset = Number(opt.offset ?? 0);
const out = path.resolve(opt.out, 'ref');
mkdirSync(out, { recursive: true });
const v1 = JSON.parse(readFileSync(path.resolve(opt.out, 'v1/manifest.json'), 'utf8'));
const probe = JSON.parse(execFileSync('ffprobe', ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=r_frame_rate,nb_frames,width,height', '-show_entries', 'format=duration', '-of', 'json', opt.video], { encoding: 'utf8' }));
const [num, den] = probe.streams[0].r_frame_rate.split('/').map(Number);
const rate = num / den;
const count = Number(probe.streams[0].nb_frames) || Math.floor(Number(probe.format.duration) * rate);
const wanted = v1.samples
  .map((s) => ({ ...s, frame: Math.round((s.time + offset) * rate) }))
  .filter((s) => s.frame >= 0 && s.frame < count);
const unique = [...new Set(wanted.map((s) => s.frame))].sort((a, b) => a - b);
const tmp = path.join(out, '.extract');
rmSync(tmp, { recursive: true, force: true });
mkdirSync(tmp);
execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-i', opt.video,
  '-vf', `select='${unique.map((n) => `eq(n,${n})`).join('+')}',scale=${v1.project.w}:${v1.project.h}:in_color_matrix=bt709:in_range=tv:out_range=pc:flags=accurate_rnd+full_chroma_int+full_chroma_inp+lanczos,format=rgba`,
  '-fps_mode', 'passthrough', path.join(tmp, '%04d.png')]);
const samples = wanted.map((s) => {
  copyFileSync(path.join(tmp, `${String(unique.indexOf(s.frame) + 1).padStart(4, '0')}.png`), path.join(out, s.file));
  return { time: s.time, frame: s.frame, videoTime: s.frame / rate, file: s.file };
});
rmSync(tmp, { recursive: true, force: true });
writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ source: `${path.basename(opt.video)} (${probe.streams[0].width}x${probe.streams[0].height}, ${rate} fps), frame round((t + ${offset}) × ${rate})`, samples }, null, 1));
console.log(`${samples.length} reference frames of ${v1.samples.length} samples`);
