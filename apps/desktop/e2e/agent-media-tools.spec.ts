import { execFile } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import { expect, launchApp, repoRoot, test } from './helpers/app';

/*
 * The agent watches and listens to real footage through the whole stack: a
 * fake OpenAI-compatible model calls the media tools, main resolves the
 * imported clip through the renderer, runs the bundled ffmpeg, and the
 * transcription seam (no model downloaded) answers model-required.
 */

const run = promisify(execFile);
const ffmpeg = path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg');
const SCREENSHOTS = process.env.POWERMOVE_MEDIA_TOOLS_SHOTS;

async function footage(directory: string): Promise<string> {
  await mkdir(directory, { recursive: true });
  const file = path.join(directory, 'interview.mp4');
  await run(ffmpeg, [
    '-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'lavfi', '-i', 'color=c=0x2050c0:s=640x360:r=30:d=4',
    '-f', 'lavfi', '-i', 'smptebars=s=640x360:r=30:d=4',
    '-f', 'lavfi', '-i', 'sine=f=440:d=8:sample_rate=48000',
    '-filter_complex', "[0][1]concat=n=2:v=1:a=0[v];[2]volume=enable='between(t,2,3.5)+between(t,5.5,6.2)':volume=0,volume=4[a]",
    '-map', '[v]', '-map', '[a]', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-g', '30', '-c:a', 'aac', file
  ]);
  return file;
}

const toolResult = (message: any) => {
  const result = JSON.parse(message.content);
  const text = result.content?.find((item: any) => item.type === 'text')?.text;
  return { ok: result.ok, error: result.error as string | undefined, json: text ? JSON.parse(text) : null };
};

