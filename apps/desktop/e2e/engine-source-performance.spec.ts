/** Paired source workloads: pm-perfbench's public fixture on a 320x180
 * monitor, and 1080p H.264, 4K 10-bit HEVC and 1080p ProRes 422 on a
 * 1280x720 monitor. Each app session imports COPIES distinct remuxes of every
 * workload into the media pool without timeline layers (same streams,
 * different container metadata, so import cannot reuse an asset); each copy's first filmstrip is one cold sample and its
 * first scrub is the first seek after opening (not startup). RUNS alternating off/on/on/off cycles feed
 * helpers/source-performance-gate.ts. Each session waits for the machine's
 * 1-minute load to drop under MAX_LOAD and records it; the gate fails a
 * session that never got there. Once the first legacy session has frozen the
 * paired targets, one untimed native session proves the displayed original
 * frames (helpers/source-frame-correctness.ts); the gate accepts timings only
 * with that evidence for this campaign, build, fixtures, targets and geometry. */
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createHash, randomUUID } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { expect, launchApp, repoRoot, test } from './helpers/app';
import { fixturePath } from './helpers/media';
import { sourceFfmpeg } from './helpers/source-media';
import { commonSourceTargets, installSourcePerformanceDisplay, sourceFrameTimeline, sourceTimelineCorrespondence, type SourceFrameTimeline, type SourceDisplayProbe } from './helpers/source-performance-display';
import { MAX_LOAD, MIN_RUNS, percentile, sourcePerformanceGate, type PerfSample } from './helpers/source-performance-gate';
import { decodedFrameGrids, FRAME_MATCH, judgeSourceFrame, sourceBuildIdentity, sourceCellGrid, sourcePlaneGrid, SRGB8_LINEAR, type CampaignFixture, type SourceFrameCorrectness } from './helpers/source-frame-correctness';

const COPIES = 3;
// Fewer runs (diagnostics) cannot pass: the gate itself needs MIN_RUNS. They
// measure at once rather than waiting for a quiet machine.
const RUNS = Number(process.env.PM_SOURCE_PERF_RUNS) || MIN_RUNS;
const LOAD_WAIT_MS = RUNS < MIN_RUNS ? 0 : 10 * 60_000;
const exec = promisify(execFile);
const sampleTimes = (scattered: boolean) => Array.from({ length: 24 }, (_, k) =>
  Math.round(((scattered ? (k * 7) % 24 : k) * 1.9 / 24 + 1 / 60) * 705_600_000) / 705_600_000);
const FILMSTRIP_TIMES = sampleTimes(false);
const digest = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
async function probeTimeline(file: string): Promise<SourceFrameTimeline> {
  const ffprobe = process.env.PM_FFMPEG_DIR ? path.join(process.env.PM_FFMPEG_DIR, 'bin/ffprobe') : 'ffprobe';
  const probed = await exec(ffprobe, ['-v', 'error', '-select_streams', 'v:0', '-show_frames', '-show_streams',
    '-show_entries', 'frame=best_effort_timestamp:stream=time_base,start_pts', '-of', 'json', file],
  { timeout: 60_000, maxBuffer: 4 * 1024 * 1024 });
  return sourceFrameTimeline(JSON.parse(probed.stdout));
}
const spread = (values: number[]) => ({ n: values.length, p50: percentile(values, .5), p95: percentile(values, .95),
  p99: percentile(values, .99), max: Math.max(...values), top: [...values].sort((a, b) => b - a).slice(0, 5) });

const synthetic = (size: string, codec: string[]) =>
  ['-f', 'lavfi', '-i', `testsrc2=size=${size}:rate=30:duration=2`, ...codec];
const WORKLOADS = [
  { name: 'h264-160', monitor: [320, 180], extension: 'mp4', encode: null },
  { name: 'h264-1080', monitor: [1280, 720], extension: 'mp4',
    encode: synthetic('1920x1080', ['-c:v', 'libx264', '-preset', 'veryfast', '-g', '30', '-pix_fmt', 'yuv420p']) },
  // Closed GOPs: today's route seeking to a CRA drops its open-GOP leading
  // pictures and shows the CRA instead, so it could never present those
  // targets. That is a legacy correctness gap, not a latency workload.
  { name: 'hevc10-4k', monitor: [1280, 720], extension: 'mp4',
    encode: synthetic('3840x2160', ['-c:v', 'libx265', '-preset', 'ultrafast', '-x265-params', 'log-level=error:keyint=30:open-gop=0',
      '-pix_fmt', 'yuv420p10le', '-tag:v', 'hvc1']) },
  { name: 'prores-1080', monitor: [1280, 720], extension: 'mov',
    encode: synthetic('1920x1080', ['-c:v', 'prores_ks', '-profile:v', '2', '-pix_fmt', 'yuv422p10le']) }
] as const;

