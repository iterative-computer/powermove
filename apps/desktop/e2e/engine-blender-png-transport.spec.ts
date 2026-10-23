import { readFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from './helpers/app';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

/** Original image bytes are the only fixture inputs; no captured PNG is injected. */
const avif = readFileSync(path.join(__dirname, 'fixtures/still-avif-icc-p3-alpha.avif')).toString('base64');
const svg = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="16" fill="#ff7733"/><rect y="16" width="32" height="16" fill="#3388ff" opacity=".55"/><circle cx="16" cy="16" r="9" fill="#fff" opacity=".4"/></svg>').toString('base64');
const sources = [
  { label: 'AVIF', name: 'still-avif-icc-p3-alpha.avif', mime: 'image/avif', encoded: avif },
  { label: 'SVG', name: 'blender-map.svg', mime: 'image/svg+xml', encoded: svg },
] as const;

for (const source of sources) test(`Blender preview obtains its ${source.label} map through the editor compatFrame PNG producer`, async ({ session }) => {
  test.setTimeout(240_000);
  await session.openEditor();
  const { page, app } = session;
  await expect.poll(() => app.evaluate(() => (globalThis as any).__powermoveNativeEngine?.engine?.status()?.interactive?.state), { timeout: 30_000 }).toBe('ready');
  const available = await page.evaluate(async () => (window as any).PM.Kernel.api('blender-png-transport').scene3d.blenderStatus());
  test.skip(!available.available, `Install Blender to run the PNG transport proof: ${available.error ?? 'unavailable'}`);

  // Count any old renderer Blender job without changing its result. The
  // engine's host-service Blender worker uses a different main-side path.
  await app.evaluate(({ ipcMain }) => {
    const ipc = ipcMain as any;
    const original = ipc._invokeHandlers.get('blender:run');
    if (!original) throw new Error('Missing legacy Blender IPC diagnostic');
    (globalThis as any).__oldBlenderRunsForPngProof = 0;
    ipc.removeHandler('blender:run');
    ipc.handle('blender:run', (...args: any[]) => {
      if (args[1]?.operation === 'render') (globalThis as any).__oldBlenderRunsForPngProof++;
      return original(...args);
    });
  });

  await page.evaluate(async source => {
    const PM = (window as any).PM;
    const transport = PM.EngineRender as any;
    const original = transport.answerAsk.bind(transport);
    const requests: Array<{ asset: string; content: string }> = [];
    const answers: Array<{ key: string; width: number; height: number; png: boolean; length: number }> = [];
    (window as any).__blenderPngCompatRequests = requests;
    (window as any).__blenderPngCompatAnswers = answers;
    const production = transport.producers.get('blenderPng');
    if (typeof production !== 'function') throw new Error('The production Blender PNG producer is not registered');
    // Observe the production producer's actual bytes; forward them verbatim.
    transport.producers.set('blenderPng', async (request: any) => {
      const answer = await production(request);
      answers.push({ key: answer.key, width: answer.width, height: answer.height,
        png: answer.data.length >= 8 && [137, 80, 78, 71, 13, 10, 26, 10].every((n, i) => answer.data[i] === n),
        length: answer.data.length });
      return answer;
    });
    // The transport's installed onCompatAsk callback invokes this method by
    // property lookup. Its actual registered BlenderPngs producer still runs.
    transport.answerAsk = async (ask: any) => {
      if (ask.request?.source === 'blenderPng') {
        requests.push({ asset: ask.request.blender?.asset, content: Array.from(ask.request.blender?.content ?? [], (n: number) => n.toString(16).padStart(2, '0')).join('') });
      }
      return original(ask);
    };

    PM.replaceProject(PM.mkProject({ name: 'Chromium Blender PNG transport', w: 192, h: 108, fps: 24, dur: 1 }));
    const bytes = Uint8Array.from(atob(source.encoded), c => c.charCodeAt(0));
    const asset = await PM.assets.add(new File([bytes], source.name, { type: source.mime }), { silent: true });
    const api = PM.Kernel.api('blender-png-transport');
    const add = (edit: any) => { const result = api.scene3d.edit(edit); if (!result.ok) throw new Error(result.message); };
    add({ operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 5 } } });
    add({ operation: 'add_object', object: { id: 'mapped', source: { primitive: 'box' }, material: { p: { color: '#ffffff', emissive: '#ffffff', emissiveIntensity: 1 }, maps: { color: asset.id, emissive: asset.id } } } });
    const setting = api.scene3d.setRendering({ enabled: true, engine: 'eevee', samples: 4, previewSamples: 2, previewScale: 1, device: 'cpu' });
    if (!setting.ok) throw new Error(setting.message);
    PM.setTime(0);
    PM.Projects?.put?.(PM.proj);
    const stored = PM.L('mapped')?.d?.data?.object?.material?.maps;
    const described = api.scene3d.describe().objects.find((object: any) => object.id === 'mapped')?.material?.maps;
    const mirrored = PM.EngineDoc?.snapshot?.()?.json;
    const canonical = typeof mirrored === 'string' ? JSON.parse(mirrored) : null;
    // canonicalProject moves an open composition's layers into comps[compId].
    const mirrorLayers = canonical?.compId ? canonical.comps?.[canonical.compId]?.layers : canonical?.layers;
    const mirrorMaps = mirrorLayers?.find((layer: any) => layer.id === 'mapped')?.d?.data?.object?.material?.maps;
    if (stored?.color !== asset.id || stored?.emissive !== asset.id || described?.color !== asset.id || described?.emissive !== asset.id ||
      mirrorMaps?.color !== asset.id || mirrorMaps?.emissive !== asset.id) {
      throw new Error(`The Blender map is absent from the stored, evaluated, or mirrored scene: ${JSON.stringify({ asset: asset.id, stored, described, mirrorMaps })}`);
    }
    (window as any).__blenderPngAssetId = asset.id;
  }, source);

  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
  await expect.poll(() => page.evaluate(() => {
    const session = (window as any).PM.EngineConnection.session();
    return !!session && session.id !== null && !session.closed && !session.resyncPending && (session.applied ?? 1) > 0;
  }), { timeout: 20_000 }).toBe(true);
  // Solid shading has not requested a Blender preview, so this synchronous
  // capture is cold. It must give today's immediate prepare-frame error; a
  // compatFrame ask here would wait on the blocked renderer IPC turn.
  const cold = await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('blender-png-transport');
    const preview = api.scene3d.previewState();
    const before = (window as any).__blenderPngCompatRequests.length;
    const started = performance.now();
    let message: string | null = null;
    try { PM.EngineRender.snapshotSync({ time: 0, width: 192, height: 108, pixels: 'opaque', motionBlurSamples: 0, background: true }); }
    catch (error) { message = error instanceof Error ? error.message : String(error); }
    return { shading: preview.shading, busy: preview.busy, before, after: (window as any).__blenderPngCompatRequests.length, elapsedMs: performance.now() - started, message };
  });
  expect(cold.shading).toBe('solid');
  expect(cold.busy).toBe(false);
  expect(cold.before).toBe(0);
  expect(cold.after).toBe(0);
  expect(cold.elapsedMs).toBeLessThan(5_000);
  expect(cold.message).toBe('The Blender frame is not prepared. Render this frame before capturing it.');

  await page.evaluate(async () => {
    const api = (window as any).PM.Kernel.api('blender-png-transport');
    api.scene3d.viewport.run('shading', 'material');
    await api.scene3d.renderFrame();
  });

  const previewWork = await app.evaluate(async () => (await (globalThis as any).__powermoveNativeEngine.engine.report('interactive'))?.presenter?.blenderPreview);

  const proof = await page.evaluate(() => {
    const PM = (window as any).PM;
    const frame = PM.Kernel.services.get('viewer')?.enginePresentation?.latest;
    return {
      route: PM.EngineConnection.status().route,
      requests: (window as any).__blenderPngCompatRequests,
      answers: (window as any).__blenderPngCompatAnswers,
      asset: (window as any).__blenderPngAssetId,
      preview: apiState(PM),
      presented: !!frame && !frame.standIn,
    };
    function apiState(pm: any) { return pm.Kernel.api('blender-png-transport').scene3d.previewState(); }
  });
  const oldRuns = await app.evaluate(() => (globalThis as any).__oldBlenderRunsForPngProof as number);
  console.log(`Blender ${source.label} PNG transport: ${JSON.stringify({ ...proof, previewWork, oldRuns })}`);
  expect(proof.route).toBe('engine');
  expect(previewWork.jobs).toBeGreaterThan(0);
  expect(proof.requests.length).toBeGreaterThan(0);
  expect(proof.requests.every((request: { asset: string; content: string }) => request.asset === proof.asset && /^[0-9a-f]{64}$/.test(request.content))).toBe(true);
  expect(proof.answers.length).toBeGreaterThan(0);
  expect(proof.answers.every((answer: { key: string; width: number; height: number; png: boolean; length: number }) =>
    answer.key === proof.requests[0].content && answer.width > 0 && answer.height > 0 && answer.png && answer.length > 33)).toBe(true);
  expect(proof.preview.error).toBeNull();
  expect(proof.preview.busy).toBe(false);
  expect(proof.presented).toBe(true);
  expect(oldRuns).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
