import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { createServer, type Server } from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import type { Page } from 'playwright';

import { expect, launchApp, repoRoot, test, type LaunchedApp } from './helpers/app';

/*
 * The media-intelligence lanes working together on real pieces: spoken media
 * made with `say`, the bundled ffmpeg, the real transcription utility process
 * with a real Parakeet model, captions generated through the UI, the agent's
 * media and caption tools called by a model through the live tool bridge,
 * subtitle files round-tripping, and the Mixer metering real playback.
 *
 * POWERMOVE_E2E_PARAKEET_DIR=<folder with the Parakeet V3 sherpa files> runs
 * the tests that need a downloaded model; the rest run without one.
 * PM_E2E_SHOTS=/some/dir saves review screenshots there (never in the repo).
 */

const run = promisify(execFile);
const ffmpeg = path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg');
const parakeetDir = process.env.POWERMOVE_E2E_PARAKEET_DIR;
const hasModel = !!parakeetDir && existsSync(path.join(parakeetDir, 'encoder.int8.onnx'));
const shots = process.env.PM_E2E_SHOTS;

const SENTENCE_A = 'The quick brown fox jumps over the lazy dog.';
const SENTENCE_B = 'Captions appear on the timeline above the video.';
/** Seconds of silence between the two sentences. */
const GAP = 2;

interface Speech { file: string; audio: string; gapStart: number; gapEnd: number; duration: number }

async function seconds(file: string): Promise<number> {
  const { stderr } = await run(ffmpeg, ['-hide_banner', '-i', file]).catch((error) => error as { stderr: string });
  const match = /Duration: (\d+):(\d+):([\d.]+)/.exec(String(stderr));
  if (!match) throw new Error(`No duration for ${file}`);
  return Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3]);
}

/** Two spoken sentences with a silence between them, muxed under a test picture. */
async function speech(directory: string): Promise<Speech> {
  await mkdir(directory, { recursive: true });
  const a = path.join(directory, 'a.aiff'), b = path.join(directory, 'b.aiff');
  await run('/usr/bin/say', ['-o', a, SENTENCE_A]);
  await run('/usr/bin/say', ['-o', b, SENTENCE_B]);
  const audio = path.join(directory, 'speech.wav');
  await run(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-y', '-i', a, '-i', b, '-filter_complex',
    `[0:a]aresample=48000,aformat=channel_layouts=mono[a];[1:a]aresample=48000,aformat=channel_layouts=mono[b];anullsrc=r=48000:cl=mono,atrim=duration=${GAP}[s];[a][s][b]concat=n=3:v=0:a=1[out]`,
    '-map', '[out]', audio]);
  const file = path.join(directory, 'speech.mp4');
  await run(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=size=1280x720:rate=30', '-i', audio,
    '-shortest', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-g', '30', '-c:a', 'aac', '-b:a', '128k', file]);
  const first = await seconds(a);
  return { file, audio, gapStart: first, gapEnd: first + GAP, duration: await seconds(file) };
}

/** A downloaded model, put where the app's ModelStore looks for it. */
async function installModel(userData: string): Promise<void> {
  const root = path.join(userData, 'models', 'transcription');
  await mkdir(root, { recursive: true });
  await symlink(parakeetDir!, path.join(root, 'parakeet-tdt-0.6b-v3'));
  await writeFile(path.join(root, 'settings.json'), JSON.stringify({ activeModelId: 'parakeet-tdt-0.6b-v3', language: 'auto' }));
}

async function open(session: LaunchedApp, duration = 20): Promise<void> {
  await session.openEditor();
  await session.app.evaluate(({ BrowserWindow }) => {
    for (const window of BrowserWindow.getAllWindows()) window.webContents.setAudioMuted(true);
  });
  await session.page.evaluate((dur) => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Media Intelligence', w: 1280, h: 720, dur });
    project.layers = [];
    PM.replaceProject(project);
  }, duration);
}

async function importMedia(page: Page, file: string): Promise<{ layer: string; asset: string }> {
  const before = await page.evaluate(() => (window as any).PM.proj.layers.map((layer: any) => layer.id));
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(file);
  await page.waitForFunction((known) => (window as any).PM.proj.layers.some((l: any) => l.type === 'video' && !known.includes(l.id) && l.d.asset),
    before, { timeout: 30_000 });
  return page.evaluate((known) => {
    const layer = (window as any).PM.proj.layers.find((l: any) => l.type === 'video' && !known.includes(l.id));
    return { layer: layer.id, asset: layer.d.asset };
  }, before);
}

