/**
 * Writes tests/vectors/timecode-vectors.json by running the editor's own
 * timecode helpers (apps/desktop/src/renderer/src/legacy/core/timecode.ts)
 * under bun. pm-time must format, parse, and pick label bases exactly as they
 * do.
 *
 *   bun engine/crates/pm-time/tests/vectors/generate-timecode-vectors.ts
 *
 * Deterministic: a seeded PRNG picks the random frames.
 */
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { formatDropFrameTimecode, parseDropFrameTimecode, timecodeBase } from '../../../../../apps/desktop/src/renderer/src/legacy/core/timecode';

let seed = 0x2545f491;
const random = () => {
  seed ^= seed << 13; seed >>>= 0; seed ^= seed >>> 17; seed ^= seed << 5; seed >>>= 0;
  return seed / 2 ** 32;
};
const int = (lo: number, hi: number) => lo + Math.floor(random() * (hi - lo + 1));

const dropFrame = [{ fps: 30000 / 1001, num: 30000, den: 1001 }, { fps: 60000 / 1001, num: 60000, den: 1001 }].map(({ fps, num, den }) => {
  const frames = new Set<number>();
  for (const f of [0, 1, 1799, 1800, 1801, 3599, 3600, 3601, 17981, 17982, 17983, 35963, 35964, 35965, 2589407, 2589408, 5178815, 5178816, -1, -2, -1800, 1e6, 1e7, 1e8, -1e8]) frames.add(f);
  while (frames.size < 2000) frames.add(int(-1e8, 1e8));
  const format = [...frames].sort((a, b) => a - b).map(frame => ({ frame, text: formatDropFrameTimecode(frame, fps) }));
  const texts = new Set<string>();
  for (const { text } of format) texts.add(text);
  for (const t of ['00:01:00;00', '00:01:00;01', '00:01:00;02', '00:01:00;03', '00:01:00;04', '00:10:00;00', '00:09:00;01', '23:59:59;29', '23:59:59;59', '24:00:00;00', '00:60:00;00', '00:00:60;00', '00:00:00;30', '00:00:00;60', '00:00:00:00', '00:01:00.02', '00:01:00,02', '00:01:00.00', '0:00:00;00', '00:00:00;000', 'aa:00:00;00', '']) texts.add(t);
  while (texts.size < 4000) {
    const pad = (n: number) => String(n).padStart(2, '0');
    texts.add(`${pad(int(0, 25))}:${pad(int(0, 61))}:${pad(int(0, 61))}${[';', ';', ';', '.', ','][int(0, 4)]}${pad(int(0, 61))}`);
  }
  const parse = [...texts].sort().map(text => ({ text, frame: parseDropFrameTimecode(text, fps) }));
  return { num, den, format, parse };
});

const rates: Array<[number, number]> = [[24000, 1001], [24, 1], [25, 1], [30000, 1001], [30, 1], [48000, 1001], [48, 1], [50, 1], [60000, 1001], [60, 1], [90, 1], [100, 1], [120000, 1001], [120, 1], [240000, 1001], [240, 1], [25, 2], [7, 3], [5, 2], [2, 5], [1, 3], [15, 1], [2997, 100]];
const bases = rates.map(([num, den]) => ({ num, den, base: timecodeBase(num / den) }));

const out = path.join(import.meta.dir, 'timecode-vectors.json');
writeFileSync(out, JSON.stringify({ generatedBy: 'engine/crates/pm-time/tests/vectors/generate-timecode-vectors.ts', dropFrame, bases }) + '\n');
console.log(`wrote ${dropFrame.reduce((n, d) => n + d.format.length + d.parse.length, 0)} drop-frame vectors and ${bases.length} bases`);
