import { describe, expect, it } from 'vitest';
import { correctnessFailures, FRAME_MATCH, judgeSourceFrame, planarFramePlanes, sourceCellGrid, sourcePlaneGrid, SRGB8_LINEAR, type DecodedFrame, type SourceFrameCorrectness } from './source-frame-correctness';
import { linearToSrgbPrimaries } from './source-colour-reference';

const COLUMNS = 160, cells = COLUMNS * 90; // a 16:9 grid for synthetic pictures
const picture = (code: (cell: number) => number) => Array.from({ length: cells * 3 }, (_, i) => code(Math.floor(i / 3)));
const frame = (frame: number, codes: number[], sha256 = `f${frame}`): DecodedFrame => ({ frame, codes, sha256 });
// A bar that moves 8 cells per frame, as testsrc2's elements do.
const moving = (index: number) => picture(cell => (cell % COLUMNS) >= index * 8 && (cell % COLUMNS) < index * 8 + 8 ? 220 : 40);

describe('sourceCellGrid', () => {
  it('averages in linear light and keeps an exact code for a uniform picture', () => {
    const rgba = new Uint8Array(320 * 180 * 4).fill(128);
    const codes = sourceCellGrid(rgba, 320, 180, 4, SRGB8_LINEAR, null, COLUMNS);
    expect(codes).toHaveLength(cells * 3);
    for (const code of codes) expect(code).toBeCloseTo(128, 9);
    // Half black, half white pixels in each cell: the linear mean, not code 127.5.
    const checker = new Uint8Array(2 * 2 * 3).map((_, i) => Math.floor(i / 3) % 2 ? 255 : 0);
    expect(sourceCellGrid(checker, 2, 2, 3, SRGB8_LINEAR, null, 1)[0]).toBeCloseTo(187.5, 0);
  });

  it('changes primaries and clips each pixel to the display range before averaging', () => {
    const matrix = linearToSrgbPrimaries('smpte170m')!;
    expect(matrix).toHaveLength(9);
    // Rows of a primaries change between D65 spaces keep white white.
    for (let row = 0; row < 3; row++) expect(matrix[row * 3]! + matrix[row * 3 + 1]! + matrix[row * 3 + 2]!).toBeCloseTo(1, 6);
    const lut = [0, 2, 1]; // samples index this; 2 means superwhite linear light
    const [r] = sourceCellGrid([1, 1, 1, 2, 2, 2], 2, 1, 3, lut, null, 1);
    expect(r).toBeCloseTo(255, 6);
    expect(linearToSrgbPrimaries('bt709')).toBeNull();
  });
});

describe('sourcePlaneGrid', () => {
  it('averages each plane by sample centre, whatever its subsampling or interleaving', () => {
    // 4x2 yuv420p10le: two luma columns per cell, one chroma sample per cell.
    const data = new Uint16Array([400, 400, 800, 800, 400, 400, 800, 800, 512, 600, 300, 700]);
    const { planes, scale } = planarFramePlanes(data, 4, 2, 'yuv420p10le');
    expect(sourcePlaneGrid(planes, 4, 2, scale, 2)).toEqual([100, 128, 75, 200, 150, 175]);
    // The same picture as interleaved 8-bit NV12.
    const nv12 = new Uint8Array([100, 100, 200, 200, 100, 100, 200, 200, 128, 75, 150, 175]);
    const plane = (offset: number, w: number, h: number, step: number, stride: number) => ({ data: nv12, offset, stride, step, width: w, height: h });
    expect(sourcePlaneGrid([plane(0, 4, 2, 1, 4), plane(8, 2, 1, 2, 4), plane(9, 2, 1, 2, 4)], 4, 2, 1, 2)).toEqual([100, 128, 75, 200, 150, 175]);
  });
});