const normal = (text: string) => text.toLowerCase().replace(/[^\p{L}\p{N}']+/gu, '');

interface Word { text: string; start: number; end: number }

/** Every word of every cue of the captions layer, in composition time. */
const captionWords = (page: Page, id: string) => page.evaluate((id) => {
  const layer = (window as any).PM.L(id);
  return layer.d.cues.flatMap((cue: any) => (cue.words?.length ? cue.words : [{ text: cue.text, start: cue.start, end: cue.end }])
    .map((word: any) => ({ text: word.text, start: layer.from + word.start, end: layer.from + word.end })));
}, id) as Promise<Word[]>;

const captionsLayers = (page: Page) => page.evaluate(() => (window as any).PM.proj.layers
  .map((layer: any, index: number) => ({ layer, index }))
  .filter(({ layer }: any) => layer.type === 'captions')
  .map(({ layer, index }: any) => ({ id: layer.id, index, from: layer.from, dur: layer.dur, text: layer.d.cues.map((cue: any) => cue.text).join(' '),
    cues: layer.d.cues.map((cue: any) => ({ start: cue.start, end: cue.end, text: cue.text })) })));

/**
 * Words the clip plays (away from its edges) appear in the captions at the
 * composition time the clip plays them.
 */
function expectMapped(reference: Word[], actual: Word[], clip: { from: number; trim: number; speed: number; dur: number }, tolerance = 0.3): void {
  const shown = (time: number) => (time - clip.trim) / clip.speed + clip.from;
  const visible = reference.filter((word) => shown(word.start) >= clip.from + 0.25 && shown(word.end) <= clip.from + clip.dur - 0.25);
  expect(visible.length).toBeGreaterThan(3);
  for (const word of visible) {
    const expected = shown(word.start);
    const match = actual.find((item) => normal(item.text) === normal(word.text) && Math.abs(item.start - expected) <= tolerance);
    expect(match, `“${word.text}” at ${expected.toFixed(2)}s in ${JSON.stringify(actual.map((item) => [item.text, +item.start.toFixed(2)]))}`).toBeTruthy();
  }
  for (const word of actual) {
    expect(word.start).toBeGreaterThanOrEqual(clip.from - 1e-6);
    expect(word.end).toBeLessThanOrEqual(clip.from + clip.dur + 1e-6);
  }
}

async function shot(page: Page, name: string, target?: string): Promise<void> {
  if (!shots) return;
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await page.waitForTimeout(300);
  const file = path.join(shots, `${name}.png`);
  if (target) await page.locator(target).screenshot({ path: file });
  else await page.screenshot({ path: file });
}

async function scheme(page: Page, mode: 'light' | 'dark'): Promise<void> {
  await page.evaluate((mode) => (window as any).PM.Kernel.setScheme(mode), mode);
  await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme === 'dark')).toBe(mode === 'dark');
}

async function showCaptionsPanel(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.WS.mutate((workspace: any) => { if (!PM.Layout.hasPanel(workspace, 'captions')) PM.Layout.addPanel(workspace, 'captions', 'right'); });
  });
  await expect(page.locator('#panel-captions')).toBeVisible();
}

test.describe('with a real speech model', () => {
  test.skip(!hasModel, 'POWERMOVE_E2E_PARAKEET_DIR is not set');

  test('Generate Captions puts the spoken words on the caption track in composition time', async () => {
    test.setTimeout(300_000);
    const home = await mkdtemp(path.join(os.tmpdir(), 'pm-mi-captions-'));
    let session: LaunchedApp | null = null;
    try {
      const media = await speech(path.join(home, 'media'));
      await installModel(path.join(home, 'profile'));
      session = await launchApp({ userData: path.join(home, 'profile') });
      await open(session);
      const { page } = session;
      expect((await page.evaluate(() => (window as any).PM.Transcription.status())).activeModelId).toBe('parakeet-tdt-0.6b-v3');
      const clip = await importMedia(page, media.file);

      // The reference: the engine's own words for the whole source file.
      const reference: Word[] = (await page.evaluate(async (file) => (await (window as any).PM.Transcription.transcribe({ path: file }))
        .segments.flatMap((segment: any) => segment.words), media.file));
      const spoken = reference.map((word) => normal(word.text)).join(' ');
      expect(spoken).toContain('quick brown fox jumps over the lazy dog');
      expect(spoken).toContain('captions appear on the timeline');

      // 1. The clip as imported, generated from the Captions panel's menu.
      await page.evaluate((id) => (window as any).PM.selectLayers([id]), clip.layer);
      await showCaptionsPanel(page);
      await page.locator('#panel-captions [aria-label="Caption actions"]').click();
      await page.getByRole('menuitem', { name: 'Generate Captions' }).click();
      await expect.poll(async () => (await captionsLayers(page)).length, { timeout: 120_000 }).toBe(1);
      const [generated] = await captionsLayers(page);
      expect(generated!.index).toBe(0);
      expect(normal(generated!.text.replace(/\s+/g, ' '))).toBe(normal(reference.map((word) => word.text).join(' ')));
      expect(generated!.text).toMatch(/quick brown fox jumps over the lazy dog/i);
      // No cue spans the silence between the sentences.
      for (const cue of generated!.cues) expect(cue.start >= media.gapEnd - 0.4 || cue.end <= media.gapStart + 0.4, JSON.stringify(cue)).toBe(true);
      expectMapped(reference, await captionWords(page, generated!.id), { from: 0, trim: 0, speed: 1, dur: media.duration });
      // It sits on the caption track, above the video track.
      await page.locator('#tl-mode [data-mode="tracks"]').click();
      const geometry = await page.evaluate((id) => {
        const timeline = (window as any).PM.Kernel.services.get('timeline');
        const item = timeline.trackView.layout().byId.get(id);
        return { area: item.area, captionY: timeline.trackView.laneY('captions', 0), videoY: timeline.trackView.laneY('video', 0) };
      }, generated!.id);
      expect(geometry.area).toBe('captions');
      expect(geometry.captionY).toBeLessThan(geometry.videoY);
      await expect(page.locator('#panel-captions .captions-row')).toHaveCount(generated!.cues.length);
      if (shots) {
        await page.evaluate(({ captions }) => {
          const PM = (window as any).PM;
          PM.selectLayers([captions]);
          PM.setTime(1.2, { force: true });
          PM.Kernel.services.get('timeline').frameView();
        }, { captions: generated!.id });
        for (const mode of ['light', 'dark'] as const) {
          await scheme(page, mode);
          await shot(page, `generated-captions-editor-${mode}`);
          await shot(page, `generated-captions-timeline-${mode}`, '#panel-timeline');
          await shot(page, `captions-panel-${mode}`, '#panel-captions');
        }
        await scheme(page, 'light');
      }
      // Undo removes the generated layer in one step.
      await page.evaluate(() => (window as any).PM.hist.undo());
      await expect.poll(async () => (await captionsLayers(page)).length).toBe(0);

      // 2. Offset and trimmed: only what the clip plays, where it plays it.
      const trimmed = { from: 3, trim: 1.2, speed: 1, dur: 5.5 };
      await page.evaluate(({ id, clip }) => {
        const PM = (window as any).PM;
        for (const command of [
          { type: 'set_layer', target: id, patch: { from: clip.from, duration: clip.dur } },
          { type: 'set_content', target: id, patch: { trim: clip.trim } }
        ]) { const result = PM.Edit.apply(command); if (!result.ok) throw new Error(result.message); }
      }, { id: clip.layer, clip: trimmed });
      expect(await page.evaluate((id) => { const l = (window as any).PM.L(id); return [l.from, l.dur, l.d.trim]; }, clip.layer)).toEqual([trimmed.from, trimmed.dur, trimmed.trim]);
      const offset = await page.evaluate((id) => (window as any).PM.Captions.generate([id]), clip.layer);
      expect(offset).toMatchObject({ ok: true, status: 'ok' });
      expectMapped(reference, await captionWords(page, offset.layerId), trimmed);
      const offsetWords = await captionWords(page, offset.layerId);
      expect(offsetWords.map((word) => normal(word.text))).not.toContain('quick');
      await page.evaluate(() => (window as any).PM.hist.undo());

      // 3. Played at 2x: words land at half their source times.
      const fast = { from: 2, trim: 0, speed: 2, dur: media.duration / 2 };
      await page.evaluate(({ id, clip }) => {
        const PM = (window as any).PM;
        for (const command of [
          { type: 'set_content', target: id, patch: { trim: clip.trim, speed: clip.speed } },
          { type: 'set_layer', target: id, patch: { from: clip.from, duration: clip.dur } }
        ]) { const result = PM.Edit.apply(command); if (!result.ok) throw new Error(result.message); }
      }, { id: clip.layer, clip: fast });
      const doubled = await page.evaluate((id) => (window as any).PM.Captions.generate([id]), clip.layer);
      expect(doubled).toMatchObject({ ok: true, status: 'ok' });
      expectMapped(reference, await captionWords(page, doubled.layerId), fast, 0.2);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally {
      await session?.close();
      await rm(home, { recursive: true, force: true });
    }
  });

  test('the agent watches, listens to and captions the clip through the live tool bridge', async () => {
    test.setTimeout(300_000);
    const home = await mkdtemp(path.join(os.tmpdir(), 'pm-mi-agent-'));
    let session: LaunchedApp | null = null;
    const provider = await fakeProvider((tools, state) => {
      const at = tools.length;
      if (at === 0) return ['get_project_state', {}];
      if (at === 1) {
        const project = tools[0]!.json;
        state.asset = project.mediaAssets[0].id;
        state.layer = project.layers.find((item: any) => item.type === 'video').id;
      }
      const sequence: [string, any][] = [
        ['get_project_state', {}],
        ['probe_media', { assetId: state.asset }],
        ['sample_media_frames', { layerId: state.layer, auto: true }],
        ['media_contact_sheet', { assetId: state.asset, count: 4 }],
        ['media_waveform', { layerId: state.layer, image: false }],
        ['transcribe_media', { layerId: state.layer, format: 'words' }],
        ['check_project', {}],
        ['generate_captions', { layerIds: [state.layer], style: 'boxed' }]
      ];
      if (at < sequence.length) return sequence[at]!;
      const last = tools.at(-1)!;
      if (last.json?.status === 'running') return ['generate_captions', { jobId: last.json.jobId }];
      if (!state.exported && last.json?.layerId) { state.captions = last.json.layerId; state.exported = true; return ['export_captions', { layerId: state.captions, format: 'vtt' }]; }
      return null;
    });
    try {
      const media = await speech(path.join(home, 'media'));
      await installModel(home);
      await writeFile(path.join(home, 'agent-provider.json'), JSON.stringify({ baseUrl: `${provider.origin}/v1`, model: 'test-editor', vision: true, hasKey: false }));
      session = await launchApp({ userData: home });
      await open(session);
      const { page } = session;
      const clip = await importMedia(page, media.file);
      const placed = { from: 1, trim: 0, speed: 1, dur: media.duration };
      await page.evaluate(({ id, from }) => (window as any).PM.Edit.apply({ type: 'set_layer', target: id, patch: { from } }), { id: clip.layer, from: placed.from });
      const reference: Word[] = (await page.evaluate(async (file) => (await (window as any).PM.Transcription.transcribe({ path: file }))
        .segments.flatMap((segment: any) => segment.words), media.file));

      await session.openAgent();
      await page.evaluate(() => {
        const w = window as any;
        w.PM.AgentUI.setAccess('project');
        w.PM.AgentUI.setProvider('compatible');
        w.PM.SpatialAssistant.open();
        w.PM.AgentUI.submit('Watch and listen to the clip, then caption it.');
      });
      await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.phase), { timeout: 240_000 }).toBe('result');

      const results = provider.lastTools();
      const byName = (name: string) => results.filter((item) => item.name === name);
      const [probe] = byName('probe_media'), [frames] = byName('sample_media_frames'), [sheet] = byName('media_contact_sheet');
      const [waveform] = byName('media_waveform'), [transcript] = byName('transcribe_media'), [check] = byName('check_project');
      const captions = byName('generate_captions').at(-1)!, [exported] = byName('export_captions');
      expect(probe!.json).toMatchObject({ asset: { id: clip.asset, name: 'speech.mp4' }, video: [{ codec: 'h264', width: 1280, height: 720 }], audio: [{ codec: 'aac' }] });
      expect(frames!.ok).toBe(true);
      expect(frames!.json.timeBase).toBe('composition');
      expect(frames!.json.frames.length).toBeGreaterThan(0);
      for (const frame of frames!.json.frames) {
        expect(frame.time).toBeGreaterThanOrEqual(placed.from - 1e-6);
        expect(frame.time).toBeLessThanOrEqual(placed.from + placed.dur);
      }
      expect(sheet!.json).toMatchObject({ timeBase: 'source' });
      // The silence between the sentences, found in composition time.
      const silences: [number, number][] = waveform!.json.compositionSilences;
      const gap = silences.find(([start, end]) => start < placed.from + media.gapEnd && end > placed.from + media.gapStart);
      expect(gap, JSON.stringify(silences)).toBeTruthy();
      expect(gap![0]).toBeCloseTo(placed.from + media.gapStart, 0);
      expect(gap![1]).toBeCloseTo(placed.from + media.gapEnd, 0);
      // transcribe_media on the layer: composition-time words.
      expect(transcript!.ok).toBe(true);
      expect(transcript!.json.timeBase).toBe('composition');
      const words: Word[] = transcript!.json.segments.flatMap((segment: any) => segment.words.map(([text, start, end]: [string, number, number]) => ({ text, start, end })));
      expectMapped(reference, words, placed, 0.15);
      expect(check!.ok).toBe(true);
      // generate_captions → a captions layer on top, styled, words in composition time.
      expect(captions.ok, captions.error).toBe(true);
      expect(captions.json.status).toBe('done');
      const layers = await captionsLayers(page);
      expect(layers).toHaveLength(1);
      expect(layers[0]).toMatchObject({ id: captions.json.layerId, index: 0 });
      expect(await page.evaluate((id) => (window as any).PM.L(id).d.style?.preset, layers[0]!.id)).toBe('boxed');
      expectMapped(reference, await captionWords(page, layers[0]!.id), placed);
      // export_captions reads it as WebVTT in composition time without changing the project.
      expect(exported!.ok).toBe(true);
      expect(exported!.json.format).toBe('vtt');
      expect(exported!.json.text).toMatch(/^WEBVTT/);
      expect(exported!.json.text).toMatch(/quick brown fox/i);
      const firstCue = /(\d\d):(\d\d):(\d\d)\.(\d\d\d) -->/.exec(exported!.json.text)!;
      expect(Number(firstCue[3]) + Number(firstCue[4]) / 1000).toBeCloseTo(layers[0]!.cues[0]!.start + layers[0]!.from, 2);

      // The agent's activity rows name each media tool.
      const rows = await page.evaluate(() => (window as any).PM.AgentUI.state.conversation
        .flatMap((message: any) => message.role === 'trace' ? message.steps || [] : [])
        .filter((step: any) => step.kind === 'tool').map((step: any) => step.label));
      expect(rows).toEqual(expect.arrayContaining(['Probing speech.mp4…', 'Mapping silences in speech.mp4…', 'Checking the project…',
        'Captioning speech.mp4…', 'Exporting speech.mp4 Captions as WebVTT…']));
      if (shots) {
        await session.openAgent();
        await page.evaluate(() => { for (const details of document.querySelectorAll('details.agent-tool-activity')) (details as HTMLDetailsElement).open = true; });
        for (const mode of ['light', 'dark'] as const) {
          await scheme(page, mode);
          await shot(page, `agent-media-rows-${mode}`);
        }
      }
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally {
      await session?.close();
      await provider.close();
      await rm(home, { recursive: true, force: true });
    }
  });
});