type LaunchedSession = Awaited<ReturnType<typeof launchApp>>;

/** A media-panel import has no timeline placement. Composition rendering
 * must not seek the elements this source benchmark owns. */
async function importCopy(session: LaunchedSession, file: string, enabled: boolean) {
  const chooser = session.page.waitForEvent('filechooser');
  await session.page.evaluate(() => {
    const input = document.createElement('input'); input.type = 'file';
    input.onchange = async () => {
      try { await (window as any).PM.importFiles([...input.files!], { placement: null }); }
      finally { input.remove(); }
    };
    document.body.append(input); input.click();
  });
  await (await chooser).setFiles(file);
  await session.page.waitForFunction(name => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === name), path.basename(file));
  if (enabled) await session.page.waitForFunction(() => !!(window as any).PM.EngineRender?.ready);
}

/** Untimed native session: every frozen target of every fixture, captured
 * after its delivery is acknowledged and presented, against FFmpeg's decode. */
async function originalFrameCorrectness(campaign: string, build: string, copies: string,
  fixtures: (CampaignFixture & { timeline: SourceFrameTimeline; frames: number[] })[]): Promise<SourceFrameCorrectness> {
  const evidence: SourceFrameCorrectness = { campaign, build, tolerances: FRAME_MATCH, pass: false, fixtures: [] };
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await session.openEditor();
    await session.page.evaluate(installSourcePerformanceDisplay);
    await session.page.evaluate(`globalThis.__sourceCellGrid = ${sourceCellGrid.toString()}; globalThis.__sourcePlaneGrid = ${sourcePlaneGrid.toString()}`);
    for (const fixture of fixtures) {
      const { timeline, frames, ...campaignFixture } = fixture;
      const entry: SourceFrameCorrectness['fixtures'][number] = { ...campaignFixture, captured: null, error: null, judgements: [] };
      evidence.fixtures.push(entry);
      try {
        const file = path.join(copies, fixture.name);
        if (digest(await readFile(file)) !== fixture.sha256) throw new Error('Fixture bytes changed since they were indexed');
        await importCopy(session, file, true);
        const wanted = new Set(frames.flatMap(frame => [frame - 1, frame, frame + 1]).filter(frame => frame >= 0 && frame < timeline.pts.length));
        const captured = await session.page.evaluate(async ({ name, timeline, targets, monitorSize: [width, height], lut, planeColumns, rgbColumns }) => {
          const PM = (window as any).PM;
          const id = Object.values<any>(PM.proj.assets).find(a => a.name === name)!.id;
          const viewer = PM.Kernel.services.get('viewer');
          const painted = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
          let display: SourceDisplayProbe | null = null;
          try {
            viewer.preview.clear();
            viewer.preview.show(id);
            const root = document.querySelector<HTMLElement>('#source-preview')!;
            root.style.width = `${width / devicePixelRatio}px`; root.style.height = `${height / devicePixelRatio}px`;
            root.style.right = root.style.bottom = 'auto';
            const monitor = PM.EngineSourceMedia.monitorValue;
            display = (window as any).__sourcePerformanceDisplay(monitor, { native: true, asset: id, timeline, trace: false }) as SourceDisplayProbe;
            const until = performance.now() + 15_000;
            while (monitor.paintedSize?.width !== width || monitor.paintedSize?.height !== height) {
              if (performance.now() > until) throw new Error('Source monitor did not reach its measured size');
              await painted();
            }
            await display.ready();
            const video = monitor.surface as HTMLVideoElement;
            const results = [];
            for (const target of targets) {
              await display.seek(target);
              const before = display.shown();
              if (!before) throw new Error(`No acknowledged delivery on screen at ${target}`);
              // The element's current frame as delivered: no canvas, no colour conversion.
              const frame = new VideoFrame(video);
              let capture;
              try {
                const rect = frame.visibleRect!, width = rect.width, height = rect.height, format = String(frame.format);
                const bytes = new Uint8Array(frame.allocationSize({ rect }));
                const layout = await frame.copyTo(bytes, { rect });
                const wide = format === 'I420P10';
                const data = wide ? new Uint16Array(bytes.buffer, 0, bytes.length / 2) : bytes;
                const unit = wide ? 2 : 1;
                const plane = (index: number, w: number, h: number, extra = 0, step = 1) => ({ data, offset: layout[index]!.offset / unit + extra,
                  stride: layout[index]!.stride / unit, step, width: w, height: h });
                const cw = Math.ceil(width / 2), ch = format === 'I422' ? height : Math.ceil(height / 2);
                const grid = (globalThis as any);
                const codes: number[] = format === 'RGBA' ? grid.__sourceCellGrid(bytes, width, height, 4, lut, null, rgbColumns)
                  : format === 'NV12' ? grid.__sourcePlaneGrid([plane(0, width, height), plane(1, cw, ch, 0, 2), plane(1, cw, ch, 1, 2)], width, height, 1, planeColumns)
                  : ['I420', 'I420A', 'I422', 'I420P10'].includes(format)
                    ? grid.__sourcePlaneGrid([plane(0, width, height), plane(1, cw, ch), plane(2, cw, ch)], width, height, wide ? .25 : 1, planeColumns)
                    : (() => { throw new Error(`Unexpected monitor frame format ${format}`); })();
                capture = { target, frame: before.frame, timestamp: before.timestamp, width, height, format, codes };
              } finally { frame.close(); }
              const after = display.shown();
              if (!after || after.timestamp !== before.timestamp) throw new Error(`The monitor changed frames while ${target} was read`);
              results.push(capture);
            }
            return results;
          } finally {
            display?.dispose();
            viewer.preview.clear();
          }
        }, { name: fixture.name, timeline, targets: fixture.targets, monitorSize: fixture.monitor, lut: SRGB8_LINEAR, planeColumns: FRAME_MATCH.planes.columns, rgbColumns: FRAME_MATCH.converted.columns });
        const mode = captured[0]?.format === 'RGBA' ? 'converted' : 'planes';
        if (captured.some(capture => (capture.format === 'RGBA') !== (mode === 'converted'))) throw new Error('The monitor changed presentation route mid-fixture');
        const reference = await decodedFrameGrids(sourceFfmpeg(), file, timeline.pts.length, wanted, mode);
        for (const [index, capture] of captured.entries()) {
          if (Math.abs(capture.width / capture.height - reference.width / reference.height) > 1 / Math.min(capture.height, reference.height))
            throw new Error(`Displayed ${capture.width}x${capture.height} does not keep the ${reference.width}x${reference.height} picture's aspect`);
          entry.captured = { width: capture.width, height: capture.height, format: capture.format };
          const intended = frames[index]!;
          if (capture.frame !== intended) throw new Error(`Delivery at ${capture.target} answered frame ${capture.frame}, not ${intended}`);
          const neighbours = [intended - 1, intended + 1].filter(frame => reference.frames.has(frame)).map(frame => reference.frames.get(frame)!);
          entry.judgements.push({ target: capture.target, timestamp: capture.timestamp,
            ...judgeSourceFrame(capture.codes, reference.frames.get(intended)!, neighbours, FRAME_MATCH[mode]) });
        }
      } catch (error) { entry.error = String(error); }
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
  evidence.pass = evidence.fixtures.every(entry => !entry.error && entry.judgements.every(judgement => judgement.verdict === 'match'));
  return evidence;
}

async function quietLoad() {
  const until = Date.now() + LOAD_WAIT_MS;
  while (os.loadavg()[0]! >= MAX_LOAD && Date.now() < until) await new Promise(resolve => setTimeout(resolve, 15_000));
  return os.loadavg();
}

test('source filmstrip and monitor scrub cost no more than the current app', async () => {
  test.setTimeout(RUNS * (4 * LOAD_WAIT_MS + 1_200_000) + 600_000); // + the untimed correctness session
  const evidence = process.env.PM_SOURCE_EVIDENCE;
  const directory = evidence ? path.join(evidence, 'performance') : await mkdtemp(path.join(os.tmpdir(), 'pm-source-performance-'));
  await mkdir(directory, { recursive: true });
  const machine = { startedAt: new Date().toISOString(), platform: `${os.platform()} ${os.release()} ${os.arch()}`,
    cpus: os.cpus().length, loadBefore: os.loadavg() };
  const results: PerfSample[] = [];
  const traceEnabled = process.env.PM_SOURCE_PERF_TRACE === '1';
  const traces: unknown[] = [];
  const provenance: unknown[] = [];
  const targetPlans = new Map<string, { legacyTimeline: SourceFrameTimeline; targets: ReturnType<typeof commonSourceTargets> }>();
  const copies = await mkdtemp(path.join(os.tmpdir(), 'pm-source-performance-copies-'));
  const campaign = randomUUID(), build = await sourceBuildIdentity(repoRoot);
  let correctness: SourceFrameCorrectness | null = null;
  try {
    const imports: { workload: (typeof WORKLOADS)[number]; copy: number; name: string; timeline: SourceFrameTimeline; sha256: string }[] = [];
    const campaignFixture = (fixture: (typeof imports)[number]): CampaignFixture => ({ workload: fixture.workload.name, name: fixture.name, sha256: fixture.sha256,
      targets: targetPlans.get(fixture.name)?.targets.map(sample => sample.target) ?? [], monitor: fixture.workload.monitor });
    for (const workload of WORKLOADS) {
      const base = workload.encode ? path.join(copies, `${workload.name}.${workload.extension}`) : fixturePath('h264-aac.mp4');
      if (workload.encode) await exec(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin', '-y', ...workload.encode, base], { timeout: 300_000 });
      for (let copy = 0; copy < COPIES; copy++) {
        const name = `${workload.name}-cold-${copy}.${workload.extension}`;
        await exec(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin', '-y', '-i', base, '-map', '0', '-c', 'copy',
          '-metadata', `comment=pm-source-cold-${workload.name}-${copy}`, '-movflags', '+faststart', path.join(copies, name)], { timeout: 60_000 });
        // Actual presentation timestamps from each imported remux, outside
        // every timed route. Never infer frame identity from nominal FPS.
        imports.push({ workload, copy, name, timeline: await probeTimeline(path.join(copies, name)),
          sha256: digest(await readFile(path.join(copies, name))) });
      }
    }
    // This spec itself runs under pm-electron-lock. Build the probe beforehand;
    // collect its native-only counters beside the end-to-end app wall clocks.
    const probe = await exec(path.resolve(repoRoot, '../../engine/target/release/examples/source_media'), [], {
      env: { ...process.env, PM_SOURCE_BENCH_MEDIA: fixturePath('h264-aac.mp4') }, timeout: 60_000
    });
    const counters = probe.stdout.trim().split('\n').map(line => JSON.parse(line));
    // Alternating order makes drift visible instead of giving one mode every warm run.
    for (let run = 0; run < RUNS; run++) for (const [round, enabled] of [[0, false], [0, true], [1, true], [1, false]] as const) {
      const loadBefore = await quietLoad();
      const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
      try {
        await session.openEditor();
        await session.page.evaluate(installSourcePerformanceDisplay);
        for (const { workload, copy, name, timeline: originalTimeline, sha256: originalSha256 } of imports) {
          await importCopy(session, path.join(copies, name), enabled);
          let timeline = originalTimeline, sha256 = originalSha256, expectedSource: string | undefined;
          if (!enabled) {
            // Match source-preview.ts's liveUrl selection, fetch those exact
            // bytes, then assert the opened video's currentSrc is this URL.
            // ProRes here can be a VP9 WebM rendition with a 1 ms timebase.
            const actual = await session.page.evaluate(async name => {
              const PM = (window as any).PM;
              const id = Object.values<any>(PM.proj.assets).find(a => a.name === name)!.id;
              const live = PM.assets.get(id);
              const url = String(live?.url || live?.el?.currentSrc || live?.el?.src || '');
              if (!url) throw new Error('Legacy source has no URL to index');
              const response = await fetch(url);
              if (!response.ok) throw new Error(`Cannot index legacy source: ${response.status}`);
              const blob = await response.blob();
              if (!blob.size || blob.size > 128 * 1024 * 1024) throw new Error('Legacy source evidence exceeds its byte bound');
              return { url: new URL(url, document.baseURI).href, bytes: [...new Uint8Array(await blob.arrayBuffer())] };
            }, name);
            expectedSource = actual.url;
            const bytes = new Uint8Array(actual.bytes);
            sha256 = digest(bytes);
            const file = path.join(copies, 'actual-legacy-source.bin');
            try { await writeFile(file, bytes); timeline = await probeTimeline(file); }
            finally { await rm(file, { force: true }); }
          }
          let correspondence: ReturnType<typeof sourceTimelineCorrespondence> | null = null, correspondenceError: string | null = null;
          let plan = targetPlans.get(name);
          try {
            // The first session is legacy. Freeze its common interior targets
            // for EVERY later session, preserving all 24 ordinals and the
            // original chronological filmstrip / (k*7)%24 scrub order. Only
            // boundary targets move; provenance records each exact delta.
            if (!plan) {
              if (enabled) throw new Error('Legacy timing must establish paired source targets first');
              plan = { legacyTimeline: timeline, targets: commonSourceTargets(originalTimeline, timeline, FILMSTRIP_TIMES) };
              targetPlans.set(name, plan);
            } else if (!enabled && JSON.stringify(timeline) !== JSON.stringify(plan.legacyTimeline)) {
              throw new Error('Legacy rendition timing changed after paired targets were frozen');
            }
            correspondence = sourceTimelineCorrespondence(originalTimeline, plan.legacyTimeline, plan.targets.map(sample => sample.target));
          }
          catch (error) { correspondenceError = String(error); }
          const reference = { workload: workload.name, run, enabled, round, copy, sha256, originalSha256,
            source: enabled ? 'original imported remux' : 'fetched legacy live URL bytes',
            currentSrcVerified: enabled ? null : false as boolean | null,
            timeline, originalTimeline, targetPlan: plan, correspondence, correspondenceError };
          provenance.push(reference);
          await writeFile(path.join(directory, 'source-frame-provenance.json'), JSON.stringify(provenance, null, 2));
          if (correspondenceError) throw new Error(correspondenceError);
          const filmstripTimes = plan!.targets.map(sample => sample.target);
          const scrubTimes = filmstripTimes.map((_, k) => filmstripTimes[(k * 7) % 24]!);
          const measured = await session.page.evaluate(async ({ enabled, name, timeline, expectedSource, traceEnabled, scrubTimes, filmstripTimes, monitorSize: [width, height] }) => {
            const PM = (window as any).PM;
            const id = Object.values<any>(PM.proj.assets).find(a => a.name === name)!.id;
            const viewer = PM.Kernel.services.get('viewer');
            const painted = () => new Promise<void>((resolve, reject) => {
              const frame = requestAnimationFrame(() => { clearTimeout(timer); resolve(); });
              const timer = setTimeout(() => { cancelAnimationFrame(frame); reject(new Error('Source setup RAF timed out')); }, 15_000);
            });
            const seekForFilmstrip = (source: HTMLVideoElement, time: number) => new Promise<void>((resolve, reject) => {
              const done = () => { if (Math.abs(source.currentTime - time) < .00001) {
                clearTimeout(timer); source.removeEventListener('seeked', done); resolve();
              } };
              const timer = setTimeout(() => {
                source.removeEventListener('seeked', done);
                reject(new Error(`${name} filmstrip seeked timed out at ${time}; currentTime=${source.currentTime}, readyState=${source.readyState}`));
              }, 15_000);
              source.addEventListener('seeked', done);
              source.currentTime = time;
            });
            let previewCleared = false;
            let sourceVerified = false;
            let display: SourceDisplayProbe | null = null;
            try {
              viewer.preview.clear(); // show() must create a fresh element, not return an already-open one.
              viewer.preview.show(id);
              const root = document.querySelector<HTMLElement>('#source-preview')!;
              root.style.width = `${width / devicePixelRatio}px`; root.style.height = `${height / devicePixelRatio}px`;
              root.style.right = root.style.bottom = 'auto';
              const monitor = enabled ? PM.EngineSourceMedia.monitorValue : root.querySelector('video');
              display = (window as any).__sourcePerformanceDisplay(monitor, { native: enabled, asset: id, timeline, expectedSource, trace: traceEnabled }) as SourceDisplayProbe;
              const until = performance.now() + 15_000;
              while (enabled ? monitor.paintedSize?.width !== width || monitor.paintedSize?.height !== height : monitor.readyState < 2) {
                if (performance.now() > until) throw new Error('Source monitor did not reach its measured size');
                await painted();
              }
              await painted();
              await display.ready();
              sourceVerified = !enabled; // ready() asserted actual currentSrc against the indexed URL.
              const scrubMs: number[] = [];
              for (const time of scrubTimes) {
                // Result + native delivery-matched / legacy source-PTS-matched
                // rVFC, then RAF. Native decoded frame identity requires
                // separate untimed original-media correctness evidence.
                scrubMs.push(await display.seek(time));
              }
              display.dispose();
              viewer.preview.clear(); previewCleared = true;
              const filmstripMs: number[] = [];
              for (let run = 0; run < 3; run++) {
                const began = performance.now();
                if (enabled) {
                  const frames = await PM.EngineSourceMedia.filmstrip(id, filmstripTimes, 160, 90);
                  if (frames.length !== 24 || frames.some((frame: Uint8Array) => frame.length !== 160 * 90 * 4)) throw new Error('Incomplete measured filmstrip');
                } else {
                  const video = PM.assets.get(id).el;
                  const canvas = document.createElement('canvas'); canvas.width = 160; canvas.height = 90;
                  const context = canvas.getContext('2d')!;
                  const frames: Uint8Array[] = [];
                  for (const time of filmstripTimes) {
                    await seekForFilmstrip(video, time);
                    context.clearRect(0, 0, 160, 90); context.drawImage(video, 0, 0, 160, 90);
                    frames.push(new Uint8Array(context.getImageData(0, 0, 160, 90).data));
                  }
                  if (frames.length !== 24) throw new Error('Incomplete current filmstrip');
                }
                filmstripMs.push(performance.now() - began);
              }
              return { scrubMs, filmstripMs, error: null, sourceVerified, trace: display.evidence() };
            } catch (error) {
              // Return diagnostics before failing so a silent/mismatched
              // delivery still reaches the existing Node evidence transport.
              return { scrubMs: [], filmstripMs: [], error: String(error), sourceVerified, trace: display?.evidence() ?? null };
            } finally {
              display?.dispose();
              if (!previewCleared) viewer.preview.clear();
            }
          }, { enabled, name, timeline, expectedSource, traceEnabled, scrubTimes, filmstripTimes, monitorSize: workload.monitor });
          if (!enabled) reference.currentSrcVerified = measured.sourceVerified;
          await writeFile(path.join(directory, 'source-frame-provenance.json'), JSON.stringify(provenance, null, 2));
          if (traceEnabled) {
            traces.push({ workload: workload.name, run, enabled, round, copy, timeline, error: measured.error, ...measured.trace });
            await writeFile(path.join(directory, 'source-display-trace.json'), JSON.stringify(traces, null, 2));
          }
          if (measured.error) throw new Error(measured.error);
          results.push({ workload: workload.name, run, enabled, round, copy, loadBefore, scrubMs: measured.scrubMs, filmstripMs: measured.filmstripMs });
        }
        expect(session.diagnostics.pageErrors).toEqual([]);
      } finally {
        await session.close();
      }
      await writeFile(path.join(directory, 'app-source-performance.json'), JSON.stringify(results, null, 2));
      if (!correctness && targetPlans.size === imports.length) {
        correctness = await originalFrameCorrectness(campaign, build, copies, imports.map(fixture => ({ ...campaignFixture(fixture), timeline: fixture.timeline,
          frames: targetPlans.get(fixture.name)!.targets.map(sample => sample.frame) })));
        await writeFile(path.join(directory, 'source-frame-correctness.json'), JSON.stringify(correctness));
      }
    }
    const rebuilt = await sourceBuildIdentity(repoRoot) !== build;
    const gate = sourcePerformanceGate(results, { evidence: correctness, campaign, fixtures: imports.map(campaignFixture),
      build: rebuilt ? 'the app build changed during the campaign' : build });
    // Per run, session and copy, so sub-vsync jitter reads apart from a tail.
    const distribution = results.map(r => ({ workload: r.workload, run: r.run, enabled: r.enabled, round: r.round, copy: r.copy,
      loadBefore: r.loadBefore, firstScrub: r.scrubMs[0], steady: spread(r.scrubMs.slice(1)), coldFilmstrip: r.filmstripMs[0] }));
    await writeFile(path.join(directory, 'comparison.json'), JSON.stringify({ gate, distribution, counters,
      measurement: { scrub: { native: 'result + delivery-matched rVFC + RAF', legacy: 'result + source-PTS-matched rVFC + RAF' },
        firstScrub: 'first seek after opening; startup excluded',
        targets: 'common interior intervals; frozen across paired sessions; exact adjustments in source-frame-provenance.json',
        correctness: { campaign, build, evidence: 'source-frame-correctness.json', pass: correctness?.pass ?? false,
          method: 'untimed native session; displayed frame after acknowledged delivery vs independent FFmpeg decode of intended and neighbouring frames' } } }, null, 2));
    expect(gate.failures).toEqual([]);
  } finally {
    await rm(copies, { recursive: true, force: true });
    try { await writeFile(path.join(directory, 'machine.json'), JSON.stringify({ ...machine, endedAt: new Date().toISOString(), loadAfter: os.loadavg() }, null, 2)); }
    finally { if (!evidence && !traceEnabled) await rm(directory, { recursive: true, force: true }); }
  }
});
