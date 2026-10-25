import type { Page } from 'playwright';

import { expect, test, type LaunchedApp } from './helpers/app';
import { fixturePath } from './helpers/media';
import { VIEWER_VARIANTS, useViewer } from './helpers/viewer-fork';

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

type NativeReading = { sequence: number; strip: number; master: number; other: number };

/** A fresh native levels message, with legacy playback still entirely idle. */
async function nativeReading(page: Page, strip: string, after: number, kind: 'sound' | 'silence', other?: string): Promise<NativeReading> {
  const handle = await page.waitForFunction(({ strip, after, kind, other }) => {
    const PM = (window as any).PM;
    const audio = PM.EngineConnection?.audio;
    const latest = audio?.latest();
    if (!latest || audio.levelsReceived <= after || PM.EngineConnection.status().driving !== true) return false;
    const levels = latest.levels;
    const peak = (id: string) => Math.max(0, ...(levels.strips.find((s: any) => s.id === id)?.levels.peak ?? []));
    const stripPeak = peak(strip);
    if (kind === 'sound' ? stripPeak <= 0 : stripPeak !== 0) return false;
    if (kind === 'silence' && other && peak(other) <= 0) return false;
    if (PM.Audio.inspect().voices.length !== 0 || Number(document.documentElement.dataset.audioVoices || 0) !== 0) return false;
    return {
      sequence: audio.levelsReceived,
      strip: stripPeak,
      master: Math.max(0, ...levels.master.peak),
      other: other ? peak(other) : 0
    };
  }, { strip, after, kind, other });
  return handle.jsonValue() as Promise<NativeReading>;
}

async function nativeTransport(page: Page): Promise<boolean> {
  const native = await page.evaluate(() => Boolean((window as any).PM.EngineConnection));
  if (native) await page.waitForFunction(() => {
    const status = (window as any).PM.EngineConnection.status();
    return status.route === 'engine' && status.driving === true;
  });
  if (native) await page.evaluate(() => {
    const PM = (window as any).PM;
    const evidence = { violations: 0 };
    const check = () => {
      if (PM.Audio.inspect().voices.length !== 0 || Number(document.documentElement.dataset.audioVoices || 0) !== 0) evidence.violations++;
    };
    check();
    const observer = new MutationObserver(check);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-audio-voices'] });
    PM.EngineConnection.audio.onLevels(check);
    (window as any).__mixerNativeEvidence = evidence;
  });
  return native;
}

/** With one strip monitored, Rust's post-master peak / pre-master peak is the fader coefficient. */
async function nativeRatio(page: Page, strip: string, after: number, expected: number): Promise<NativeReading> {
  const handle = await page.waitForFunction(({ strip, after, expected }) => {
    const PM = (window as any).PM;
    const audio = PM.EngineConnection?.audio;
    const latest = audio?.latest();
    if (!latest || audio.levelsReceived <= after || PM.EngineConnection.status().driving !== true) return false;
    const levels = latest.levels;
    const tone = levels.strips.find((s: any) => s.id === strip);
    const stripPeak = Math.max(0, ...(tone?.levels.peak ?? []));
    const masterPeak = Math.max(0, ...levels.master.peak);
    if (stripPeak <= 0 || Math.abs(masterPeak / stripPeak - expected) >= 1e-6) return false;
    if (PM.Audio.inspect().voices.length !== 0 || Number(document.documentElement.dataset.audioVoices || 0) !== 0) return false;
    return { sequence: audio.levelsReceived, strip: stripPeak, master: masterPeak, other: 0 };
  }, { strip, after, expected });
  return handle.jsonValue() as Promise<NativeReading>;
}

