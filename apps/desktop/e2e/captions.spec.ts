import { mkdir, mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test } from './helpers/app';

/* Screenshots are review material, never committed: they go outside the repo. */
const SHOTS = process.env.PM_CAPTION_SHOTS || '/tmp/pm-captions';

const SRT = [
  '1', '00:00:00,500 --> 00:00:02,000', 'Every frame tells a story.', '',
  '2', '00:00:02,200 --> 00:00:04,000', 'Captions make it heard.', '',
  '3', '00:00:04,400 --> 00:00:06,500', '<i>Even</i> in a quiet room.', ''
].join('\r\n');

test.beforeEach(async ({ session }) => { await session.openEditor(); });

async function seed(page: import('playwright').Page) {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Captions E2E', w: 1280, h: 720, dur: 8 });
    const background = PM.mkLayer('solid', { name: 'Backdrop', d: { color: '#2B3A4A' } }, project);
    const clip = PM.mkLayer('text', { name: 'Title', d: { text: 'Powermove', size: 96 } }, project);
    project.layers = [clip, background];
    PM.replaceProject(project);
    PM.selectLayers([]);
    PM.Kernel.services.get('timeline').frameView();
  });
}

/** Drop a subtitle file on the timeline at a composition time, the way Finder would. */
async function dropSubtitles(page: import('playwright').Page, name: string, text: string, time: number) {
  await page.evaluate(({ name, text, time }) => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const canvas: HTMLCanvasElement = timeline.cv;
    const rect = canvas.getBoundingClientRect();
    const transfer = new DataTransfer();
    transfer.items.add(new File([text], name, { type: '' }));
    const clientX = rect.left + timeline.gut + (time - timeline.scrollT) * timeline.pps;
    const clientY = rect.top + timeline.ruler + 12;
    for (const type of ['dragenter', 'dragover', 'drop']) {
      canvas.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: transfer, clientX, clientY }));
    }
  }, { name, text, time });
}

const captionsLayer = (page: import('playwright').Page) => page.evaluate(() => {
  const layer = (window as any).PM.proj.layers.find((item: any) => item.type === 'captions');
  return layer ? { id: layer.id, name: layer.name, index: (window as any).PM.proj.layers.indexOf(layer), from: layer.from, language: layer.d.language ?? null,
    cues: layer.d.cues.map((cue: any) => ({ id: cue.id, start: cue.start, end: cue.end, text: cue.text })) } : null;
});

