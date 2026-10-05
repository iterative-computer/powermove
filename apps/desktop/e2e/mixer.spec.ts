import type { Page } from 'playwright';

import { expect, test, type LaunchedApp } from './helpers/app';
import { fixturePath } from './helpers/media';

/* The suite plays real audio through a hidden window; keep it off the speakers. */
async function openMixerProject(session: LaunchedApp): Promise<{ tone: string; video: string }> {
  await session.openEditor();
  const { page, app } = session;
  await app.evaluate(({ BrowserWindow }) => {
    for (const window of BrowserWindow.getAllWindows()) window.webContents.setAudioMuted(true);
  });
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles([fixturePath('tone.wav'), fixturePath('h264-aac.mp4')]);
  await page.waitForFunction(() => {
    const layers = (window as any).PM.proj.layers;
    return layers.some((l: any) => l.type === 'audio') && layers.some((l: any) => l.type === 'video' && l.d.embeddedAudio === true);
  }, null, { timeout: 30_000 });
  return page.evaluate(() => {
    const PM = (window as any).PM;
    const tone = PM.proj.layers.find((l: any) => l.type === 'audio');
    const video = PM.proj.layers.find((l: any) => l.type === 'video');
    tone.name = 'Tone'; video.name = 'Interview';
    PM.bus.emit('layers');
    return { tone: tone.id, video: video.id };
  });
}

function agentTool(page: Page, tool: string, args: Record<string, unknown>) {
  return page.evaluate(async ({ tool, args }) => {
    const PM = (window as any).PM;
    const result = await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'mixer-e2e', callId: tool, tool, arguments: args, baseRevision: PM.proj.revision || 0 });
    return { ok: result.ok, text: result.content?.[0]?.text ?? '' };
  }, { tool, args });
}

const gain = (page: Page, id: string) => page.evaluate((id) => {
  const value = (window as any).PM.L(id).d.gain;
  return typeof value === 'number' ? value : value.v;
}, id);

