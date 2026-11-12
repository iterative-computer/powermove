// Reference frames from a delivered video (README.md): what was actually
// exported and posted, at the sheet's sample times.
//
//   node tools/visual-parity/ref-frames.mjs --video <file> --out <dir> [--offset 0]
//
// Sample times come from <out>/v1/manifest.json, or <out>/v2/manifest.json
// without v1; project time t is video time t + offset, taken as the video
// frame round((t + offset) * rate). The frames one before and one after go to
// ref-prev/ and ref-next/, so the sheet can tell a timing difference from a
// content one. Samples past the video's end are left out. Decoding assumes
// BT.709 video range (the tags of an untagged H.264 file). Writes
// <out>/ref/<time>.png and <out>/ref/manifest.json.
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const argv = process.argv.slice(2);
const opt = {};
for (let i = 0; i < argv.length; i += 2) opt[argv[i].replace(/^--/, '')] = argv[i + 1];
const offset = Number(opt.offset ?? 0);
const root = path.resolve(opt.out);
const timesFrom = ['v1', 'v2'].map((d) => path.join(root, d, 'manifest.json')).find(existsSync);
if (!timesFrom) throw new Error('no v1/ or v2/ manifest to take sample times from');
const base = JSON.parse(readFileSync(timesFrom, 'utf8'));
const probe = JSON.parse(execFileSync('ffprobe', ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=r_frame_rate,nb_frames,width,height', '-show_entries', 'format=duration', '-of', 'json', opt.video], { encoding: 'utf8' }));
const [num, den] = probe.streams[0].r_frame_rate.split('/').map(Number);
const rate = num / den;
const count = Number(probe.streams[0].nb_frames) || Math.floor(Number(probe.format.duration) * rate);
const wanted = base.samples
  .map((s) => ({ ...s, frame: Math.round((s.time + offset) * rate) }))
  .filter((s) => s.frame >= 0 && s.frame < count);

// Frames `shift` from each sample's into <out>/<name>/.
function extract(name, shift) {
  const out = path.join(root, name);
  mkdirSync(out, { recursive: true });
  const at = (s) => Math.min(count - 1, Math.max(0, s.frame + shift));
  const unique = [...new Set(wanted.map(at))].sort((a, b) => a - b);
  const tmp = path.join(out, '.extract');
  rmSync(tmp, { recursive: true, force: true });
  mkdirSync(tmp);
  execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-i', opt.video,
    '-vf', `select='${unique.map((n) => `eq(n,${n})`).join('+')}',scale=${base.project.w}:${base.project.h}:in_color_matrix=bt709:in_range=tv:out_range=pc:flags=accurate_rnd+full_chroma_int+full_chroma_inp+lanczos,format=rgba`,
    '-fps_mode', 'passthrough', path.join(tmp, '%04d.png')]);
  const samples = wanted.map((s) => {
    const file = s.file.replace(/^.*\//, '');
    copyFileSync(path.join(tmp, `${String(unique.indexOf(at(s)) + 1).padStart(4, '0')}.png`), path.join(out, file));
    return { time: s.time, frame: at(s), videoTime: at(s) / rate, file };
  });
  rmSync(tmp, { recursive: true, force: true });
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ source: `${path.basename(opt.video)} (${probe.streams[0].width}x${probe.streams[0].height}, ${rate} fps), frame round((t + ${offset}) × ${rate})${shift ? ` ${shift > 0 ? '+' : ''}${shift}` : ''}`, samples }, null, 1));
  return samples.length;
}
const n = extract('ref', 0);
extract('ref-prev', -1);
extract('ref-next', 1);
console.log(`${n} reference frames (and their neighbours) of ${base.samples.length} samples`);
