import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { expect, test, repoRoot } from './helpers/app';

for (const format of ['webm', 'mp4']) test(`prepares exact ${format} pixels for independent, shared and nested video layers`, async ({ session }) => {
  test.setTimeout(60_000);
  await session.openEditor();
  await session.app.evaluate(({ ipcMain }) => {
    const state = { chunks: [] as number[][], finished: false };
    (globalThis as any).__preparedExportPixels = state;
    for (const name of ['start', 'write', 'finish', 'cancel']) ipcMain.removeHandler(`render:${name}`);
    ipcMain.handle('render:start', () => 'prepared-pixels');
    ipcMain.handle('render:write', (_event, request) => { state.chunks.push(Array.from(request.data)); });
    ipcMain.handle('render:finish', () => { state.finished = true; return { path: '/test/prepared-pixels' }; });
    ipcMain.handle('render:cancel', () => {});
  });
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-prepare-frames-'));
  const ffmpeg = path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg');
  try {
    const source = path.join(root, `steps.${format}`);
    const raw = Buffer.concat(Array.from({ length: 20 }, (_, i) => Buffer.alloc(32 * 32 * 3, 20 + i * 10)));
    execFileSync(ffmpeg, ['-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '32x32', '-r', '30', '-i', 'pipe:0',
      ...(format === 'webm' ? ['-c:v', 'libvpx-vp9', '-lossless', '1'] : ['-c:v', 'libx264', '-crf', '0', '-pix_fmt', 'yuv420p']), source], { input: raw });
    const result = await session.page.evaluate(async ({ bytes, format }) => {
      const PM = (window as any).PM;
      PM.pause();
      PM.agentFrameCapture = true;
      PM.proj = PM.mkProject({ name: 'Parallel video preparation', w: 32, h: 32, fps: 30, dur: 20 / 30 });
      const assets = new Map<string, any>();
      const get = PM.assets.get;
      const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: `video/${format}` }));
      const videos: HTMLVideoElement[] = [];
      try {
        for (let i = 0; i < 5; i++) {
          const el = document.createElement('video'); el.muted = true; el.preload = 'auto';
          videos.push(el);
          await new Promise<void>((resolve, reject) => {
            el.addEventListener('loadeddata', () => resolve(), { once: true });
            el.addEventListener('error', () => reject(new Error('Fixture video did not load')), { once: true });
            el.src = url;
          });
          assets.set(`source${i}`, { el, w: 32, h: 32, dur: 20 / 30, videoTimestampScale: format === 'webm' ? .001 : undefined });
        }
        PM.assets.get = (id: string) => assets.get(id) || get.call(PM.assets, id);
        const clip = (asset: string, trim = 0, comp = PM.proj) => PM.mkLayer('video', {
          from: 0, dur: 20 / 30, d: { asset, w: 32, h: 32, speed: 1, trim, embeddedAudio: false },
        }, comp);
        const independent = [...assets.keys()].map(id => clip(id));
        // Consecutive requests share the actual decoder position. Returning to
        // the same target after other seeks can land at a different tolerated time.
        const consecutiveDuplicate = clip('source0'), duplicate = clip('source0'), shifted = clip('source0', 3 / 30);
        const child = PM.mkProject({ w: 32, h: 32, fps: 30, dur: 20 / 30 });
        const nested = clip('source0', 0, child); child.layers = [nested];
        PM.proj.comps[child.id] = child;
        const precomp = PM.mkLayer('precomp', { from: 0, dur: 20 / 30, d: { comp: child.id, w: 32, h: 32, speed: 1, trim: 2 / 30 } }, PM.proj);
        PM.proj.layers = [independent[0], consecutiveDuplicate, ...independent.slice(1), shifted, precomp, duplicate];
        PM.touch();
        const samples: Array<{ frame: number; values: number[]; sameCanvas: boolean }> = [];
        // Include backward jumps so cached duplicate pixels cannot accidentally
        // come from an earlier capture or the decoder's most recent source time.
        for (const frame of [0, 1, 2, 8, 4, 14, 3]) {
          const time = frame / 30;
          await PM.prepareFrame(time);
          const frames = PM.preparedVideoFrames as Map<string, HTMLCanvasElement>;
          const read = (layer: any, at = time) => frames.get(`${layer.id}@${at}`)!.getContext('2d')!.getImageData(0, 0, 1, 1).data[0]!;
          samples.push({ frame, values: [...independent.map(layer => read(layer)), read(consecutiveDuplicate), read(duplicate), read(shifted), read(nested, time + 2 / 30)],
            sameCanvas: frames.get(`${independent[0].id}@${time}`) === frames.get(`${consecutiveDuplicate.id}@${time}`) });
        }
        // A relinked/converted source can be shorter than saved asset metadata.
        // Decode the real last frame instead of waiting forever at an impossible time.
        const layersBeforeClamp = PM.proj.layers, durationBeforeClamp = independent[0].dur;
        PM.proj.layers = [independent[0]]; independent[0].dur = 2; assets.get('source0').dur = 2;
        await PM.prepareFrame(1.2);
        const lastPixel = PM.preparedVideoFrames.get(`${independent[0].id}@1.2`).getContext('2d').getImageData(0, 0, 1, 1).data[0];
        if (Math.abs(lastPixel - 210) >= 5) throw new Error(`Clamped video captured the wrong frame: ${lastPixel}`);
        PM.proj.layers = layersBeforeClamp; independent[0].dur = durationBeforeClamp; assets.get('source0').dur = 20 / 30;

        // Reference captures retain separate per-layer source textures because
        // no export is active. Compare every delivered raw RGBA byte against it.
        const expected: number[] = [];
        const captureOptions = { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 };
        for (let frame = 0; frame < 20; frame++) {
          const time = frame / 30;
          await PM.prepareFrame(time);
          expected.push(...PM.GL.renderToPixels(time, 32, 32, captureOptions));
        }

        const gl = PM.GL.gl, upload = gl.texImage2D;
        const generations = new Map<number, { uploads: number; unique: number }>();
        let phase: 'isolated' | 'export' | null = null, isolatedUploads = 0;
        gl.texImage2D = function (...args: any[]) {
          const frames = PM.preparedVideoFrames as Map<string, HTMLCanvasElement> | null;
          if (phase && args.length === 6 && frames && [...frames.values()].includes(args[5])) {
            if (phase === 'isolated') isolatedUploads++;
            else {
              let generation = generations.get(PM.preparedVideoVersion);
              if (!generation) {
                generation = { uploads: 0, unique: new Set(frames.values()).size };
                generations.set(PM.preparedVideoVersion, generation);
              }
              generation.uploads++;
            }
          }
          return upload.apply(this, args as any);
        };
        try {
          await PM.prepareFrame(0);
          const layers = PM.proj.layers;
          // Private thumbnail captures must keep their own texture namespace,
          // even if an export currently owns a shared prepared-frame map.
          PM.Export.busy = true;
          PM.proj.layers = layers.filter((layer: any) => layer.type === 'video');
          phase = 'isolated';
          try { PM.GL.renderToPixels(0, 32, 32, { ...captureOptions, isolatedVideoFrames: true }); }
          finally { PM.Export.busy = false; PM.proj.layers = layers; phase = null; }
          const isolatedRetained = [...PM.GL.texes.keys()].some((key: any) => key.startsWith('capture-video:'));
          phase = 'export';
          // A stuck live decoder must recover once, then reuse the replacement
          // for all following frames while preserving the exact exported pixels.
          if (format === 'mp4') Object.defineProperty(videos[0], 'seeking', { configurable: true, get: () => true });
          PM.agentFrameCapture = false;
          const exported = await PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 30, range: 'all',
            quality: 'draft', audio: false, mblur: false });
          return { samples, decoders: new Set([...assets.values()].map(asset => asset.el)).size,
            expected, generations: [...generations.values()], isolatedUploads, isolatedRetained, exported };
        } finally { phase = null; gl.texImage2D = upload; }
      } finally {
        if (format === 'mp4') delete (videos[0] as any).seeking;
        PM.agentFrameCapture = false; PM.preparedVideoFrames = null; PM.assets.get = get; PM.proj.layers = [];
        for (const video of videos) { video.pause(); video.removeAttribute('src'); video.load(); }
        URL.revokeObjectURL(url);
      }
    }, { bytes: Array.from(await readFile(source)), format });
    const delivered = await session.app.evaluate(() => (globalThis as any).__preparedExportPixels as { chunks: number[][]; finished: boolean });
    expect(result.exported.cancelled).toBe(false);
    expect(delivered.finished).toBe(true);
    expect(delivered.chunks.flat()).toEqual(result.expected);
    expect(result.expected.slice(0, 32 * 32 * 4)).not.toEqual(result.expected.slice(-32 * 32 * 4));
    expect(result.generations).toHaveLength(20);
    for (const generation of result.generations) expect(generation.uploads).toBe(generation.unique);
    expect(result.generations.some(generation => generation.uploads < 8)).toBe(true);
    expect(result.isolatedUploads).toBe(8);
    expect(result.isolatedRetained).toBe(false);
    expect(result.decoders).toBe(5);
    for (const sample of result.samples) {
      expect(sample.sameCanvas).toBe(true);
      const expected = [...Array(7).fill(20 + sample.frame * 10), 20 + (sample.frame + 3) * 10, 20 + (sample.frame + 2) * 10];
      sample.values.forEach((value, index) => expect(Math.abs(value - expected[index]!), JSON.stringify(sample)).toBeLessThan(5));
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await rm(root, { recursive: true, force: true }); }
});