test('without a model, Generate Captions asks for one and dismissing changes nothing', async ({ session }) => {
  test.setTimeout(120_000);
  const home = await mkdtemp(path.join(os.tmpdir(), 'pm-mi-nomodel-'));
  try {
    const media = await speech(home);
    await open(session);
    const { page } = session;
    const clip = await importMedia(page, media.file);
    await page.evaluate((id) => (window as any).PM.selectLayers([id]), clip.layer);
    const before = await page.evaluate(() => {
      const PM = (window as any).PM;
      return { project: JSON.stringify(PM.proj.layers), history: PM.hist.list().length, revision: PM.proj.revision };
    });
    await page.evaluate(() => (window as any).PM.cmd('generateCaptions'));
    const sheet = page.locator('.transcription-modal');
    await expect(sheet).toBeVisible();
    await expect(sheet.getByText(/Captions for “speech\.mp4”|speech\.mp4/)).toBeVisible();
    await expect(sheet.getByRole('radio', { name: /Parakeet V3/ })).toBeChecked();
    await sheet.getByRole('button', { name: 'Not Now' }).click();
    await expect(sheet).toBeHidden();
    await expect.poll(() => page.evaluate(() => (window as any).PM.hist.list().length)).toBe(before.history);
    expect(await page.evaluate(() => {
      const PM = (window as any).PM;
      return { project: JSON.stringify(PM.proj.layers), history: PM.hist.list().length, revision: PM.proj.revision };
    })).toEqual(before);
    expect(await captionsLayers(page)).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await rm(home, { recursive: true, force: true });
  }
});

test('without a model, the agent\'s transcribe_media and generate_captions say so and the sheet opens', async () => {
  test.setTimeout(150_000);
  const home = await mkdtemp(path.join(os.tmpdir(), 'pm-mi-agent-nomodel-'));
  let session: LaunchedApp | null = null;
  const provider = await fakeProvider((tools, state) => {
    if (tools.length === 0) return ['get_project_state', {}];
    if (tools.length === 1) state.layer = tools[0]!.json.layers.find((item: any) => item.type === 'video').id;
    if (tools.length === 1) return ['transcribe_media', { layerId: state.layer }];
    if (tools.length === 2) return ['generate_captions', { layerIds: [state.layer] }];
    return null;
  });
  try {
    const media = await speech(path.join(home, 'media'));
    await writeFile(path.join(home, 'agent-provider.json'), JSON.stringify({ baseUrl: `${provider.origin}/v1`, model: 'test-editor', vision: true, hasKey: false }));
    session = await launchApp({ userData: home });
    await open(session);
    const { page } = session;
    await importMedia(page, media.file);
    const layers = await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers));
    await session.openAgent();
    await page.evaluate(() => {
      const w = window as any;
      w.PM.AgentUI.setAccess('project');
      w.PM.AgentUI.setProvider('compatible');
      w.PM.SpatialAssistant.open();
      w.PM.AgentUI.submit('Transcribe the clip.');
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.phase), { timeout: 90_000 }).not.toBe('running');
    const results = provider.lastTools();
    const transcript = results.find((item) => item.name === 'transcribe_media')!;
    expect(transcript.ok).toBe(false);
    expect(JSON.parse(transcript.error!)).toMatchObject({ status: 'model-required', code: 'transcription-model-missing' });
    const captions = results.find((item) => item.name === 'generate_captions')!;
    expect(captions.ok).toBe(false);
    expect(captions.error).toContain('transcription-model-missing');
    const sheet = page.locator('.transcription-modal');
    await expect(sheet).toBeVisible();
    await expect(sheet.getByText(/The agent wants to/)).toBeVisible();
    await sheet.getByRole('button', { name: 'Not Now' }).click();
    await expect(sheet).toBeHidden();
    expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(layers);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session?.close();
    await provider.close();
    await rm(home, { recursive: true, force: true });
  }
});

const SRT = [
  '1', '00:00:00,500 --> 00:00:02,000', 'Every frame tells a story.', '',
  '2', '00:00:02,200 --> 00:00:04,000', 'Captions make it heard.', ''
].join('\r\n');

test('an imported SRT edits, round-trips as SRT and WebVTT, burns into a frame and rides along an export', async ({ session }) => {
  test.setTimeout(150_000);
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-mi-srt-'));
  try {
    await open(session, 6);
    const { page, app } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const backdrop = PM.mkLayer('solid', { name: 'Backdrop', d: { color: '#204060' } }, PM.proj);
      PM.proj.layers = [backdrop];
      PM.bus.emit('layers');
    });
    // Import through the Captions service, as File › Import Captions… would.
    const id = await page.evaluate(async (text) => (window as any).PM.Captions.importFile(new File([text], 'Talk.en.srt')), SRT);
    expect(id).toBeTruthy();
    const read = () => page.evaluate((id) => (window as any).PM.L(id).d.cues.map((cue: any) => [cue.start, cue.end, cue.text]), id);
    expect(await read()).toEqual([[0.5, 2, 'Every frame tells a story.'], [2.2, 4, 'Captions make it heard.']]);
    // Edit a cue's text and timing through the edit command the panel uses.
    const edited = await page.evaluate((id) => {
      const PM = (window as any).PM, cue = PM.L(id).d.cues[1];
      return PM.Edit.apply({ type: 'edit_captions', target: id, op: 'update', cues: [{ id: cue.id, text: 'Captions — make it “heard”.', end: 4.5 }] });
    }, id);
    expect(edited.ok, edited.message).toBe(true);
    expect((await read())[1]).toEqual([2.2, 4.5, 'Captions — make it “heard”.']);

    const save = async (name: string, format: 'srt' | 'vtt') => {
      const file = path.join(folder, name);
      await app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, file);
      expect(await page.evaluate(({ id, format }) => (window as any).PM.Captions.exportFile(id, format), { id, format })).toBe(true);
      return readFile(file, 'utf8');
    };
    const srt = await save('Talk.srt', 'srt');
    expect(srt).toContain('00:00:02,200 --> 00:00:04,500\nCaptions — make it “heard”.');
    const vtt = await save('Talk.vtt', 'vtt');
    expect(vtt.startsWith('WEBVTT')).toBe(true);
    expect(vtt).toContain('00:00:02.200 --> 00:00:04.500');
    // Both files import back to the same cues.
    const cues = await read();
    for (const [name, text] of [['Back.srt', srt], ['Back.vtt', vtt]] as const) {
      const back = await page.evaluate(async ({ name, text }) => {
        const PM = (window as any).PM;
        const layer = await PM.Captions.importFile(new File([text], name));
        return PM.L(layer).d.cues.map((cue: any) => [cue.start, cue.end, cue.text]);
      }, { name, text });
      expect(back).toEqual(cues);
    }
    await page.evaluate(() => { const PM = (window as any).PM; PM.Edit.apply({ type: 'delete_layers', targets: PM.proj.layers.filter((l: any) => l.type === 'captions').slice(0, 2).map((l: any) => l.id) }); });
    expect((await captionsLayers(page)).map((layer) => layer.id)).toEqual([id]);

    // A still exported through the real save path shows the caption.
    const still = async (hidden: boolean) => {
      const file = path.join(folder, `still-${hidden ? 'clean' : 'burned'}.png`);
      await app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, file);
      const result = await page.evaluate(async (hidden) => {
        const PM = (window as any).PM;
        PM.setTime(3, { raw: true, force: true });
        return PM.Export.run({ format: 'still', w: 640, h: 360, alpha: false, mblur: false, captionsBurn: !hidden });
      }, hidden);
      expect(result).toEqual({ cancelled: false });
      return page.evaluate(async (bytes) => {
        const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
        const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height;
        const context = canvas.getContext('2d')!;
        context.drawImage(image, 0, 0);
        const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
        let bright = 0;
        for (let index = 0; index < data.length; index += 4) if (data[index]! > 200 && data[index + 1]! > 200 && data[index + 2]! > 200) bright++;
        return bright;
      }, [...await readFile(file)]);
    };
    expect(await still(true)).toBe(0);
    expect(await still(false)).toBeGreaterThan(200);

    // An MP4 export writes the caption file beside the video.
    const video = path.join(folder, 'Talk Export.mp4');
    await app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, video);
    const exported = await page.evaluate(() => (window as any).PM.Export.run({ format: 'mp4', scale: 0.25, fps: 30, range: 'all', quality: 'draft', mblur: false, alpha: false, audio: false, captionsSidecar: 'srt' }));
    expect(exported).toMatchObject({ cancelled: false });
    await expect.poll(() => existsSync(path.join(folder, 'Talk Export.srt')), { timeout: 30_000 }).toBe(true);
    expect(await readFile(path.join(folder, 'Talk Export.srt'), 'utf8')).toBe(srt);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
});

test('the Mixer shows the speech clip, its meters move while it plays and its fader is undoable', async ({ session }) => {
  test.setTimeout(120_000);
  const home = await mkdtemp(path.join(os.tmpdir(), 'pm-mi-mixer-'));
  try {
    const media = await speech(home);
    await open(session, 10);
    const { page } = session;
    const clip = await importMedia(page, media.file);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.WS.mutate((workspace: any) => { if (!PM.Layout.hasPanel(workspace, 'mixer')) PM.Layout.addPanel(workspace, 'mixer', 'right'); });
    });
    const panel = page.locator('#panel-mixer');
    await expect(panel).toBeVisible();
    const strip = panel.locator(`[data-mixer-strip="${clip.layer}"]`);
    await expect(strip).toBeVisible();
    await expect(panel.getByRole('slider', { name: 'speech.mp4 level' })).toBeVisible();

    const level = () => page.evaluate((id) => {
      const out = { peak: [0, 0], rms: [0, 0] };
      const read = (window as any).PM.Audio.meters?.read?.(id, out);
      return read ? Math.max(...out.peak) : -1;
    }, clip.layer);
    const pixels = () => strip.locator('canvas.mx-canvas').evaluate((canvas: HTMLCanvasElement) => {
      const copy = document.createElement('canvas'); copy.width = canvas.width; copy.height = canvas.height;
      const context = copy.getContext('2d')!; context.drawImage(canvas, 0, 0);
      const data = context.getImageData(0, 0, copy.width, copy.height).data;
      let sum = 0; for (let index = 0; index < data.length; index += 4) sum += data[index]! + data[index + 1]! * 3 + data[index + 2]!;
      return sum;
    });
    const quiet = await pixels();
    await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0.3); PM.play(); });
    await expect.poll(level, { timeout: 10_000 }).toBeGreaterThan(0.01);
    await expect.poll(pixels, { timeout: 10_000 }).not.toBe(quiet);
    if (shots) {
      for (const mode of ['light', 'dark'] as const) {
        await scheme(page, mode);
        await page.evaluate(() => { const PM = (window as any).PM; if (!PM.playing) { PM.setTime(0.3); PM.play(); } });
        await page.waitForTimeout(600);
        await shot(page, `mixer-playing-${mode}`, '#panel-mixer');
      }
    }
    // In the silence the meter falls back.
    await page.evaluate((time) => { const PM = (window as any).PM; PM.pause(); PM.setTime(time); PM.play(); }, media.gapStart + 0.6);
    await expect.poll(level, { timeout: 5_000 }).toBeLessThan(0.01);
    await page.evaluate(() => (window as any).PM.pause());

    // The fader changes the clip's gain; one Undo step restores unity.
    // A video layer's sound level is its audioGain.
    const gain = () => page.evaluate((id) => { const value = (window as any).PM.L(id).d.audioGain; return typeof value === 'number' ? value : value?.v ?? 1; }, clip.layer);
    expect(await gain()).toBe(1);
    const fader = panel.getByRole('slider', { name: 'speech.mp4 level' });
    await fader.focus();
    await page.keyboard.press('PageDown');
    await expect.poll(gain).toBeCloseTo(10 ** (-6 / 20), 3);
    await page.evaluate(() => (window as any).PM.hist.undo());
    await expect.poll(gain).toBe(1);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await rm(home, { recursive: true, force: true });
  }
});