test('subtitles import onto a caption track, edit in place, burn into frames and export back', async ({ session }) => {
  const { page, app } = session;
  await mkdir(SHOTS, { recursive: true });
  await seed(page);

  // 1. Import: an .srt dropped on the timeline becomes a captions layer on top.
  await dropSubtitles(page, 'Interview.en.srt', SRT, 0);
  await expect.poll(() => captionsLayer(page)).not.toBeNull();
  const imported = (await captionsLayer(page))!;
  expect(imported.index).toBe(0);
  expect(imported.name).toBe('Interview');
  expect(imported.language).toBe('en');
  expect(imported.cues.map(cue => cue.text)).toEqual(['Every frame tells a story.', 'Captions make it heard.', 'Even in a quiet room.']);

  // 2. The track timeline shows it as caption track C1, above V1.
  await page.locator('#tl-mode [data-mode="tracks"]').click();
  const geometry = await page.evaluate((id) => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const item = timeline.trackView.layout().byId.get(id);
    return { area: item.area, lane: item.lane, captionY: timeline.trackView.laneY('captions', 0), videoY: timeline.trackView.laneY('video', 0) };
  }, imported.id);
  expect(geometry).toMatchObject({ area: 'captions', lane: 0 });
  expect(geometry.captionY).toBeLessThan(geometry.videoY);

  // 3. Double-click the second cue on its track and retype it.
  const cuePoint = await page.evaluate((id) => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    const layer = PM.L(id), cue = layer.d.cues[1];
    const rect = timeline.cv.getBoundingClientRect();
    return {
      x: rect.left + timeline.gut + (layer.from + (cue.start + cue.end) / 2 - timeline.scrollT) * timeline.pps,
      y: rect.top + timeline.trackView.laneY('captions', 0) + timeline.trackView.rowHeight() / 2,
    };
  }, imported.id);
  await page.mouse.dblclick(cuePoint.x, cuePoint.y);
  const editor = page.locator('#tl-canvas-wrap .tl-caption-editor');
  await expect(editor).toBeVisible();
  await expect(editor).toHaveValue('Captions make it heard.');
  await editor.fill('Captions make every word heard.');
  await editor.press('Enter');
  await expect(editor).toHaveCount(0);
  await expect.poll(async () => (await captionsLayer(page))!.cues[1]!.text).toBe('Captions make every word heard.');
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(async () => (await captionsLayer(page))!.cues[1]!.text).toBe('Captions make it heard.');
  await page.evaluate(() => (window as any).PM.hist.redo());
  await expect.poll(async () => (await captionsLayer(page))!.cues[1]!.text).toBe('Captions make every word heard.');

  // 4. A rendered frame inside the cue shows the caption; the same frame
  //    without burned-in captions does not.
  const frames = await page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.setTime(3, { force: true });
    const shot = async () => PM.Export.snapshotAsync ? PM.Export.snapshotAsync(3, 960) : PM.Export.snapshot(3, 960);
    const shown = await shot();
    PM.captionsHidden = true;
    const hidden = await shot();
    PM.captionsHidden = false;
    return { shown, hidden };
  });
  expect(frames.shown).not.toBe(frames.hidden);
  const framePath = path.join(SHOTS, 'caption-frame.png');
  await (await import('node:fs/promises')).writeFile(framePath, Buffer.from(String(frames.shown).split(',')[1]!, 'base64'));

  // 5. Export SRT through the real save path, then read it back.
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-captions-export-'));
  try {
    const destination = path.join(folder, 'Interview.srt');
    await app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, destination);
    expect(await page.evaluate((id) => (window as any).PM.Captions.exportFile(id, 'srt'), imported.id)).toBe(true);
    const written = await readFile(destination, 'utf8');
    expect(written).toBe([
      '1', '00:00:00,500 --> 00:00:02,000', 'Every frame tells a story.', '',
      '2', '00:00:02,200 --> 00:00:04,000', 'Captions make every word heard.', '',
      '3', '00:00:04,400 --> 00:00:06,500', 'Even in a quiet room.', ''
    ].join('\n'));
    // Round trip: importing the exported file reproduces the same cues.
    await dropSubtitles(page, 'Roundtrip.srt', written, 0);
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.filter((layer: any) => layer.type === 'captions').length)).toBe(2);
    const roundTrip = await page.evaluate(() => (window as any).PM.proj.layers[0].d.cues.map((cue: any) => [cue.start, cue.end, cue.text]));
    expect(roundTrip).toEqual((await captionsLayer(page))!.cues.map(cue => [cue.start, cue.end, cue.text]));
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
});

test('timeline cue gestures keep neighbours apart and Delete removes selected cues', async ({ session }) => {
  const { page } = session;
  await seed(page);
  await dropSubtitles(page, 'Lines.srt', SRT, 0);
  await expect.poll(() => captionsLayer(page)).not.toBeNull();
  const { id } = (await captionsLayer(page))!;
  const point = (time: number) => page.evaluate(({ id, time }) => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    const rect = timeline.cv.getBoundingClientRect();
    const index = timeline.rows.findIndex((row: any) => row.kind === 'layer' && row.L.id === id);
    return { x: rect.left + timeline.gut + (time - timeline.scrollT) * timeline.pps, y: rect.top + timeline.ruler + index * timeline.row - timeline.scrollY + timeline.row / 2 };
  }, { id, time });

  // Layer timeline: drag cue 1 far right; it stops at cue 2.
  const start = await point(1.2);
  const end = await point(5);
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(start.x + 10, start.y, { steps: 2 });
  await page.mouse.move(end.x, end.y, { steps: 8 });
  await page.mouse.up();
  const moved = (await captionsLayer(page))!.cues;
  expect(moved[0]!.end).toBeLessThanOrEqual(moved[1]!.start + 1e-6);
  expect(moved[0]!.start).toBeGreaterThan(0.5);
  expect(await page.evaluate(() => (window as any).PM.Captions.selection())).toMatchObject({ layerId: id, cues: [moved[0]!.id] });

  // Delete removes the selected cue, not the layer; Undo brings it back.
  await page.keyboard.press('Backspace');
  await expect.poll(async () => (await captionsLayer(page))?.cues.length).toBe(2);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(async () => (await captionsLayer(page))?.cues.length).toBe(3);

  // Clicking the layer's empty lane selects the layer instead of the cue, so
  // Delete removes the captions layer rather than a cue the user moved past.
  const cue = await point(3);
  await page.mouse.click(cue.x, cue.y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Captions.selection().cues.length)).toBe(1);
  const gap = await point(4.2);
  await page.mouse.click(gap.x, gap.y);
  expect(await page.evaluate(() => (window as any).PM.Captions.selection())).toEqual({ layerId: null, cues: [] });
  await page.keyboard.press('Backspace');
  await expect.poll(() => captionsLayer(page)).toBeNull();
});