test('the agent opens the mixer, which shows a strip per audible layer and an undoable fader', async ({ session }) => {
  const { page } = session;
  const ids = await openMixerProject(session);

  const opened = await agentTool(page, 'open_panel', { panelId: 'mixer' });
  expect(opened.ok).toBe(true);
  const panel = page.locator('#panel-mixer');
  await expect(panel).toBeVisible();
  await expect(panel.locator('[data-mixer-strip]')).toHaveCount(3);
  await expect(panel.locator(`[data-mixer-strip="${ids.tone}"]`)).toBeVisible();
  await expect(panel.locator(`[data-mixer-strip="${ids.video}"]`)).toBeVisible();
  expect(opened.text).toContain('Tone level');

  // Drag the knob up: one live gesture, one Undo step, unity again after Undo.
  const fader = panel.getByRole('slider', { name: 'Tone level' });
  const knob = fader.locator('.mx-knob');
  const box = (await knob.boundingBox())!;
  const history = await page.evaluate(() => (window as any).PM.hist.list().length);
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  for (let step = 1; step <= 6; step++) await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 - step * 5);
  await page.mouse.up();
  const raised = await gain(page, ids.tone);
  expect(raised).toBeGreaterThan(1.2);
  await expect(panel.locator(`[data-mixer-strip="${ids.tone}"] .mx-readout`)).toHaveValue(/^\+\d/);
  expect(await page.evaluate(() => (window as any).PM.hist.list().length)).toBe(history + 1);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await gain(page, ids.tone)).toBe(1);

  // A held arrow key is one coalesced step; double-click returns to unity.
  await fader.focus();
  await page.keyboard.down('ArrowDown');
  await page.keyboard.down('ArrowDown');
  await page.keyboard.down('ArrowDown');
  await page.keyboard.up('ArrowDown');
  expect(await gain(page, ids.tone)).toBeCloseTo(10 ** (-1.5 / 20), 4);
  expect(await page.evaluate(() => (window as any).PM.hist.list().length)).toBe(history + 1);
  await knob.dblclick();
  expect(await gain(page, ids.tone)).toBe(1);

  // Typing a level into the readout.
  const readout = panel.locator(`[data-mixer-strip="${ids.tone}"] .mx-readout`);
  await readout.click();
  await readout.fill('-6');
  await readout.press('Enter');
  expect(await gain(page, ids.tone)).toBeCloseTo(10 ** (-6 / 20), 4);

  // A double-click on the rail (away from the knob) is one step to unity,
  // not a jump followed by a reset.
  const typed = await gain(page, ids.tone);
  const steps = await page.evaluate(() => (window as any).PM.hist.list().length);
  const rail = (await fader.boundingBox())!;
  await fader.dblclick({ position: { x: rail.width / 2, y: rail.height - 3 } });
  expect(await gain(page, ids.tone)).toBe(1);
  expect(await page.evaluate(() => (window as any).PM.hist.list().length)).toBe(steps + 1);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await gain(page, ids.tone)).toBeCloseTo(typed, 6);

  // A single click on the rail jumps there and settles as its own step once
  // the double-click window has passed.
  const busy = () => page.evaluate(() => (window as any).PM.hist.busy());
  await fader.click({ position: { x: rail.width / 2, y: rail.height - 3 } });
  expect(await gain(page, ids.tone)).toBeLessThan(typed);
  await page.waitForFunction(() => !(window as any).PM.hist.busy());
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await gain(page, ids.tone)).toBeCloseTo(typed, 6);
  // Cmd+Z straight after a rail click settles the jump first, then undoes it.
  await fader.click({ position: { x: rail.width / 2, y: rail.height - 3 } });
  expect(await busy()).toBe(true);
  await page.keyboard.press('Meta+z');
  expect(await busy()).toBe(false);
  expect(await gain(page, ids.tone)).toBeCloseTo(typed, 6);

  // The master fader is the composition's output level and reaches the engine
  // while it plays; Undo restores both.
  const masterLevels = () => page.evaluate(() => {
    const PM = (window as any).PM;
    return { project: PM.proj.audioGain ?? 1, engine: PM.Audio.inspect().master };
  });
  await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); PM.play(); });
  await page.waitForFunction(() => (window as any).PM.Audio.inspect().voices.length > 0);
  await panel.getByRole('slider', { name: 'Master level' }).focus();
  await page.keyboard.press('PageDown');
  await page.waitForFunction(() => {
    const PM = (window as any).PM;
    return Math.abs(PM.Audio.inspect().master - PM.proj.audioGain) < 1e-6;
  });
  const lowered = await masterLevels();
  expect(lowered.project).toBeCloseTo(10 ** (-6 / 20), 4);
  expect(lowered.engine).toBeCloseTo(lowered.project, 6);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await page.waitForFunction(() => (window as any).PM.Audio.inspect().master === 1);
  expect(await masterLevels()).toEqual({ project: 1, engine: 1 });

  // Each composition keeps its own output level: opening a precomp plays it at
  // unity, and the parent's level is still there when you come back.
  await panel.getByRole('slider', { name: 'Master level' }).focus();
  await page.keyboard.press('PageDown');
  const parent = await page.evaluate(() => (window as any).PM.proj.audioGain);
  const nested = await page.evaluate((id) => {
    const PM = (window as any).PM;
    const root = PM.proj.compId;
    const comp = PM.Comps.precompose([id], { name: 'Tone Comp' });
    const precomp = PM.proj.layers.find((layer: any) => layer.type === 'precomp');
    PM.Comps.open(precomp.d.comp);
    PM.setTime(0); PM.play();
    return { root, comp: precomp.d.comp, made: !!comp, level: PM.proj.audioGain ?? null };
  }, ids.tone);
  expect(nested).toMatchObject({ made: true, level: null });
  await page.waitForFunction(() => (window as any).PM.Audio.inspect().voices.length > 0);
  await page.waitForFunction(() => (window as any).PM.Audio.inspect().master === 1);
  await expect(panel.getByRole('slider', { name: 'Master level' })).toHaveAttribute('aria-valuenow', /^0(\.0+)?$/);
  const back = await page.evaluate(({ root, comp }) => {
    const PM = (window as any).PM;
    PM.Comps.open(root);
    return { level: PM.proj.audioGain, child: Object.hasOwn(PM.proj.comps[comp], 'audioGain') };
  }, nested);
  expect(back).toEqual({ level: parent, child: false });
  await page.evaluate(() => (window as any).PM.pause());
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('mute and solo change what plays, and meters run only while sound plays', async ({ session }) => {
  const { page } = session;
  const ids = await openMixerProject(session);
  await agentTool(page, 'open_panel', { panelId: 'mixer' });
  const panel = page.locator('#panel-mixer');
  await expect(panel.locator('[data-mixer-strip]')).toHaveCount(3);

  await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); PM.play(); });
  await page.waitForFunction((id) => (window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);
  // Meters tap the strip while it is visible and playing.
  await page.waitForFunction((id) => (window as any).PM.Audio.inspect().taps.includes(id), ids.tone);

  // Mute writes the layer's own switch; its voice stops. Undo brings it back.
  const mute = panel.getByRole('button', { name: 'Mute Tone' });
  await mute.click();
  await page.waitForFunction((id) => !(window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);
  expect(await page.evaluate((id) => (window as any).PM.L(id).on, ids.tone)).toBe(false);
  await expect(mute).toHaveAttribute('aria-pressed', 'true');
  await page.evaluate(() => (window as any).PM.hist.undo());
  await page.waitForFunction((id) => (window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);
  expect(await page.evaluate((id) => (window as any).PM.L(id).on, ids.tone)).toBe(true);
  await expect(mute).toHaveAttribute('aria-pressed', 'false');
  // And the button toggles it back the same way.
  await mute.click();
  await page.waitForFunction((id) => !(window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);
  await mute.click();
  await page.waitForFunction((id) => (window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);

  // Solo is a listening aid: the other strips go quiet, the project does not change.
  const revision = await page.evaluate(() => (window as any).PM.proj.revision);
  await panel.getByRole('button', { name: 'Solo Interview' }).click();
  // The solo gain glides (a few ms), so wait for it to settle rather than racing it.
  await page.waitForFunction((id) => ((window as any).PM.Audio.inspect().strips.find((s: any) => s.id === id)?.solo ?? 0) < 0.5, ids.tone);
  const soloed = await page.evaluate(() => (window as any).PM.Audio.inspect());
  expect(soloed.monitorSolo).toEqual([ids.video]);
  await expect(panel.locator(`[data-mixer-strip="${ids.tone}"]`)).toHaveClass(/quiet/);
  expect(await page.evaluate(() => (window as any).PM.proj.revision)).toBe(revision);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.some((l: any) => l.solo))).toBe(false);
  // A minimised or covered window hides the page, not the panel: solo stays.
  await page.evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  expect(await page.evaluate(() => (window as any).PM.Audio.inspect().monitorSolo)).toEqual([ids.video]);
  await page.evaluate(() => {
    delete (document as any).visibilityState;
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await panel.getByRole('button', { name: 'Solo Interview' }).click();
  await page.waitForFunction(() => (window as any).PM.Audio.inspect().monitorSolo.length === 0);

  // The keyframe toggle names the property it animates in History.
  await page.evaluate(() => (window as any).PM.pause());
  await panel.getByRole('button', { name: 'Add Interview level keyframe' }).click();
  expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Add keyframe for Audio gain');
  await panel.getByRole('button', { name: 'Remove Interview level keyframe' }).click();
  expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Remove keyframe for Audio gain');

  // After a stop the meters fall, then the taps are released.
  await page.waitForFunction(() => (window as any).PM.Audio.inspect().taps.length === 0, null, { timeout: 15_000 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('the mixer can be placed in a new workspace and is offered as a panel', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  const offered = await page.evaluate(() => {
    const PM = (window as any).PM;
    const mixer = PM.PANELS.mixer;
    return { title: mixer?.title, library: mixer?.library !== false };
  });
  expect(offered).toEqual({ title: 'Mixer', library: true });
  const result = await agentTool(page, 'set_panel_layout', {
    name: 'Sound',
    docks: [
      { id: 'left', panels: [{ id: 'assets' }, { id: 'mixer', size: 320 }] },
      { id: 'center', panels: [{ id: 'viewer', flex: true }, { id: 'timeline', size: 300 }] },
      { id: 'right', panels: [{ id: 'inspector', flex: true }] }
    ]
  });
  expect(result.ok).toBe(true);
  await expect(page.locator('#panel-mixer')).toBeVisible();
  await expect(page.locator('#panel-mixer')).toContainText('No audio in this composition');
  const placed = await page.evaluate(() => (window as any).PM.WS.current.layout.docks.find((d: any) => d.id === 'left').panels.map((p: any) => p.id));
  expect(placed).toContain('mixer');
});
