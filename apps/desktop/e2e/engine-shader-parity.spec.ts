/** First-use parity uses the real native engine and in-window ANGLE runner.
 * Pins and native verifications must survive an app restart. No injected
 * native pixels or forced runner verdicts are used here.
 */
import { readdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, type LaunchedApp } from './helpers/app';

const STABLE = 'o = vec4(0.25, 0.5, 0.75, 1.0);';
// Catastrophic cancellation exposes different interpolation rounding in
// ANGLE and the native pipeline on Metal. It is legal GLSL, with no seed
// listing or injected verdict: the actual first draw must discover the pin.
const UNSTABLE = 'float a = v_st.x * u_res.x - gl_FragCoord.x; o = vec4(vec3(fract(a * 1048576.0)), 1.0);';

async function decisions(session: LaunchedApp): Promise<Record<string, string>> {
  const dir = path.join(session.userData, 'Engine Cache/Shader Cache/angle-parity');
  const out: Record<string, string> = {};
  for (const name of await readdir(dir).catch(() => [] as string[])) out[name] = await readFile(path.join(dir, name), 'utf8');
  return out;
}

async function draw(session: LaunchedApp, code: string, samples = 5): Promise<{ firstMs: number; steadyMs: number[]; firstRounds: number; steadyRounds: number; legacyMaxCv: number; pixels: number[] }> {
  return session.page.evaluate(async ({ code, samples }) => {
    const PM = (window as any).PM;
    const reg = PM.Kernel.registerEffect('parity-e2e', { id: 'parity-e2e', label: 'Parity', group: 'Test', params: [], frag: code });
    try {
      const project = PM.mkProject({ name: 'Parity', w: 239, h: 137, fps: 30, dur: 2 });
      const layer = PM.mkLayer('solid', { from: 0, dur: 2, d: { color: '#FFFFFF', w: 239, h: 137 } }, project);
      layer.p['position.x'].v = 0; layer.p['position.y'].v = 0;
      layer.fx = [{ id: 'parity-instance', type: 'parity-e2e', on: true, p: {} }];
      project.layers = [layer]; PM.replaceProject(project);
      await PM.EngineDefinitions.settle();
      const api = PM.Kernel.api('parity-e2e');
      const before = PM.EngineRender.stats.rounds;
      const start = performance.now();
      const first = api.render.renderFrameTo(0.25, 239, 137, { mblur: false });
      const firstMs = performance.now() - start;
      const firstRounds = PM.EngineRender.stats.rounds - before;
      const pixels = [...first.getContext('2d').getImageData(0, 0, 239, 137).data];
      // Today's actual compositor is the reference, on the same project.
      const legacy = PM.GL.renderToPixels(0.25, 239, 137, { opaque: true, topDownOpaque: true, mblur: false });
      let legacyMaxCv = 0;
      for (let i = 0; i < pixels.length; i++) legacyMaxCv = Math.max(legacyMaxCv, Math.abs(pixels[i]! - legacy[i]));
      const steadyMs: number[] = [];
      const steadyStart = PM.EngineRender.stats.rounds;
      for (let i = 0; i < samples; i++) {
        const start = performance.now();
        const canvas = api.render.renderFrameTo(0.5 + i / 30, 239, 137, { mblur: false });
        canvas.getContext('2d').getImageData(0, 0, 1, 1);
        steadyMs.push(performance.now() - start);
      }
      return { firstMs, steadyMs, firstRounds, steadyRounds: PM.EngineRender.stats.rounds - steadyStart, legacyMaxCv, pixels };
    } finally { reg.dispose(); }
  }, { code, samples });
}

test('first-use parity verifies stable programs, pins ill-conditioned programs and survives restart', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  await session.page.waitForFunction(() => !!(window as any).PM.EngineRender);
  const stable = await draw(session, STABLE);
  console.log('stable shader parity', { firstMs: stable.firstMs, firstRounds: stable.firstRounds, steadyMs: stable.steadyMs, steadyRounds: stable.steadyRounds });
  await expect.poll(async () => Object.values(await decisions(session)).filter(v => v === 'native-verified\n').length).toBeGreaterThan(0);
  expect(stable.steadyRounds).toBe(0);
  expect(stable.legacyMaxCv).toBeLessThanOrEqual(1);
  const unstable = await draw(session, UNSTABLE);
  console.log('ill-conditioned shader parity', { firstMs: unstable.firstMs, firstRounds: unstable.firstRounds, steadyMs: unstable.steadyMs, steadyRounds: unstable.steadyRounds });
  if (process.env['PM_SHADER_PARITY_MEASURE']) await writeFile(process.env['PM_SHADER_PARITY_MEASURE'], JSON.stringify({ stable: { ...stable, pixels: undefined }, unstable: { ...unstable, pixels: undefined } }, null, 2));
  await expect.poll(async () => Object.values(await decisions(session)).filter(v => v === 'runner\n').length).toBeGreaterThan(0);
  expect(unstable.legacyMaxCv).toBeLessThanOrEqual(1);
  const before = await decisions(session);
  await session.relaunch(); await session.openEditor();
  await session.page.waitForFunction(() => !!(window as any).PM.EngineRender);
  const stableRestart = await draw(session, STABLE);
  const unstableRestart = await draw(session, UNSTABLE);
  expect(stableRestart.firstRounds).toBe(0);
  expect(stableRestart.pixels).toEqual(stable.pixels);
  expect(unstableRestart.pixels).toEqual(unstable.pixels);
  expect(await decisions(session)).toEqual(before);
  const output = process.env['PM_SHADER_PARITY_MEASURE'];
  if (output) await writeFile(output, JSON.stringify({ stable: { ...stable, pixels: undefined }, unstable: { ...unstable, pixels: undefined }, stableRestart: { ...stableRestart, pixels: undefined }, unstableRestart: { ...unstableRestart, pixels: undefined } }, null, 2));
});

test('an asynchronous snapshot completes and verifies its first-use program', async ({ session }) => {
  await session.openEditor();
  await session.page.waitForFunction(() => !!(window as any).PM.EngineRender);
  const rgba = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const registration = PM.Kernel.registerEffect('async-parity', { id: 'async-parity', label: 'Async parity', group: 'Test', params: [], frag: 'o = vec4(0.25, 0.5, 0.75, 1.0); /* asynchronous first use */' });
    try {
      const project = PM.mkProject({ name: 'Async parity', w: 239, h: 137, fps: 30, dur: 2 });
      const layer = PM.mkLayer('solid', { from: 0, dur: 2, d: { color: '#FFFFFF', w: 239, h: 137 } }, project);
      layer.p['position.x'].v = 0; layer.p['position.y'].v = 0;
      layer.fx = [{ id: 'async-instance', type: 'async-parity', on: true, p: {} }];
      project.layers = [layer]; PM.replaceProject(project);
      await PM.EngineDefinitions.settle();
      const canvas = await PM.EngineRender.renderFrameTo(0.25, 239, 137, { mblur: false });
      return [...canvas.getContext('2d').getImageData(0, 0, 1, 1).data];
    } finally { registration.dispose(); }
  });
  expect(rgba[3]).toBe(255);
  await expect.poll(async () => Object.values(await decisions(session)).filter(v => v === 'native-verified\n').length).toBeGreaterThan(0);
});

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });
