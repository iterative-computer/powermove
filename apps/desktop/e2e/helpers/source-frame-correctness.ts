/** Untimed original-media correctness for the source performance campaign.
 *
 * A native delivery stamp proves which REQUEST reached the screen, not which
 * picture. Before any timing is accepted, one untimed native session seeks
 * every frozen target of every imported fixture, waits for the delivery to be
 * acknowledged and presented, then copies the frame the monitor's video
 * element holds (VideoFrame.copyTo: no canvas or colour conversion). FFmpeg
 * independently decodes the same file, and the capture is compared with the
 * intended frame and both neighbours on a fixed grid of cells: Y'CbCr sample
 * means for decoded planes, linear-light sRGB means for frames the engine
 * converted itself. A neighbour whose decoded picture is identical to the intended
 * one is the same picture; a distinct neighbour must be decisively farther
 * away than the intended frame, or the match is ambiguous and fails.
 *
 * The evidence is bound to the campaign id, the app build, every fixture hash,
 * target list and monitor geometry; sourcePerformanceGate rejects a campaign
 * whose evidence is missing, failing, or for anything else.
 */
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { linearToSrgbPrimaries, readSdrVideoTags, srgbEotf, type VideoTags } from './source-colour-reference';

/** Explicit tolerances, in 8-bit codes of a cell mean. */
export const FRAME_MATCH = {
  /** Intended frame, decoded planes (NV12, I420A, I422, I420P10) against
   * FFmpeg's decode as Y'CbCr means (10-bit scaled to 8-bit codes): mean over
   * every cell and channel, and the 99th percentile of a cell's worst channel.
   * Conformant H.264/HEVC decoders are bit-exact; only ProRes-class decoders
   * may differ by a code. */
  planes: { columns: 160, meanCodes: 0.5, p99Codes: 2 },
  /** Intended frame, engine-converted RGBA (resampled to the monitor) as
   * linear-light sRGB means. The engine's resampling kernel is not a box
   * mean, so colour is judged on cells the reference shows flat (linear
   * standard deviation at most `flatSpread`) with no step above `edgeStep`
   * to a neighbouring cell: at a hard edge the kernels
   * legitimately differ, and near black the sRGB curve magnifies that. Every
   * cell still counts for frame identity. Colour accuracy itself is gated
   * against the f64 reference by engine-source-compositor. */
  converted: { columns: 40, meanCodes: 1.5, p99Codes: 6, flatSpread: .02, edgeStep: .1 },
  /** A cell discriminates a neighbour when the two decoded pictures differ there by this much. */
  distinctCodes: 8,
  /** Fewer discriminating cells than this is an ambiguous neighbour. */
  minDistinctCells: 16,
  /** On discriminating cells the neighbour's error must be this many times the intended frame's... */
  separationRatio: 3,
  /** ...and larger by at least this many codes. */
  separationCodes: 4
} as const;

/** Cell means of an RGB(A) picture, as sRGB codes (three per cell, row-major).
 * Self-contained: page.evaluate installs this same function in the window.
 * `lut` maps a sample code to linear light; `matrix` (row-major, or null)
 * changes linear primaries into sRGB's. Each pixel is clipped to the display
 * range before averaging, as the monitor clips it before it is seen. */
export function sourceCellGrid(samples: ArrayLike<number>, width: number, height: number, channels: number,
  lut: ArrayLike<number>, matrix: ArrayLike<number> | null, columns: number, spread?: number[]): number[] {
  const rows = Math.max(1, Math.round(columns * height / width));
  const sums = new Float64Array(columns * rows * 3), counts = new Float64Array(columns * rows);
  const squares = spread ? new Float64Array(columns * rows * 3) : null;
  const column = new Int32Array(width), row = new Int32Array(height);
  for (let x = 0; x < width; x++) column[x] = Math.min(columns - 1, Math.floor((x + .5) * columns / width));
  for (let y = 0; y < height; y++) row[y] = Math.min(rows - 1, Math.floor((y + .5) * rows / height));
  const clip = (v: number) => v < 0 ? 0 : v > 1 ? 1 : v;
  for (let y = 0; y < height; y++) {
    const base = row[y]! * columns;
    for (let x = 0, at = y * width * channels; x < width; x++, at += channels) {
      let r = lut[samples[at]!]!, g = lut[samples[at + 1]!]!, b = lut[samples[at + 2]!]!;
      if (matrix) {
        const [mr, mg, mb] = [matrix[0]! * r + matrix[1]! * g + matrix[2]! * b,
          matrix[3]! * r + matrix[4]! * g + matrix[5]! * b, matrix[6]! * r + matrix[7]! * g + matrix[8]! * b];
        r = mr; g = mg; b = mb;
      }
      const cell = base + column[x]!;
      r = clip(r); g = clip(g); b = clip(b);
      sums[cell * 3] += r; sums[cell * 3 + 1] += g; sums[cell * 3 + 2] += b;
      if (squares) { squares[cell * 3] += r * r; squares[cell * 3 + 1] += g * g; squares[cell * 3 + 2] += b * b; }
      counts[cell]++;
    }
  }
  const encode = (v: number) => 255 * (v <= .0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - .055);
  const codes: number[] = new Array(sums.length);
  for (let i = 0; i < sums.length; i++) codes[i] = encode(sums[i]! / counts[Math.floor(i / 3)]!);
  // Optional per-cell linear standard deviation (worst channel).
  if (spread && squares) for (let cell = 0; cell < counts.length; cell++) {
    let worst = 0;
    for (let k = 0; k < 3; k++) {
      const mean = sums[cell * 3 + k]! / counts[cell]!;
      worst = Math.max(worst, Math.sqrt(Math.max(0, squares[cell * 3 + k]! / counts[cell]! - mean * mean)));
    }
    spread[cell] = worst;
  }
  return codes;
}

/** sRGB EOTF for 8-bit display codes (the browser capture's samples). */
export const SRGB8_LINEAR = Array.from({ length: 256 }, (_, code) => srgbEotf(code / 255));

export type DecodedFrame = { frame: number; sha256: string; codes: number[]; flat?: boolean[] };
export type FrameTolerance = { columns: number; meanCodes: number; p99Codes: number; flatSpread?: number; edgeStep?: number };
export type GridPlane = { data: ArrayLike<number>; offset: number; stride: number; step: number; width: number; height: number };

/** Cell means of three Y'CbCr planes (any subsampling), as `scale`d codes
 * (three per cell, row-major). Self-contained: also installed in the window.
 * A sample belongs to the cell holding its centre in picture coordinates. */
export function sourcePlaneGrid(planes: GridPlane[], width: number, height: number, scale: number, cells: number): number[] {
  // No finer than the smallest plane, so every cell holds every channel.
  const columns = Math.max(1, Math.min(cells, ...planes.map(plane => plane.width)));
  const rows = Math.max(1, Math.min(Math.round(columns * height / width), ...planes.map(plane => plane.height)));
  const sums = new Float64Array(columns * rows * 3), counts = new Float64Array(columns * rows * 3);
  for (let channel = 0; channel < 3; channel++) {
    const plane = planes[channel]!;
    const column = new Int32Array(plane.width);
    for (let x = 0; x < plane.width; x++) column[x] = Math.min(columns - 1, Math.floor((x + .5) * columns / plane.width));
    for (let y = 0; y < plane.height; y++) {
      const base = Math.min(rows - 1, Math.floor((y + .5) * rows / plane.height)) * columns;
      for (let x = 0, at = plane.offset + y * plane.stride; x < plane.width; x++, at += plane.step) {
        const cell = (base + column[x]!) * 3 + channel;
        sums[cell] += plane.data[at]!; counts[cell]++;
      }
    }
  }
  const codes: number[] = new Array(sums.length);
  for (let i = 0; i < sums.length; i++) codes[i] = scale * sums[i]! / counts[i]!;
  return codes;
}

/** The planes of a tight planar FFmpeg frame (`yuv4xxp` or `yuv4xxp10le`). */
export function planarFramePlanes(data: ArrayLike<number>, width: number, height: number, format: string): { planes: GridPlane[]; scale: number } {
  const match = /^yuv(420|422|444)p(10le)?$/.exec(format);
  if (!match) throw new Error(`Unsupported planar reference format ${format}`);
  const cw = match[1] === '444' ? width : Math.ceil(width / 2), ch = match[1] === '420' ? Math.ceil(height / 2) : height;
  const plane = (offset: number, w: number, h: number): GridPlane => ({ data, offset, stride: w, step: 1, width: w, height: h });
  return { planes: [plane(0, width, height), plane(width * height, cw, ch), plane(width * height + cw * ch, cw, ch)], scale: match[2] ? .25 : 1 };
}

/** Independently decode every presented frame of `file` with FFmpeg, in
 * presentation order, and grid the requested ordinals. The stream's matrix and
 * range come from its own tags (section 9 reading), never from the engine. */
export async function decodedFrameGrids(ffmpeg: string, file: string, frames: number, wanted: Set<number>, mode: 'planes' | 'converted') {
  const { stdout } = await new Promise<{ stdout: string }>((resolve, reject) => {
    const probe = spawn(ffmpeg.replace(/ffmpeg$/, 'ffprobe'), ['-v', 'error', '-select_streams', 'v:0', '-show_entries',
      'stream=width,height,pix_fmt,color_range,color_space,color_transfer,color_primaries,sample_aspect_ratio', '-of', 'json', file]);
    let out = '', err = '';
    probe.stdout.on('data', chunk => { out += chunk; });
    probe.stderr.on('data', chunk => { err += chunk; });
    probe.on('error', reject);
    probe.on('close', code => code === 0 ? resolve({ stdout: out }) : reject(new Error(`ffprobe ${code}: ${err}`)));
  });
  const stream = JSON.parse(stdout).streams?.[0];
  if (!stream) throw new Error('Correctness reference has no video stream');
  if (stream.sample_aspect_ratio && !['1:1', '0:1', 'N/A'].includes(stream.sample_aspect_ratio))
    throw new Error('Correctness reference fixtures must have square pixels');
  const tags: VideoTags = readSdrVideoTags(stream);
  const width = Number(stream.width), height = Number(stream.height);
  const pixFmt = String(stream.pix_fmt);
  const planar = mode === 'planes' ? planarFramePlanes(new Uint8Array(0), width, height, pixFmt) : null;
  const sampleBytes = planar && planar.scale !== 1 ? 2 : 1;
  const frameBytes = planar
    ? (planar.planes.reduce((sum, plane) => sum + plane.width * plane.height, 0)) * sampleBytes
    : width * height * 6;
  const lut = Float64Array.from({ length: 65536 }, (_, code) => srgbEotf(code / 65535));
  const matrix = linearToSrgbPrimaries(tags.primaries);
  const decoded: DecodedFrame[] = [];
  let count = 0;
  await new Promise<void>((resolve, reject) => {
    // Matrix and range from the tags; RGB48 keeps 10-bit sources' precision.
    // Planes mode keeps the decoder's own samples: no scaling or conversion.
    const filter = planar ? [] : ['-vf', `scale=in_color_matrix=${tags.matrix}:in_range=${tags.fullRange ? 'pc' : 'tv'}:flags=accurate_rnd+full_chroma_int+bitexact,format=rgb48le`];
    const decoder = spawn(ffmpeg, ['-v', 'error', '-nostdin', '-threads', '2', '-i', file, '-map', '0:v:0',
      '-fps_mode', 'passthrough', ...filter, '-f', 'rawvideo', 'pipe:1']);
    let pending = Buffer.alloc(frameBytes), filled = 0, err = '', failed: Error | null = null;
    decoder.stdout.on('data', (chunk: Buffer) => {
      for (let offset = 0; offset < chunk.length && !failed;) {
        const take = Math.min(chunk.length - offset, frameBytes - filled);
        chunk.copy(pending, filled, offset, offset + take);
        filled += take; offset += take;
        if (filled < frameBytes) continue;
        if (wanted.has(count)) {
          const sha256 = createHash('sha256').update(pending).digest('hex');
          const samples = sampleBytes === 2 || !planar ? new Uint16Array(pending.buffer, pending.byteOffset, frameBytes / 2) : pending;
          const spread: number[] = [];
          const codes = planar
            ? (() => { const { planes, scale } = planarFramePlanes(samples, width, height, pixFmt); return sourcePlaneGrid(planes, width, height, scale, FRAME_MATCH.planes.columns); })()
            : sourceCellGrid(samples, width, height, 3, lut, matrix, FRAME_MATCH.converted.columns, spread);
          decoded.push({ frame: count, sha256, codes, ...(planar ? {} : { flat: flatCells(codes, spread, FRAME_MATCH.converted.columns) }) });
          pending = Buffer.alloc(frameBytes);
        }
        filled = 0; count++;
      }
    });
    decoder.stderr.on('data', chunk => { err += chunk; });
    decoder.on('error', reject);
    decoder.on('close', code => {
      if (code !== 0) reject(new Error(`ffmpeg ${code}: ${err}`));
      else if (filled) reject(new Error('FFmpeg ended inside a frame'));
      else resolve();
    });
  });
  if (count !== frames) throw new Error(`FFmpeg decoded ${count} frames; the indexed timeline has ${frames}`);
  return { width, height, tags, frames: new Map(decoded.map(frame => [frame.frame, frame])) };
}