test('the agent watches and listens to imported footage through the media tools', async () => {
  test.setTimeout(150_000);
  const home = await mkdtemp(path.join(os.tmpdir(), 'powermove-media-tools-e2e-'));
  const media = await footage(path.join(home, 'footage'));
  let session: Awaited<ReturnType<typeof launchApp>> | undefined;
  const seen: any[] = [];
  let asset = '', layer = '';
  const server = createServer(async (req, res) => {
    try {
      if (req.method === 'GET') { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ data: [{ id: 'test-editor' }] })); return; }
      let input = '';
      for await (const chunk of req) input += chunk.toString();
      const body = JSON.parse(input);
      if (!body.stream) { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ choices: [{ message: { content: 'OK' } }] })); return; }
      seen.push(body);
      const tools = body.messages.filter((message: any) => message.role === 'tool');
      if (tools.length === 1) {
        const state = toolResult(tools[0]).json;
        asset = state.mediaAssets[0].id;
        layer = state.layers.find((item: any) => item.type === 'video').id;
      }
      const result = { summary: 'Watched and listened to the interview.', commands: [], artifacts: [], extensions: [], notes: [], externalActions: [] };
      const prompt = JSON.stringify(body.messages.filter((message: any) => message.role === 'user').at(-1)?.content ?? '');
      const followUp = prompt.includes('Look at the middle');
      const sequence: [string, any][] = followUp ? [['sample_media_frames', { layerId: layer, count: 3, start: 2, end: 6 }], ['complete_task', result]] : [
        ['get_project_state', {}],
        ['probe_media', { assetId: asset }],
        ['sample_media_frames', { layerId: layer, auto: true }],
        ['media_contact_sheet', { assetId: asset, count: 6 }],
        ['media_waveform', { layerId: layer, image: false }],
        ['transcribe_media', { layerId: layer }],
        ['media_contact_sheet', { target: 'composition', count: 4 }],
        ['check_project', {}],
        ['complete_task', result]
      ];
      const [name, args] = sequence[tools.length] || ['complete_task', result];
      res.writeHead(200, { 'Content-Type': 'text/event-stream' });
      res.end(`data: ${JSON.stringify({ choices: [{ delta: { tool_calls: [{ index: 0, id: `call-${tools.length}`, function: { name, arguments: JSON.stringify(args) } }] }, finish_reason: 'tool_calls' }] })}\n\n`);
    } catch (error) { res.writeHead(500); res.end(String(error)); }
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const port = (server.address() as { port: number }).port;
    await writeFile(path.join(home, 'agent-provider.json'), JSON.stringify({ baseUrl: `http://127.0.0.1:${port}/v1`, model: 'test-editor', vision: true, hasKey: false }));
    session = await launchApp({ userData: home });
    await session.openEditor();
    const { page } = session;
    const chooser = page.waitForEvent('filechooser');
    await page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(media);
    await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const clip = PM.proj.layers.find((l: any) => l.type === 'video');
      PM.Edit.apply({ type: 'set_layer', target: clip.id, patch: { from: 1 } });
    });
    await session.openAgent();
    await page.evaluate(() => {
      const w = window as any;
      w.PM.AgentUI.setAccess('project');
      w.PM.AgentUI.setProvider('compatible');
      w.PM.SpatialAssistant.open();
      w.PM.AgentUI.submit('Watch and listen to the interview before cutting it.');
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.phase), { timeout: 90_000 }).toBe('result');

    const last = seen.at(-1);
    const results = last.messages.filter((message: any) => message.role === 'tool').map(toolResult);
    const [, probe, frames, sheet, waveform, transcript, compSheet, check] = results;
    expect(probe.json).toMatchObject({ asset: { id: asset, name: 'interview.mp4' }, format: 'mov/mp4', video: [{ codec: 'h264', width: 640, height: 360 }], audio: [{ codec: 'aac' }] });
    expect(frames.json).toMatchObject({ timeBase: 'composition', auto: { distinctStates: 2 } });
    expect(frames.json.frames.map((frame: any) => frame.time)).toEqual([1, 5.5]);
    expect(sheet.json).toMatchObject({ columns: 3, rows: 2, timeBase: 'source' });
    const silences = waveform.json.compositionSilences;
    expect(silences).toHaveLength(2);
    expect(silences[0][0]).toBeCloseTo(3, 1);
    expect(silences[1][1]).toBeCloseTo(7.2, 1);
    expect(transcript.ok).toBe(false);
    expect(JSON.parse(transcript.error!)).toMatchObject({ status: 'model-required', code: 'transcription-model-missing' });
    expect(compSheet.error).toBeUndefined();
    expect(compSheet.json).toMatchObject({ target: 'composition', columns: 2, rows: 2 });
    expect(check.ok).toBe(true);
    expect(check.json.issues.map((issue: any) => issue.code)).toContain('empty-frames');
    const images = last.messages.filter((message: any) => message.role === 'user' && Array.isArray(message.content))
      .flatMap((message: any) => message.content).filter((item: any) => item.image_url?.url?.startsWith('data:image/'));
    expect(images.length).toBe(2 + 1 + 1);

    const rows = await page.evaluate(() => (window as any).PM.AgentUI.state.conversation
      .flatMap((message: any) => message.role === 'trace' ? message.steps || [] : [])
      .filter((step: any) => step.kind === 'tool').map((step: any) => step.label));
    expect(rows).toEqual(expect.arrayContaining([
      'Probing interview.mp4…', 'Finding scene changes in interview.mp4…', 'Building a contact sheet of interview.mp4…',
      'Mapping silences in interview.mp4…', 'Needs a transcription model for interview.mp4', 'Building a contact sheet of the composition…', 'Checking the project…'
    ]));

    if (SCREENSHOTS) {
      await session.openAgent();
      await page.evaluate(() => { for (const details of document.querySelectorAll('details.agent-tool-activity')) (details as HTMLDetailsElement).open = true; });
      for (const theme of ['light', 'dark']) {
        await page.evaluate(theme => (window as any).PM.Kernel.setScheme(theme), theme);
        await page.waitForTimeout(400);
        await page.screenshot({ path: path.join(SCREENSHOTS, `agent-media-rows-${theme}.png`) });
      }

    }

    // With the original file gone, the stored bytes are staged for tools instead.
    // A slowed media store keeps that call running long enough to see its live row.
    await rm(media);
    await page.evaluate(() => {
      const store = (window as any).PM.MediaStore;
      const get = store.get.bind(store);
      store.get = async (value: unknown) => { await new Promise(resolve => setTimeout(resolve, 2500)); return get(value); };
    });
    const before = seen.length;
    await page.evaluate(() => (window as any).PM.AgentUI.submit('Look at the middle of the interview.'));
    await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.trace
      .some((step: any) => step.kind === 'tool' && step.status === 'running' && step.label === 'Sampling 3 frames from interview.mp4…')), { timeout: 20_000 }).toBe(true);
    if (SCREENSHOTS) {
      await session.openAgent();
      for (const theme of ['light', 'dark']) {
        await page.evaluate(theme => (window as any).PM.Kernel.setScheme(theme), theme);
        await page.waitForTimeout(250);
        await page.screenshot({ path: path.join(SCREENSHOTS, `agent-media-rows-live-${theme}.png`) });
      }
    }
    await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.phase), { timeout: 30_000 }).not.toBe('running');
    const followUp = seen.slice(before).at(-1).messages.filter((message: any) => message.role === 'tool').map(toolResult).at(-1);
    expect(followUp.ok).toBe(true);
    expect(followUp.json.frames.map((frame: any) => frame.time)).toEqual([2, 4, 6]);
    const staged = await page.evaluate(async (assetId) => {
      const response = await (window as any).PM.AgentHarness.test.handleLiveAgentTool({ runId: 'staging', callId: 'c', tool: '__media_source', arguments: { assetId }, baseRevision: 0 });
      return JSON.parse(response.content[0].text);
    }, asset);
    expect(staged.origin).toBe('cache');
    expect(staged.path).toContain(path.join(session.userData, 'Media Tool Cache'));
    expect((await stat(staged.path)).size).toBeGreaterThan(1000);
  } finally {
    await session?.close();
    await rm(home, { recursive: true, force: true });
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
  }
});
