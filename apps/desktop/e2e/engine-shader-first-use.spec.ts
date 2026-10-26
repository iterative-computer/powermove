/** A user effect's first use presents in one frame on the native pipeline:
 * no first-frame hold, no runner round trip, nothing persisted (E-R1). The
 * offline shader gate (pm-render `tests/shader_gate.rs`, G-E1) is what
 * judges native translations against WebGL; the app never probes.
 */
import { readdir } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, type LaunchedApp } from './helpers/app';

const STABLE = 'o = vec4(0.25, 0.5, 0.75, 1.0);';
// Catastrophic cancellation: an ill-conditioned program (WebGL itself moves
// under a one-step input change). It used to be pinned to ANGLE at first use.
const UNSTABLE = 'float a = v_st.x * u_res.x - gl_FragCoord.x; o = vec4(vec3(fract(a * 1048576.0)), 1.0);';

async function draw(session: LaunchedApp, code: string, samples = 5): Promise<{ firstMs: number; steadyMs: number[]; firstRounds: number; steadyRounds: number; legacyMaxCv: number; pixels: number[] }> {
  return session.page.evaluate(async ({ code, samples }) => {
    const PM = (window as any).PM;
    const reg = PM.Kernel.registerEffect('first-use-e2e', { id: 'first-use-e2e', label: 'First use', group: 'Test', params: [], frag: code });
    try {
      const project = PM.mkProject({ name: 'First use', w: 239, h: 137, fps: 30, dur: 2 });
      const layer = PM.mkLayer('solid', { from: 0, dur: 2, d: { color: '#FFFFFF', w: 239, h: 137 } }, project);
      layer.p['position.x'].v = 0; layer.p['position.y'].v = 0;
      layer.fx = [{ id: 'first-use-instance', type: 'first-use-e2e', on: true, p: {} }];
      project.layers = [layer]; PM.replaceProject(project);
      await PM.EngineDefinitions.settle();
      const api = PM.Kernel.api('first-use-e2e');
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

test('a user effect presents natively on first use with no runner round trip', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  await session.page.waitForFunction(() => !!(window as any).PM.EngineRender);
  const stable = await draw(session, STABLE);
  console.log('stable shader first use', { firstMs: stable.firstMs, firstRounds: stable.firstRounds, steadyMs: stable.steadyMs, steadyRounds: stable.steadyRounds });
  expect(stable.firstRounds).toBe(0);
  expect(stable.steadyRounds).toBe(0);
  expect(stable.legacyMaxCv).toBeLessThanOrEqual(1);
  const unstable = await draw(session, UNSTABLE);
  console.log('ill-conditioned shader first use', { firstMs: unstable.firstMs, firstRounds: unstable.firstRounds, steadyMs: unstable.steadyMs, steadyRounds: unstable.steadyRounds });
  expect(unstable.firstRounds).toBe(0);
  expect(unstable.steadyRounds).toBe(0);
  // Nothing is decided or persisted about a program.
  expect(await readdir(path.join(session.userData, 'Engine Cache/Shader Cache/angle-parity')).catch(() => [])).toEqual([]);
  const output = process.env['PM_SHADER_FIRST_USE_MEASURE'];
  if (output) {
    const { writeFile } = await import('node:fs/promises');
    await writeFile(output, JSON.stringify({ stable: { ...stable, pixels: undefined }, unstable: { ...unstable, pixels: undefined } }, null, 2));
  }
});

test('an asynchronous snapshot of a first-use program completes', async ({ session }) => {
  await session.openEditor();
  await session.page.waitForFunction(() => !!(window as any).PM.EngineRender);
  const rgba = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const registration = PM.Kernel.registerEffect('async-first-use', { id: 'async-first-use', label: 'Async first use', group: 'Test', params: [], frag: 'o = vec4(0.25, 0.5, 0.75, 1.0); /* asynchronous first use */' });
    try {
      const project = PM.mkProject({ name: 'Async first use', w: 239, h: 137, fps: 30, dur: 2 });
      const layer = PM.mkLayer('solid', { from: 0, dur: 2, d: { color: '#FFFFFF', w: 239, h: 137 } }, project);
      layer.p['position.x'].v = 0; layer.p['position.y'].v = 0;
      layer.fx = [{ id: 'async-instance', type: 'async-first-use', on: true, p: {} }];
      project.layers = [layer]; PM.replaceProject(project);
      await PM.EngineDefinitions.settle();
      const before = PM.EngineRender.stats.rounds;
      const canvas = await PM.EngineRender.renderFrameTo(0.25, 239, 137, { mblur: false });
      return { pixel: [...canvas.getContext('2d').getImageData(0, 0, 1, 1).data], rounds: PM.EngineRender.stats.rounds - before };
    } finally { registration.dispose(); }
  });
  expect(rgba.pixel).toEqual([64, 128, 191, 255]);
  expect(rgba.rounds).toBe(0);
});

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });
