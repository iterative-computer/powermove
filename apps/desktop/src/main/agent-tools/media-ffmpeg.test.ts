import { describe, expect, it } from 'vitest';

import {
  TRIAGE_GRID,
  analysisArgs,
  compositionToSource,
  displaySize,
  drawtextEscape,
  fitBudget,
  frameArgs,
  parseLoudness,
  parseProbe,
  parseSilences,
  pickInformativeTimes,
  planSheet,
  resolveSampleTimes,
  sourceToComposition,
  splitTopLevel,
  tickStep,
  triageScanArgs,
  waveformArgs
} from './media-ffmpeg';

const MP4_BANNER = `Input #0, mov,mp4,m4a,3gp,3g2,mj2, from '/tmp/gen.mp4':
  Metadata:
    major_brand     : isom
    encoder         : Lavf60.3.100
  Duration: 00:00:08.00, start: 0.000000, bitrate: 114 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 6 kb/s, 29.97 fps, 29.97 tbr, 30k tbn (default)
    Metadata:
      handler_name    : VideoHandler
    Side data:
      displaymatrix: rotation of -90.00 degrees
  Stream #0:1[0x2](eng): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 97 kb/s (default)
    Metadata:
      handler_name    : SoundHandler
  Stream #0:2[0x3](eng): Data: none (tmcd / 0x64636D74), 0 kb/s
At least one output file must be specified`;

const WEBM_ALPHA = `Input #0, matroska,webm, from 'cutout.webm':
  Metadata:
    ENCODER         : Lavf60.3.100
  Duration: 00:00:02.50, start: 0.000000, bitrate: 900 kb/s
  Stream #0:0: Video: vp9 (Profile 0), yuv420p(tv, progressive), 640x360, SAR 1:1 DAR 16:9, 25 fps, 25 tbr, 1k tbn (default)
    Metadata:
      alpha_mode      : 1
      DURATION        : 00:00:02.500000000
  Stream #0:1: Audio: opus, 48000 Hz, 5.1(side), fltp (default)
At least one output file must be specified`;

const PRORES = `Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mov':
  Duration: 00:01:02.04, start: 0.000000, bitrate: 120000 kb/s
  Stream #0:0[0x1]: Video: prores (4444) (ap4h / 0x68347061), yuva444p12le(bt709, progressive), 3840x2160, 119000 kb/s, SAR 1:1 DAR 16:9, 23.98 fps, 23.98 tbr, 24k tbn (default)
  Stream #0:1[0x2]: Audio: pcm_s24le (lpcm / 0x6D63706C), 48000 Hz, 2 channels, s32 (24 bit), 2304 kb/s (default)`;

const MP3 = `Input #0, mp3, from 'voice.mp3':
  Duration: 00:12:00.50, start: 0.025057, bitrate: 128 kb/s
  Stream #0:0: Audio: mp3, 44100 Hz, mono, fltp, 128 kb/s`;

describe('probe parsing', () => {
  it('splits only top-level commas', () => {
    expect(splitTopLevel('h264 (High), yuv420p(tv, bt709), 1920x1080 [SAR 1:1 DAR 16:9], 30 fps'))
      .toEqual(['h264 (High)', 'yuv420p(tv, bt709)', '1920x1080 [SAR 1:1 DAR 16:9]', '30 fps']);
  });

  it('reads container, duration and every stream of an mp4', () => {
    const probe = parseProbe(MP4_BANNER);
    expect(probe).toMatchObject({ format: 'mov/mp4', duration: 8, start: 0, bitrateKbps: 114, otherStreams: 1 });
    expect(probe.video).toEqual([{ index: 0, codec: 'h264', profile: 'High', pixelFormat: 'yuv420p', width: 1920, height: 1080, fps: 29.97, bitrateKbps: 6, alpha: false, rotation: -90 }]);
    expect(probe.audio).toEqual([{ index: 1, codec: 'aac', profile: 'LC', sampleRate: 48000, layout: 'stereo', channels: 2, sampleFormat: 'fltp', bitrateKbps: 97, language: 'eng' }]);
    expect(displaySize(probe.video[0])).toEqual({ width: 1080, height: 1920 });
  });

  it('detects alpha from VP9 alpha_mode and from alpha pixel formats', () => {
    const webm = parseProbe(WEBM_ALPHA);
    expect(webm.format).toBe('matroska/webm');
    expect(webm.video[0]).toMatchObject({ codec: 'vp9', profile: 'Profile 0', alpha: true, width: 640, height: 360, fps: 25 });
    expect(webm.audio[0]).toMatchObject({ codec: 'opus', layout: '5.1(side)', channels: 6, sampleRate: 48000 });
    const prores = parseProbe(PRORES);
    expect(prores.duration).toBeCloseTo(62.04, 5);
    expect(prores.video[0]).toMatchObject({ codec: 'prores', profile: '4444', pixelFormat: 'yuva444p12le', alpha: true, width: 3840, height: 2160, fps: 23.98, bitrateKbps: 119000 });
    expect(prores.audio[0]).toMatchObject({ codec: 'pcm_s24le', channels: 2, layout: '2 channels', sampleRate: 48000 });
  });

  it('reads audio-only files and a non-zero start', () => {
    const probe = parseProbe(MP3);
    expect(probe).toMatchObject({ format: 'mp3', duration: 720.5, start: 0.025057, video: [] });
    expect(probe.audio[0]).toMatchObject({ codec: 'mp3', channels: 1, sampleRate: 44100 });
  });

  it('returns an empty probe for a file ffmpeg rejects', () => {
    expect(parseProbe('/tmp/x.bin: Invalid data found when processing input')).toMatchObject({ format: null, video: [], audio: [] });
  });
});

describe('frame sampling', () => {
  it('resolves explicit, negative and evenly spaced times onto real frames', () => {
    expect(resolveSampleTimes({ duration: 10, fps: 25, times: [1, -1, 10] })).toEqual([1, 9, 9.96]);
    expect(resolveSampleTimes({ duration: 10, fps: 25, count: 3 })).toEqual([0, 4.98, 9.96]);
    expect(resolveSampleTimes({ duration: 10, fps: 25, count: 2, start: 2, end: 4 })).toEqual([2, 4]);
    expect(resolveSampleTimes({ duration: 10, fps: 25, count: 1, start: 2, end: 4 })).toEqual([3]);
    expect(() => resolveSampleTimes({ duration: 10, times: [12] })).toThrow(/outside the media/);
    expect(() => resolveSampleTimes({ duration: 10, times: [-11] })).toThrow(/outside the media/);
    expect(() => resolveSampleTimes({ duration: 10, count: 2, start: 6, end: 5 })).toThrow(/window is empty/);
  });

  it('fits frames to a pixel budget without upscaling and keeps even sizes', () => {
    expect(fitBudget(1920, 1080, 384 * 384)).toEqual({ width: 512, height: 288 });
    expect(fitBudget(320, 240, 384 * 384)).toEqual({ width: 320, height: 240 });
    expect(fitBudget(1081, 1921, 768 * 768)).toEqual({ width: 576, height: 1024 });
  });

  it('builds an exact-seek, labelled frame command', () => {
    const args = frameArgs({ input: '/m/a.mov', time: 4.25, width: 320, height: 180, pad: true, label: '0:04.25', fontFile: '/fonts/Menlo.ttc', format: 'jpeg', output: '/t/cell-000.jpg' });
    expect(args.slice(args.indexOf('-ss'), args.indexOf('-ss') + 4)).toEqual(['-ss', '4.25', '-i', '/m/a.mov']);
    const filter = args[args.indexOf('-vf') + 1]!;
    expect(filter).toContain('scale=320:180:force_original_aspect_ratio=decrease,pad=320:180');
    expect(filter).toContain("drawtext=fontfile='/fonts/Menlo.ttc':text='0\\:04.25'");
    expect(args.at(-1)).toBe('/t/cell-000.jpg');
    const unlabeled = frameArgs({ input: 'a', time: 0, width: 2, height: 2, label: 'x', fontFile: null, format: 'png', output: 'o' });
    expect(unlabeled.join(' ')).not.toContain('drawtext');
  });

  it('keeps drawtext labels inert', () => {
    expect(drawtextEscape("1:02:03.40")).toBe('1\\:02\\:03.40');
    expect(drawtextEscape("a'b%{pts}\\c")).toBe('abptsc');
  });
});