async function nativeStripRatio(page: Page, strip: string, after: number, baseline: number, expected: number): Promise<NativeReading> {
  const handle = await page.waitForFunction(({ strip, after, baseline, expected }) => {
    const PM = (window as any).PM;
    const audio = PM.EngineConnection?.audio;
    const latest = audio?.latest();
    if (!latest || audio.levelsReceived <= after || PM.EngineConnection.status().driving !== true) return false;
    const levels = latest.levels;
    const stripPeak = Math.max(0, ...(levels.strips.find((s: any) => s.id === strip)?.levels.peak ?? []));
    if (stripPeak <= 0 || Math.abs(stripPeak / baseline - expected) >= 0.002) return false;
    if (PM.Audio.inspect().voices.length !== 0 || Number(document.documentElement.dataset.audioVoices || 0) !== 0) return false;
    return { sequence: audio.levelsReceived, strip: stripPeak, master: Math.max(0, ...levels.master.peak), other: 0 };
  }, { strip, after, baseline, expected });
  return handle.jsonValue() as Promise<NativeReading>;
}

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
  // Isolate Tone so the native master/strip meter ratio measures the master
  // coefficient even while the imported video has its own audio.
  await page.evaluate((id) => (window as any).PM.Audio.setMonitorSolo([id]), ids.tone);
  const native = await nativeTransport(page);
  await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); PM.play(); });
  let nativeBefore = 0;
  if (native) {
    // The existing Tone fader must retune the running Rust mix, then Undo it.
    const baseline = await nativeReading(page, ids.tone, 0, 'sound');
    nativeBefore = baseline.sequence;
    const beforeGain = await gain(page, ids.tone);
    await fader.focus();
    await page.keyboard.press('PageDown');
    const afterGain = await gain(page, ids.tone);
    expect(afterGain).toBeLessThan(beforeGain);
    const retuned = await nativeStripRatio(page, ids.tone, nativeBefore, baseline.strip, afterGain / beforeGain);
    nativeBefore = retuned.sequence;
    await page.evaluate(() => (window as any).PM.hist.undo());
    expect(await gain(page, ids.tone)).toBeCloseTo(beforeGain, 6);
    const restored = await nativeStripRatio(page, ids.tone, nativeBefore, baseline.strip, 1);
    nativeBefore = restored.sequence;
  } else {
    await page.waitForFunction(() => (window as any).PM.Audio.inspect().voices.length > 0);
  }
  await panel.getByRole('slider', { name: 'Master level' }).focus();
  await page.keyboard.press('PageDown');
  if (native) {
    const project = await page.evaluate(() => (window as any).PM.proj.audioGain);
    expect(project).toBeCloseTo(10 ** (-6 / 20), 4);
    await page.evaluate(() => (window as any).PM.setTime(0));
    const lowered = await nativeRatio(page, ids.tone, nativeBefore, project);
    expect(lowered.master / lowered.strip).toBeCloseTo(project, 6);
    nativeBefore = lowered.sequence;
  } else {
    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      return Math.abs(PM.Audio.inspect().master - PM.proj.audioGain) < 1e-6;
    });
    const lowered = await masterLevels();
    expect(lowered.project).toBeCloseTo(10 ** (-6 / 20), 4);
    expect(lowered.engine).toBeCloseTo(lowered.project, 6);
  }
  await page.evaluate(() => (window as any).PM.hist.undo());
  if (native) {
    await page.evaluate(() => (window as any).PM.setTime(0));
    const restored = await nativeRatio(page, ids.tone, nativeBefore, 1);
    expect(restored.master / restored.strip).toBeCloseTo(1, 6);
    nativeBefore = restored.sequence;
    expect(await page.evaluate(() => (window as any).PM.proj.audioGain ?? 1)).toBe(1);
  } else {
    await page.waitForFunction(() => (window as any).PM.Audio.inspect().master === 1);
    expect(await masterLevels()).toEqual({ project: 1, engine: 1 });
  }
  await page.evaluate(() => (window as any).PM.Audio.setMonitorSolo([]));

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
  if (native) {
    const nestedLevels = await nativeRatio(page, ids.tone, nativeBefore, 1);
    expect(nestedLevels.master / nestedLevels.strip).toBeCloseTo(1, 6);
  } else {
    await page.waitForFunction(() => (window as any).PM.Audio.inspect().voices.length > 0);
    await page.waitForFunction(() => (window as any).PM.Audio.inspect().master === 1);
  }
  await expect(panel.getByRole('slider', { name: 'Master level' })).toHaveAttribute('aria-valuenow', /^0(\.0+)?$/);
  const back = await page.evaluate(({ root, comp }) => {
    const PM = (window as any).PM;
    PM.Comps.open(root);
    return { level: PM.proj.audioGain, child: Object.hasOwn(PM.proj.comps[comp], 'audioGain') };
  }, nested);
  expect(back).toEqual({ level: parent, child: false });
  await page.evaluate(() => (window as any).PM.pause());
  if (native) expect(await page.evaluate(() => (window as any).__mixerNativeEvidence.violations)).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('mute and solo change what plays, and meters run only while sound plays', async ({ session }) => {
  const { page } = session;
  const ids = await openMixerProject(session);
  await agentTool(page, 'open_panel', { panelId: 'mixer' });
  const panel = page.locator('#panel-mixer');
  await expect(panel.locator('[data-mixer-strip]')).toHaveCount(3);

  const native = await nativeTransport(page);
  await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); PM.play(); });
  let nativeBefore = 0;
  if (native) {
    const tone = await nativeReading(page, ids.tone, 0, 'sound');
    expect(tone.strip).toBeGreaterThan(0);
    nativeBefore = tone.sequence;
  } else {
    await page.waitForFunction((id) => (window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);
    // Meters tap the strip while it is visible and playing.
    await page.waitForFunction((id) => (window as any).PM.Audio.inspect().taps.includes(id), ids.tone);
  }

  // Mute writes the layer's own switch; its voice stops. Undo brings it back.
  const mute = panel.getByRole('button', { name: 'Mute Tone' });
  await mute.click();
  if (native) {
    await page.evaluate(() => (window as any).PM.setTime(0));
    nativeBefore = (await nativeReading(page, ids.tone, nativeBefore, 'silence', ids.video)).sequence;
  }
  else await page.waitForFunction((id) => !(window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);
  expect(await page.evaluate((id) => (window as any).PM.L(id).on, ids.tone)).toBe(false);
  await expect(mute).toHaveAttribute('aria-pressed', 'true');
  await page.evaluate(() => (window as any).PM.hist.undo());
  if (native) {
    await page.evaluate(() => (window as any).PM.setTime(0));
    nativeBefore = (await nativeReading(page, ids.tone, nativeBefore, 'sound')).sequence;
  } else await page.waitForFunction((id) => (window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);
  expect(await page.evaluate((id) => (window as any).PM.L(id).on, ids.tone)).toBe(true);
  await expect(mute).toHaveAttribute('aria-pressed', 'false');
  // And the button toggles it back the same way.
  await mute.click();
  if (native) {
    await page.evaluate(() => (window as any).PM.setTime(0));
    nativeBefore = (await nativeReading(page, ids.tone, nativeBefore, 'silence', ids.video)).sequence;
  }
  else await page.waitForFunction((id) => !(window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);
  await mute.click();
  if (native) {
    await page.evaluate(() => (window as any).PM.setTime(0));
    nativeBefore = (await nativeReading(page, ids.tone, nativeBefore, 'sound')).sequence;
  } else await page.waitForFunction((id) => (window as any).PM.Audio.inspect().voices.some((v: any) => v.strip === id), ids.tone);

  // Solo is a listening aid: the other strips go quiet, the project does not change.
  const revision = await page.evaluate(() => (window as any).PM.proj.revision);
  await panel.getByRole('button', { name: 'Solo Interview' }).click();
  if (native) {
    await page.evaluate(() => (window as any).PM.setTime(0));
    // Strip taps are before monitor solo. With Interview alone monitored,
    // the post-solo master must equal its strip, while Tone still meters.
    const video = await nativeRatio(page, ids.video, nativeBefore, 1);
    expect(video.master / video.strip).toBeCloseTo(1, 6);
    nativeBefore = video.sequence;
    const tone = await nativeReading(page, ids.tone, nativeBefore, 'sound');
    expect(tone.strip).toBeGreaterThan(0);
    nativeBefore = tone.sequence;
  } else {
    // The solo gain glides (a few ms), so wait for it to settle rather than racing it.
    await page.waitForFunction((id) => ((window as any).PM.Audio.inspect().strips.find((s: any) => s.id === id)?.solo ?? 0) < 0.5, ids.tone);
  }
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
  if (native) {
    await page.evaluate(() => (window as any).PM.setTime(0));
    nativeBefore = (await nativeReading(page, ids.tone, nativeBefore, 'sound')).sequence;
  }

  // The keyframe toggle names the property it animates in History.
  await page.evaluate(() => (window as any).PM.pause());
  await panel.getByRole('button', { name: 'Add Interview level keyframe' }).click();
  expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Add keyframe for Audio gain');
  await panel.getByRole('button', { name: 'Remove Interview level keyframe' }).click();
  expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Remove keyframe for Audio gain');

  // After a stop the meters fall, then the taps are released.
  if (native) {
    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      const meters = PM.Audio.meters;
      const out = { peak: [1, 1], rms: [1, 1] };
      meters.read(meters.MASTER, out);
      const tone = { peak: [1, 1], rms: [1, 1] };
      meters.read(PM.proj.layers.find((l: any) => l.name === 'Tone')?.id, tone);
      return !meters.running() && out.peak.every((peak: number) => peak === 0) && tone.peak.every((peak: number) => peak === 0)
        && PM.Audio.inspect().voices.length === 0 && Number(document.documentElement.dataset.audioVoices || 0) === 0;
    }, null, { timeout: 15_000 });
  } else await page.waitForFunction(() => (window as any).PM.Audio.inspect().taps.length === 0, null, { timeout: 15_000 });
  if (native) expect(await page.evaluate(() => (window as any).__mixerNativeEvidence.violations)).toBe(0);
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

/*
 * The retune matrix (appendix C row C6), in both themes with the current
 * viewer and forks of the viewers users can have: every asset kind that
 * carries sound plays through its strip, and every audio control today's
 * mix has (legacy/core/audio.ts: gain, video audio gain and mute, layer
 * switch, group switch, precomp output level, relink) changes what plays,
 * with the engine on (its levels, and no legacy voices) and off (today's
 * voices). Today's audio model has no pan or pitch control.
 */
const KINDS = ['tone.wav', 'tone.mp3', 'tone.m4a', 'tone.flac', 'tone.opus', 'h264-aac.mp4', 'h264-aac.mov', 'vp9-opus.webm'];

/** Waits until `strip` sounds (or is silent) in a fresh reading of what plays now. */
async function plays(page: Page, strip: string, kind: 'sound' | 'silence', native: boolean): Promise<number> {
  const before = native ? await page.evaluate(() => (window as any).PM.EngineConnection.audio.levelsReceived) : 0;
  await page.evaluate(() => (window as any).PM.setTime(0));
  const handle = await page.waitForFunction(({ strip, kind, native, before }) => {
    const PM = (window as any).PM;
    if (native) {
      const audio = PM.EngineConnection.audio;
      const latest = audio.latest();
      if (!latest || audio.levelsReceived <= before + 1 || PM.EngineConnection.status().driving !== true) return false;
      if (PM.Audio.inspect().voices.length !== 0 || Number(document.documentElement.dataset.audioVoices || 0) !== 0) return false;
      const peak = Math.max(0, ...(latest.levels.strips.find((s: any) => s.id === strip)?.levels.peak ?? []));
      return (kind === 'sound' ? peak > 0 : peak === 0) ? Math.max(peak, 1e-9) : false;
    }
    const voice = PM.Audio.inspect().voices.some((v: any) => v.strip === strip);
    return (kind === 'sound') === voice ? 1 : false;
  }, { strip, kind, native, before }, { timeout: 20_000 });
  return handle.jsonValue() as Promise<number>;
}

/** With the engine on, waits for `strip`'s peak to be `ratio` × `baseline` in fresh readings. */
async function playsAt(page: Page, strip: string, baseline: number, ratio: number): Promise<void> {
  const before = await page.evaluate(() => (window as any).PM.EngineConnection.audio.levelsReceived);
  await page.evaluate(() => (window as any).PM.setTime(0));
  await page.waitForFunction(({ strip, baseline, ratio, before }) => {
    const PM = (window as any).PM;
    const audio = PM.EngineConnection.audio;
    const latest = audio.latest();
    if (!latest || audio.levelsReceived <= before + 1) return false;
    if (PM.Audio.inspect().voices.length !== 0) return false;
    const peak = Math.max(0, ...(latest.levels.strips.find((s: any) => s.id === strip)?.levels.peak ?? []));
    return peak > 0 && Math.abs(peak / baseline - ratio) < 0.01;
  }, { strip, baseline, ratio, before }, { timeout: 20_000 });
}

/** An undoable change to the open composition, as the panels make it. */
function change(page: Page, label: string, edit: string, id: string) {
  return page.evaluate(({ label, edit, id }) => {
    const PM = (window as any).PM;
    PM.hist.do(label, () => new Function('PM', 'id', edit)(PM, id));
    PM.bus.emit('layers');
  }, { label, edit, id });
}

for (const theme of ['light', 'dark'] as const) for (const variant of VIEWER_VARIANTS) {
  test(`retune matrix: every asset kind and audio control in ${theme}, ${variant.name}`, async ({ session }) => {
    test.setTimeout(150_000);
    await session.openEditor();
    const { page, app } = session;
    await useViewer(page, session.userData, variant);
    await page.evaluate((theme) => (window as any).PM.theme.apply(theme), theme);
    await app.evaluate(({ BrowserWindow }) => {
      for (const window of BrowserWindow.getAllWindows()) window.webContents.setAudioMuted(true);
    });
    const chooser = page.waitForEvent('filechooser');
    await page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(KINDS.map(name => fixturePath(name)));
    await page.waitForFunction((count) => {
      const layers = (window as any).PM.proj.layers;
      return layers.filter((l: any) => l.type === 'audio' || (l.type === 'video' && l.d.embeddedAudio === true)).length === count;
    }, KINDS.length, { timeout: 60_000 });
    const ids = await page.evaluate((kinds) => {
      const PM = (window as any).PM;
      const out: Record<string, string> = {};
      for (const layer of PM.proj.layers) {
        const asset = PM.proj.assets[layer.d?.asset];
        const names = [asset?.sourcePath, asset?.name, asset?.fileName].filter(Boolean).map((name: string) => String(name).split('/').pop());
        const kind = kinds.find((k: string) => names.includes(k));
        if (kind) out[kind] = layer.id;
      }
      return out;
    }, KINDS);
    expect(Object.keys(ids).sort()).toEqual([...KINDS].sort());
    expect((await agentTool(page, 'open_panel', { panelId: 'mixer' })).ok).toBe(true);
    const panel = page.locator('#panel-mixer');
    await expect(panel.locator('[data-mixer-strip]')).toHaveCount(KINDS.length + 1);
    const native = await nativeTransport(page);
    await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); PM.play(); });

    // Every kind sounds through its own strip.
    const baseline: Record<string, number> = {};
    for (const kind of KINDS) {
      await page.evaluate((id) => (window as any).PM.Audio.setMonitorSolo([id]), ids[kind]!);
      baseline[kind] = await plays(page, ids[kind]!, 'sound', native);
    }
    await page.evaluate(() => (window as any).PM.Audio.setMonitorSolo([]));

    // Gain on an audio layer and on a video's sound retune what plays; Undo restores it.
    const wav = ids['tone.wav']!, mp4 = ids['h264-aac.mp4']!;
    await change(page, 'Audio gain', 'PM.L(id).d.gain = 0.5', wav);
    if (native) await playsAt(page, wav, baseline['tone.wav']!, 0.5); else await plays(page, wav, 'sound', native);
    await page.evaluate(() => (window as any).PM.hist.undo());
    if (native) await playsAt(page, wav, baseline['tone.wav']!, 1);
    await change(page, 'Audio gain', 'PM.L(id).d.audioGain = 0.5', mp4);
    if (native) await playsAt(page, mp4, baseline['h264-aac.mp4']!, 0.5); else await plays(page, mp4, 'sound', native);
    await page.evaluate(() => (window as any).PM.hist.undo());
    if (native) await playsAt(page, mp4, baseline['h264-aac.mp4']!, 1);

    // A video's audio mute, the layer switch and a group switch silence the strip.
    await change(page, 'Mute audio', 'PM.L(id).d.audioMuted = true', mp4);
    await plays(page, mp4, 'silence', native);
    await page.evaluate(() => (window as any).PM.hist.undo());
    await plays(page, mp4, 'sound', native);
    const mov = ids['h264-aac.mov']!;
    await change(page, 'Hide layer', 'PM.L(id).on = false', mov);
    await plays(page, mov, 'silence', native);
    await page.evaluate(() => (window as any).PM.hist.undo());
    await plays(page, mov, 'sound', native);
    const flac = ids['tone.flac']!;
    const group = await page.evaluate((id) => (window as any).PM.groupLayers([id], 'Sound group').id, flac);
    await plays(page, flac, 'sound', native);
    await change(page, 'Hide group', 'PM.L(id).on = false', group);
    await plays(page, flac, 'silence', native);
    await page.evaluate(() => (window as any).PM.hist.undo());
    await plays(page, flac, 'sound', native);

    // A precomp's output level applies to the sound inside it, on the precomp's strip.
    const mp3 = ids['tone.mp3']!;
    const precomp = await page.evaluate((id) => {
      const PM = (window as any).PM;
      PM.Comps.precompose([id], { name: 'Sound comp', adjustDuration: false });
      return PM.proj.layers.find((layer: any) => layer.type === 'precomp' && layer.name === 'Sound comp').id;
    }, mp3);
    await page.evaluate((id) => (window as any).PM.Audio.setMonitorSolo([id]), precomp);
    const nested = await plays(page, precomp, 'sound', native);
    await change(page, 'Precomp level', 'PM.L(id).d.audioGain = 0.5', precomp);
    if (native) await playsAt(page, precomp, nested, 0.5); else await plays(page, precomp, 'sound', native);
    await change(page, 'Mute precomp', 'PM.L(id).d.audioMuted = true', precomp);
    await plays(page, precomp, 'silence', native);
    await page.evaluate(() => { const PM = (window as any).PM; PM.hist.undo(); PM.hist.undo(); PM.Audio.setMonitorSolo([]); });
    await plays(page, precomp, 'sound', native);

    // Relinking a source plays the new file through the same strip. (Relink
    // while paused, as the Replace dialog does: with the engine off, an agent
    // relink during playback leaves today's audio stopped while the transport
    // runs, until playback restarts.)
    await page.evaluate(() => (window as any).PM.pause());
    const asset = await page.evaluate((id) => (window as any).PM.L(id).d.asset, wav);
    const relinked = await page.evaluate(async ({ asset, path }) => {
      const PM = (window as any).PM;
      const reply = await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'mixer-matrix', callId: 'relink', tool: 'replace_media', arguments: { assetId: asset, path }, baseRevision: PM.proj.revision || 0 });
      await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'mixer-matrix', callId: 'finish', tool: '__finish_run', arguments: { commit: true }, baseRevision: PM.proj.revision || 0 });
      return { ok: reply.ok, source: PM.proj.assets[asset]?.sourcePath ?? PM.proj.assets[asset]?.name ?? '' };
    }, { asset, path: fixturePath('tone.opus') });
    expect(relinked.ok).toBe(true);
    expect(relinked.source).toMatch(/tone\.opus$/);
    await page.evaluate((id) => { const PM = (window as any).PM; PM.Audio.setMonitorSolo([id]); PM.setTime(0); PM.play(); }, wav);
    await plays(page, wav, 'sound', native);
    await page.evaluate(() => (window as any).PM.pause());
    if (native) expect(await page.evaluate(() => (window as any).__mixerNativeEvidence.violations)).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