describe('judgeSourceFrame', () => {
  const intended = frame(5, moving(5)), before = frame(4, moving(4)), after = frame(6, moving(6));
  it('matches the intended frame within the colour tolerance', () => {
    const capture = intended.codes.map(code => code + .4);
    const judgement = judgeSourceFrame(capture, intended, [before, after], FRAME_MATCH.converted);
    expect(judgement.verdict).toBe('match');
    expect(judgement.neighbours.every(n => n.distinctCells >= FRAME_MATCH.minDistinctCells)).toBe(true);
  });

  it('rejects a neighbouring frame as a mismatch', () => {
    expect(judgeSourceFrame(after.codes, intended, [before, after], FRAME_MATCH.converted).verdict).toBe('mismatch');
  });

  it('rejects an unchanged colour shift beyond the tolerance', () => {
    const judgement = judgeSourceFrame(intended.codes.map(code => code + 3), intended, [before, after], FRAME_MATCH.converted);
    expect(judgement.verdict).toBe('mismatch');
    expect(judgement.reasons[0]).toMatch(/^mean 3\.000/);
  });

  it('calls a barely distinct neighbour ambiguous, but accepts an identical decoded picture', () => {
    const subtle = frame(6, intended.codes.map((code, i) => i < 3 * 4 ? code + 20 : code));
    expect(judgeSourceFrame(intended.codes, intended, [before, subtle], FRAME_MATCH.converted).verdict).toBe('ambiguous');
    const same = frame(6, [...intended.codes], intended.sha256);
    const judgement = judgeSourceFrame(intended.codes, intended, [same], FRAME_MATCH.converted);
    expect(judgement.verdict).toBe('match');
    expect(judgement.neighbours[0]).toMatchObject({ identical: true, distinctCells: 0 });
  });

  it('judges converted colour on the cells the reference marks flat, but identity on all', () => {
    const edge = intended.codes.map((code, i) => i < 3 * 300 ? code + 40 : code + .2);
    expect(judgeSourceFrame(edge, intended, [before, after], FRAME_MATCH.converted).verdict).toBe('mismatch');
    const flat = Array.from({ length: cells }, (_, cell) => cell >= 300);
    const judgement = judgeSourceFrame(edge, { ...intended, flat }, [before, after], FRAME_MATCH.converted);
    expect(judgement.verdict).toBe('match');
    expect(judgement.error.cells).toBe(cells - 300);
    expect(() => judgeSourceFrame(edge, { ...intended, flat: flat.map(() => false) }, [before, after], FRAME_MATCH.converted)).toThrow('flat cells');
  });

  it('calls a capture halfway between two frames ambiguous', () => {
    const blend = intended.codes.map((code, i) => (code + after.codes[i]!) / 2);
    expect(judgeSourceFrame(blend, intended, [before, after], FRAME_MATCH.converted).verdict).not.toBe('match');
  });
});

describe('correctnessFailures', () => {
  const fixture = { workload: 'h264-1080', name: 'h264-1080-cold-0.mp4', sha256: 'abc', targets: [.1, .2], monitor: [1280, 720] as const };
  const judged = (target: number, verdict: 'match' | 'ambiguous' = 'match') => ({ target, timestamp: 1, intended: 3, verdict, reasons: verdict === 'match' ? [] : ['close'],
    error: { mean: 0, p99: 0, max: 0, cells: 1 }, neighbours: [] });
  const evidence = (overrides: Partial<SourceFrameCorrectness> = {}): SourceFrameCorrectness => ({ campaign: 'c', build: 'b', tolerances: FRAME_MATCH, pass: true,
    fixtures: [{ ...fixture, captured: { width: 1920, height: 1080, format: 'NV12' }, error: null, judgements: [judged(.1), judged(.2)] }], ...overrides });

  it('accepts evidence that covers exactly the measured campaign', () => {
    expect(correctnessFailures(evidence(), 'c', 'b', [fixture])).toEqual([]);
  });

  it('rejects other campaigns, builds, fixtures, targets, geometry and unmatched frames', () => {
    expect(correctnessFailures(evidence(), 'other', 'b', [fixture])).toContain('original-media correctness: evidence is for another campaign');
    expect(correctnessFailures(evidence(), 'c', 'b', [fixture, { ...fixture, name: 'x.mp4' }])).toContain('original-media correctness: x.mp4 unchecked');
    expect(correctnessFailures(evidence(), 'c', 'b', [{ ...fixture, sha256: 'changed' }])[0]).toMatch(/differs from the measured fixture/);
    expect(correctnessFailures(evidence(), 'c', 'b', [{ ...fixture, monitor: [320, 180] }])[0]).toMatch(/differs from the measured fixture/);
    const failing = evidence({ pass: false, fixtures: [{ ...evidence().fixtures[0]!, judgements: [judged(.1), judged(.2, 'ambiguous')] }] });
    expect(correctnessFailures(failing, 'c', 'b', [fixture])).toEqual(['original-media correctness: h264-1080-cold-0.mp4 at 0.2: ambiguous (close)']);
    expect(correctnessFailures({ ...failing, pass: true }, 'c', 'b', [fixture])).toContain('original-media correctness: recorded pass disagrees with its judgements');
  });
});