/* ── a fake OpenAI-compatible model that calls tools in order ── */

interface ToolResult { name: string; ok: boolean; error?: string; json: any }

async function fakeProvider(next: (tools: ToolResult[], state: Record<string, any>) => [string, any] | null): Promise<{ origin: string; lastTools(): ToolResult[]; close(): Promise<void> }> {
  const state: Record<string, any> = {};
  let last: ToolResult[] = [];
  const parse = (message: any, calls: Map<string, string>): ToolResult => {
    const result = JSON.parse(message.content);
    const text = result.content?.find((item: any) => item.type === 'text')?.text;
    let json: any = null;
    try { json = text ? JSON.parse(text) : null; } catch { json = text; }
    return { name: calls.get(message.tool_call_id) ?? '', ok: result.ok, error: result.error, json };
  };
  const server: Server = createServer(async (req, res) => {
    try {
      if (req.method === 'GET') { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ data: [{ id: 'test-editor' }] })); return; }
      let input = '';
      for await (const chunk of req) input += chunk.toString();
      const body = JSON.parse(input);
      if (!body.stream) { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ choices: [{ message: { content: 'OK' } }] })); return; }
      const calls = new Map<string, string>();
      for (const message of body.messages) for (const call of message.tool_calls ?? []) calls.set(call.id, call.function.name);
      const tools = body.messages.filter((message: any) => message.role === 'tool').map((message: any) => parse(message, calls));
      last = tools;
      const step = next(tools, state) ?? ['complete_task', { summary: 'Done.', commands: [], artifacts: [], extensions: [], notes: [], externalActions: [] }];
      res.writeHead(200, { 'Content-Type': 'text/event-stream' });
      res.end(`data: ${JSON.stringify({ choices: [{ delta: { tool_calls: [{ index: 0, id: `call-${tools.length}`, function: { name: step[0], arguments: JSON.stringify(step[1]) } }] }, finish_reason: 'tool_calls' }] })}\n\n`);
    } catch (error) { res.writeHead(500); res.end(String(error)); }
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  return {
    origin: `http://127.0.0.1:${(server.address() as { port: number }).port}`,
    lastTools: () => last,
    close: async () => { server.closeAllConnections(); await new Promise<void>((resolve) => server.close(() => resolve())); }
  };
}