/** Cells flat in the reference with no hard step to their four neighbours
 * (no edge inside or at the border a resampling kernel could reach across). */
function flatCells(codes: number[], spread: number[], columns: number): boolean[] {
  const rows = codes.length / 3 / columns, { flatSpread, edgeStep } = FRAME_MATCH.converted;
  const linear = codes.map(code => srgbEotf(code / 255));
  const level = (a: number, b: number) => [0, 1, 2].every(k => Math.abs(linear[a * 3 + k]! - linear[b * 3 + k]!) <= edgeStep);
  return spread.map((value, cell) => {
    const x = cell % columns, y = Math.floor(cell / columns);
    return value <= flatSpread && [[x - 1, y], [x + 1, y], [x, y - 1], [x, y + 1]]
      .every(([nx, ny]) => nx! < 0 || ny! < 0 || nx! >= columns || ny! >= rows || level(cell, ny! * columns + nx!));
  });
}

const percentile = (values: number[], p: number) => [...values].sort((a, b) => a - b)[Math.round((values.length - 1) * p)] ?? 0;
const cellErrors = (a: number[], b: number[]) => Array.from({ length: a.length / 3 }, (_, cell) =>
  Math.max(Math.abs(a[cell * 3]! - b[cell * 3]!), Math.abs(a[cell * 3 + 1]! - b[cell * 3 + 1]!), Math.abs(a[cell * 3 + 2]! - b[cell * 3 + 2]!)));

export type FrameJudgement = {
  intended: number; verdict: 'match' | 'mismatch' | 'ambiguous'; reasons: string[];
  error: { mean: number; p99: number; max: number; cells: number };
  neighbours: { frame: number; identical: boolean; distinctCells: number; intendedError: number; neighbourError: number }[];
};

/** Classify one captured grid against the intended frame and its neighbours. */
export function judgeSourceFrame(capture: number[], intended: DecodedFrame, neighbours: DecodedFrame[], tolerance: FrameTolerance): FrameJudgement {
  if (capture.length !== intended.codes.length) throw new Error('Capture and reference grids differ in shape');
  if (![capture, intended.codes, ...neighbours.map(n => n.codes)].every(codes => codes.every(Number.isFinite)))
    throw new Error('A frame grid has an empty or non-finite cell');
  const reasons: string[] = [];
  // Colour is judged on the cells the reference marks flat (all, for planes).
  const judged = (cell: number) => !intended.flat || intended.flat[cell]!;
  const all = cellErrors(capture, intended.codes).filter((_, cell) => judged(cell));
  if (all.length < capture.length / 6) throw new Error('Too few flat cells to judge colour');
  let sum = 0;
  for (let i = 0; i < capture.length; i++) if (judged(Math.floor(i / 3))) sum += Math.abs(capture[i]! - intended.codes[i]!);
  const error = { mean: sum / (all.length * 3), p99: percentile(all, .99), max: Math.max(...all), cells: all.length };
  if (error.mean > tolerance.meanCodes) reasons.push(`mean ${error.mean.toFixed(3)} > ${tolerance.meanCodes}`);
  if (error.p99 > tolerance.p99Codes) reasons.push(`p99 ${error.p99.toFixed(3)} > ${tolerance.p99Codes}`);
  let ambiguous = false;
  const near = neighbours.map(neighbour => {
    const identical = neighbour.sha256 === intended.sha256;
    const apart = cellErrors(intended.codes, neighbour.codes);
    const toIntended = cellErrors(capture, intended.codes), toNeighbour = cellErrors(capture, neighbour.codes);
    let distinctCells = 0, intendedError = 0, neighbourError = 0;
    for (let cell = 0; cell < apart.length; cell++) {
      if (apart[cell]! < FRAME_MATCH.distinctCodes) continue;
      distinctCells++; intendedError += toIntended[cell]!; neighbourError += toNeighbour[cell]!;
    }
    if (distinctCells) { intendedError /= distinctCells; neighbourError /= distinctCells; }
    if (!identical) {
      if (distinctCells < FRAME_MATCH.minDistinctCells) {
        ambiguous = true;
        reasons.push(`frame ${neighbour.frame} differs from the intended picture in only ${distinctCells} cells`);
      } else if (neighbourError < FRAME_MATCH.separationRatio * intendedError || neighbourError - intendedError < FRAME_MATCH.separationCodes) {
        ambiguous = true;
        reasons.push(`frame ${neighbour.frame} is not decisively farther (${neighbourError.toFixed(3)} vs ${intendedError.toFixed(3)})`);
      }
    }
    return { frame: neighbour.frame, identical, distinctCells, intendedError, neighbourError };
  });
  const verdict = error.mean > tolerance.meanCodes || error.p99 > tolerance.p99Codes ? 'mismatch' : ambiguous ? 'ambiguous' : 'match';
  return { intended: intended.frame, verdict, reasons, error, neighbours: near };
}

/** What the timed campaign measured, which the correctness evidence must cover exactly. */
export type CampaignFixture = { workload: string; name: string; sha256: string; targets: number[]; monitor: readonly [number, number] };
export type SourceFrameCorrectness = {
  campaign: string; build: string; tolerances: typeof FRAME_MATCH; pass: boolean;
  fixtures: (CampaignFixture & { captured: { width: number; height: number; format: string } | null; error: string | null;
    targets: number[]; judgements: (FrameJudgement & { target: number; timestamp: number })[] })[];
};

/** Acceptance failures for the campaign's correctness evidence. */
export function correctnessFailures(evidence: SourceFrameCorrectness | null, campaign: string, build: string, fixtures: CampaignFixture[]): string[] {
  if (!evidence) return ['original-media correctness: no evidence for this campaign'];
  const failures: string[] = [];
  if (evidence.campaign !== campaign) failures.push('original-media correctness: evidence is for another campaign');
  if (evidence.build !== build) failures.push('original-media correctness: evidence is for another build');
  if (JSON.stringify(evidence.tolerances) !== JSON.stringify(FRAME_MATCH)) failures.push('original-media correctness: tolerances differ');
  for (const fixture of fixtures) {
    const found = evidence.fixtures.find(entry => entry.name === fixture.name);
    if (!found) { failures.push(`original-media correctness: ${fixture.name} unchecked`); continue; }
    if (found.sha256 !== fixture.sha256 || found.workload !== fixture.workload
        || JSON.stringify(found.targets) !== JSON.stringify(fixture.targets) || JSON.stringify(found.monitor) !== JSON.stringify(fixture.monitor))
      failures.push(`original-media correctness: ${fixture.name} evidence differs from the measured fixture, targets or geometry`);
    if (found.error) failures.push(`original-media correctness: ${fixture.name}: ${found.error}`);
    if (found.judgements.length !== fixture.targets.length
        || found.judgements.some((judgement, i) => judgement.target !== fixture.targets[i]))
      failures.push(`original-media correctness: ${fixture.name} judged ${found.judgements.length} of ${fixture.targets.length} targets`);
    for (const judgement of found.judgements)
      if (judgement.verdict !== 'match')
        failures.push(`original-media correctness: ${fixture.name} at ${judgement.target}: ${judgement.verdict} (${judgement.reasons.join('; ')})`);
  }
  const judgedPass = evidence.fixtures.every(entry => !entry.error && entry.judgements.every(judgement => judgement.verdict === 'match'));
  if (judgedPass !== evidence.pass) failures.push('original-media correctness: recorded pass disagrees with its judgements');
  return failures;
}

/** The app build under measurement: native addons plus every bundled file. */
export async function sourceBuildIdentity(desktop: string): Promise<string> {
  const hash = createHash('sha256');
  const files: string[] = [path.join(desktop, 'native', 'pm_node.node'), path.join(desktop, 'native', 'pm_bridge.node')];
  const walk = async (directory: string) => {
    for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) await walk(full); else files.push(full);
    }
  };
  await walk(path.join(desktop, 'out'));
  for (const file of files) hash.update(path.relative(desktop, file)).update('\0').update(await readFile(file)).update('\0');
  return hash.digest('hex');
}