describe('auto frame triage', () => {
  const DIM = TRIAGE_GRID * TRIAGE_GRID;
  const flat = (value: number) => new Uint8Array(DIM).fill(value);
  const join = (frames: Uint8Array[]) => { const out = new Uint8Array(frames.length * DIM); frames.forEach((frame, i) => out.set(frame, i * DIM)); return out; };

  it('keeps one frame per settled state and finds a hard cut', () => {
    const raw = join([...Array(8).fill(flat(40)), ...Array(8).fill(flat(200))]);
    const { picks, states } = pickInformativeTimes(raw, 10, 2, 8);
    expect(states).toBe(2);
    expect(picks.map((pick) => pick.time)).toEqual([10, 14.5]);
    expect(picks[1]!.since).toBe(14);
  });

  it('waits out a transition and caps to the most distinct states', () => {
    const ramp = [60, 90, 120, 150].map(flat);
    const raw = join([flat(30), flat(30), ...ramp, flat(180), flat(180), flat(185), flat(250), flat(250)]);
    const all = pickInformativeTimes(raw, 0, 2, 8);
    expect(all.picks.map((pick) => pick.time)).toEqual([0, 3.5, 5]);
    const capped = pickInformativeTimes(raw, 0, 2, 2);
    expect(capped.states).toBe(3);
    expect(capped.picks).toHaveLength(2);
  });

  it('still yields frames from footage that never settles', () => {
    const frames = Array.from({ length: 24 }, (_, index) => flat(index % 2 ? 0 : 255));
    const { picks } = pickInformativeTimes(join(frames), 0, 2, 8);
    expect(picks.length).toBeGreaterThan(0);
  });

  it('scans a window at a fixed rate as 32×32 gray', () => {
    const args = triageScanArgs('/m/a.mp4', 3, 20, 2);
    expect(args).toEqual(expect.arrayContaining(['-ss', '3', '-t', '20', '-f', 'rawvideo']));
    expect(args[args.indexOf('-vf') + 1]).toBe('fps=2,scale=32:32:flags=area,format=gray');
    expect(triageScanArgs('a', 0, 1, 1, true).slice(0, 6)).toContain('nokey');
  });
});

describe('contact sheet plans', () => {
  it('chooses a grid and a cell size within the sheet width', () => {
    expect(planSheet(1, 16 / 9)).toMatchObject({ columns: 1, rows: 1 });
    expect(planSheet(12, 16 / 9)).toMatchObject({ columns: 4, rows: 3 });
    expect(planSheet(48, 16 / 9)).toMatchObject({ columns: 8, rows: 6 });
    const plan = planSheet(12, 16 / 9);
    expect(plan.columns * plan.cellWidth + 4 * (plan.columns + 1)).toBeLessThanOrEqual(1600);
    expect(plan.cellHeight).toBe(Math.round(plan.cellWidth / (16 / 9) / 2) * 2);
    expect(planSheet(7, 9 / 16, 7)).toMatchObject({ columns: 7, rows: 1 });
  });

  it('keeps every sheet within the image size providers accept', () => {
    const bounds = (plan: ReturnType<typeof planSheet>) => ({
      width: plan.columns * plan.cellWidth + 4 * (plan.columns + 1),
      height: plan.rows * plan.cellHeight + 4 * (plan.rows + 1)
    });
    for (const [count, aspect, columns] of [[48, 16 / 9, 1], [48, 9 / 16, 2], [48, 9 / 16, undefined], [48, 0.25, 1], [30, 4, 10], [1, 9 / 16, 1], [5, 1, 1]] as const) {
      const plan = planSheet(count, aspect, columns);
      const size = bounds(plan);
      expect(size.width).toBeLessThanOrEqual(1600);
      expect(size.height).toBeLessThanOrEqual(2400);
      expect(plan.columns * plan.rows).toBeGreaterThanOrEqual(count);
      expect(Math.min(plan.cellWidth, plan.cellHeight)).toBeGreaterThanOrEqual(90);
      // Cells keep the footage's aspect (to the even-pixel rounding ffmpeg needs).
      expect(Math.abs(plan.cellWidth / plan.cellHeight - Math.min(4, Math.max(0.25, aspect)))).toBeLessThan(0.1 * Math.max(1, aspect));
    }
    // One column of 48 landscape frames would be 17k px tall; the grid widens instead.
    expect(planSheet(48, 16 / 9, 1).columns).toBeGreaterThan(1);
    expect(planSheet(48, 9 / 16, 2).columns).toBeGreaterThan(2);
    // A single tall frame fits the height too.
    expect(bounds(planSheet(1, 9 / 16, 1)).height).toBeLessThanOrEqual(2400);
  });
});

describe('audio analysis parsing', () => {
  const LOG = `[silencedetect @ 0x1] silence_start: 2.00533
[silencedetect @ 0x1] silence_end: 3.52 | silence_duration: 1.51467
[silencedetect @ 0x1] silence_start: 5.504
[silencedetect @ 0x1] silence_end: 6.20802 | silence_duration: 0.704021
[silencedetect @ 0x1] silence_start: 7.6
[Parsed_astats_2 @ 0x2] Overall
[Parsed_astats_2 @ 0x2] Peak level dB: -22.332721
[Parsed_astats_2 @ 0x2] RMS level dB: -28.509493
[Parsed_astats_2 @ 0x2] Flat factor: 0.000000
[Parsed_astats_2 @ 0x2] Peak count: 2.000000
[Parsed_ebur128_1 @ 0x3] Summary:

  Integrated loudness:
    I:         -28.3 LUFS
    Threshold: -38.4 LUFS

  Loudness range:
    LRA:         1.9 LU
    Threshold: -49.8 LUFS

  Sample peak:
    Peak:      -22.3 dBFS

  True peak:
    Peak:      -22.1 dBFS`;

  it('turns silencedetect lines into media-time ranges, closing an open run at the window end', () => {
    expect(parseSilences(LOG, 10, 18)).toEqual([[12.005, 13.52], [15.504, 16.208], [17.6, 18]]);
    expect(parseSilences('', 0, 5)).toEqual([]);
  });

  it('reads loudness, peaks and a clipping verdict', () => {
    expect(parseLoudness(LOG)).toEqual({ integratedLufs: -28.3, loudnessRangeLu: 1.9, truePeakDbfs: -22.1, samplePeakDbfs: -22.3, rmsDbfs: -28.5, clipping: false });
    expect(parseLoudness('Peak level dB: 0.000265\nFlat factor: 31.2\n').clipping).toBe(true);
    expect(parseLoudness('Peak level dB: 6.83\nFlat factor: 0.0\n').clipping).toBe(true);
    expect(parseLoudness('Peak level dB: -0.01\nFlat factor: 0.0\n').clipping).toBe(false);
    expect(parseLoudness('Summary:\n  Integrated loudness:\n    I:         -inf LUFS\nPeak level dB: -inf\n')).toMatchObject({ integratedLufs: null, samplePeakDbfs: null, clipping: false });
  });

  it('analyses only the first audio stream of the requested window', () => {
    const args = analysisArgs('/m/a.mov', 1.5, 4, { thresholdDb: -45, minDuration: 0.3 });
    expect(args).toEqual(expect.arrayContaining(['-ss', '1.5', '-t', '4', '-map', '0:a:0', '-f', 'null']));
    expect(args[args.indexOf('-af') + 1]).toMatch(/^silencedetect=noise=-45dB:d=0\.3,ebur128=peak=true\+sample:framelog=verbose,astats=/);
  });

  it('draws shaded silences and a ruler on the waveform', () => {
    expect(tickStep(8)).toBe(1);
    expect(tickStep(600)).toBe(60);
    const args = waveformArgs({ input: 'a', start: 0, duration: 8, width: 800, height: 200, silences: [[2, 3.5]], fontFile: '/f.ttc', output: 'w.png' });
    const graph = args[args.indexOf('-filter_complex') + 1]!;
    expect(graph).toContain('showwavespic=s=800x200');
    expect(graph).toContain('drawbox=x=200:y=0:w=150:h=200:color=0xff453a@0.30:t=fill');
    expect(graph).toContain("text='0\\:01.00'");
    expect(graph).toContain('pad=800:222');
  });
});

describe('composition and source time', () => {
  const constant: Array<[number, number]> = [[10, 1], [16, 7]];
  const doubled: Array<[number, number]> = [[10, 0], [14, 8]];
  const reversed: Array<[number, number]> = [[0, 5], [5, 0]];

  it('maps both ways along piecewise-linear timing', () => {
    expect(compositionToSource(constant, 12)).toBe(3);
    expect(sourceToComposition(constant, 3)).toBe(12);
    expect(sourceToComposition(doubled, 4)).toBe(12);
    expect(sourceToComposition(reversed, 1)).toBe(4);
    expect(sourceToComposition(constant, 0.5)).toBeNull();
    expect(sourceToComposition(constant, 7.5)).toBeNull();
  });
});
