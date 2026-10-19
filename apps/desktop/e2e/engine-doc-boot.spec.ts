import { requireEngineDiskSpace } from './helpers/engine-fixups-resources';
import { build } from 'esbuild';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, launchApp, repoRoot, test } from './helpers/app';

const activationProbe = `export default function activate(api) {
  const layer = api.model.mkLayer('solid');
  const prop = api.model.P(10, { kf: [{t:0,v:10,hold:true},{t:1,v:80}] });
  layer.p.opacity = prop;
  const value = api.anim.evP(layer, prop, 0.5, 'opacity');
  if (value !== 10) throw new Error('activation evaluation: ' + value);
  api.services.register('boot-evaluation-probe', { value, at: performance.now() });
}`;

async function profile(): Promise<string> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-evaluator-boot-'));
  const ext = path.join(root, 'extensions/boot-evaluation-probe');
  await mkdir(ext, { recursive: true });
  await writeFile(path.join(ext, 'manifest.json'), JSON.stringify({ id: 'boot-evaluation-probe', name: 'Boot evaluation probe', version: '1.0.0', apiVersion: 1, author: 'user' }));
  await writeFile(path.join(ext, 'index.ts'), activationProbe);
  return root;
}

async function waitForQuietMachine(): Promise<void> {
  for (let attempt = 0; attempt < 4; attempt++) {
    const load = os.loadavg()[0];
    if (load <= 8) return;
    if (attempt < 3) await new Promise(resolve => setTimeout(resolve, 5_000));
  }
  throw new Error('B3 paired boot measurement deferred: one-minute load average stayed above 8 after three retries.');
}

// The same synchronous apiVersion-1 call in activate(), through real disk
// compilation and the production loader, with each launch's actual engine.
test('an extension evaluates a property during activate without a boot-window failure', async () => {
  const root = await profile();
  requireEngineDiskSpace();
  const app = await launchApp({ userData: root });
  try {
    await app.page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services.get('boot-evaluation-probe')));
    const answer = await app.page.evaluate(() => {
      const PM = (window as any).PM;
      return { value: PM.Kernel.services.get('boot-evaluation-probe').value, ready: PM.EngineDoc?.ready, enabled: PM.EngineDoc?.enabled, failures: PM.EngineDoc?.failures() ?? [], stats: PM.EngineDoc?.stats() };
    });
    expect(answer.value).toBe(10);
    if (answer.enabled) {
      expect(answer.ready).toBe(true);
      expect((answer.stats as any).native.calls.evP).toBeGreaterThan(0);
      expect((answer.stats as any).native.fallbacks).toEqual({});
    }
    expect(answer.failures).toEqual([]);
  } finally { await app.close(); await rm(root, { recursive: true, force: true }); }
});

test('an agent connects at launch and immediately evaluates the restored project', async () => {
  test.setTimeout(120_000);
  const root = await profile();
  const bundle = path.join(root, 'agent-bridge.cjs');
  await build({ entryPoints: [path.join(repoRoot, 'src/main/agent-tools/bridge.ts')], outfile: bundle,
    bundle: true, platform: 'node', format: 'cjs', external: ['electron'] });
  let setup: Awaited<ReturnType<typeof launchApp>> | null = null;
  let app: Awaited<ReturnType<typeof launchApp>> | null = null;
  let releaseWasm: () => void = () => {};
  try {
    requireEngineDiskSpace();
    setup = await launchApp({ userData: root, env: { POWERMOVE_NEW_ENGINE: '1' } });
    await setup.openEditor();
    const projectId = await setup.page.evaluate(async () => {
      const PM = (window as any).PM;
      const layer = PM.mkLayer('solid');
      layer.p.opacity = PM.P(10, { kf: [{ t: 0, v: 10, hold: true }, { t: 1, v: 80 }] });
      PM.proj.layers = [layer];
      PM.touch();
      PM.Projects.put(PM.proj);
      PM.Projects.putState(PM.proj.id, { time: 0.5 });
      await PM.store.flush?.();
      return PM.proj.id;
    });
    await setup.close(); setup = null;

    requireEngineDiskSpace();
    app = await launchApp({ userData: root, env: { POWERMOVE_NEW_ENGINE: '1' } });
    // Reload into a fresh renderer with the real module response held. Route
    // interception must happen before reload; a request that slipped through
    // would fail the explicit pending assertions below.
    const moduleHeld = new Promise<void>(resolve => { releaseWasm = resolve; });
    let sawModuleRequest!: () => void;
    const moduleRequested = new Promise<void>(resolve => { sawModuleRequest = resolve; });
    await app.page.route('**/engine/pm_wasm.wasm', async route => {
      sawModuleRequest();
      await moduleHeld;
      await route.continue();
    });
    const navigation = app.page.reload({ waitUntil: 'domcontentloaded' });
    void navigation.catch(() => undefined);
    let requestTimer!: ReturnType<typeof setTimeout>;
    try {
      await Promise.race([moduleRequested, new Promise<never>((_, reject) => {
        requestTimer = setTimeout(() => reject(new Error('The boot e2e did not intercept the wasm request.')), 10_000);
      })]);
    } finally { clearTimeout(requestTimer); }
    await app.page.waitForFunction(() => Boolean((window as any).PM?.AgentHarness?.test?.answerLiveAgentTool));
    const before = await app.page.evaluate(() => {
      const PM = (window as any).PM;
      return { projectId: PM.proj.id, enabled: PM.EngineDoc.enabled, ready: PM.EngineDoc.ready };
    });
    expect(before).toEqual({ projectId, enabled: true, ready: false });

    // Main connects and sends the first evaluation while pm-wasm is still
    // blocked at the network boundary. The bridge keeps this IPC request
    // pending; releasing the response then lets that same request complete.
    const dispatched = await app.app.evaluate(async ({ ipcMain, BrowserWindow }, bundle) => {
      const { PowermoveAgentToolBridge } = process.getBuiltinModule('module').createRequire(bundle)(bundle);
      const owner = BrowserWindow.getAllWindows().find(w => w.webContents.getURL().startsWith('app://powermove'))!.webContents;
      const bridge = new PowermoveAgentToolBridge(ipcMain, { mcpServerPath: '', timeoutMs: 10_000 });
      const session = await bridge.openSession({ runId: 'boot-agent-evaluation', owner, baseRevision: 0 });
      (globalThis as any).__bootAgentBridge = bridge;
      (globalThis as any).__bootAgentResponse = bridge.callTool(session, 'get_project_state', {});
      return { pending: (bridge as any).pending.size };
    }, bundle);
    expect(dispatched.pending).toBe(1);
    expect(await app.app.evaluate(() => (globalThis as any).__bootAgentBridge.pending.size)).toBe(1);
    expect(await app.page.evaluate(() => (window as any).PM.EngineDoc.ready)).toBe(false);
    releaseWasm(); releaseWasm = () => {};
    await navigation;
    const response = await app.app.evaluate(async () => {
      const bridge = (globalThis as any).__bootAgentBridge;
      try { return await (globalThis as any).__bootAgentResponse; }
      finally {
        await bridge.shutdown();
        delete (globalThis as any).__bootAgentBridge;
        delete (globalThis as any).__bootAgentResponse;
      }
    });
    expect(response.ok, JSON.stringify(response)).toBe(true);
    const state = JSON.parse(response.content[0].text);
    expect(state.composition.id).toBe(projectId);
    expect(state.layers[0].properties.find((prop: any) => prop.path === 'opacity').value).toBe(10);
    const engine = await app.page.evaluate(() => {
      const doc = (window as any).PM.EngineDoc;
      return { ready: doc.ready, failures: doc.failures(), calls: doc.stats().native.calls.evP,
        fallbacks: doc.stats().native.fallbacks };
    });
    expect(engine.ready).toBe(true);
    expect(engine.failures).toEqual([]);
    expect(engine.calls).toBeGreaterThan(0);
    expect(engine.fallbacks).toEqual({});
  } finally {
    releaseWasm();
    await setup?.close();
    await app?.app.evaluate(async () => {
      const bridge = (globalThis as any).__bootAgentBridge;
      if (bridge) await bridge.shutdown();
      delete (globalThis as any).__bootAgentBridge;
      delete (globalThis as any).__bootAgentResponse;
    }).catch(() => undefined);
    await app?.close();
    await rm(root, { recursive: true, force: true });
  }
});

test('measure five alternating boot pairs', async () => {
  test.skip(!process.env.PM_FIXUPS2_MEASURE, 'opt-in paired measurements');
  test.setTimeout(600_000);
  const rows = [];
  for (let pair = 0; pair < 5; pair++) for (const enabled of pair % 2 === 0 ? [false, true] : [true, false]) {
    await waitForQuietMachine();
    const root = await profile();
    requireEngineDiskSpace();
    const started = performance.now();
    const app = await launchApp({ userData: root, env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
    try {
      await app.page.waitForFunction(() => Boolean((window as any).PM?.ProjectsScreen));
      const uiMs = performance.now() - started;
      const waits = await app.page.evaluate(async () => {
        const PM = (window as any).PM, uiAt = performance.now();
        await PM.Kernel.loader.builtinsReady;
        return { uiAt, builtinsAt: performance.now() };
      });
      await app.page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services.get('boot-evaluation-probe')));
      const ready = await app.page.evaluate(() => {
        const PM = (window as any).PM;
        return { connectionBeforeOpen:PM.EngineConnection?.status()??null, stats: PM.EngineDoc?.stats(), failures: PM.EngineDoc?.failures() ?? [], probe: PM.Kernel.services.get('boot-evaluation-probe') };
      });
      const openStarted = performance.now();
      await app.openEditor();
      const opened = performance.now();
      const editorMs = opened - started;
      const openMs = opened - openStarted;
      const phases = await app.page.evaluate(() => ({
        connectionAfterOpen:(window as any).PM.EngineConnection?.status()??null,
        marks: performance.getEntriesByType('mark').filter(e=>e.name.startsWith('powermove:')).map(e=>({name:e.name,at:e.startTime})),
        installs: performance.getEntriesByType('measure').filter(e=>e.name.startsWith('powermove:install:')).map(e=>({name:e.name,ms:e.duration})),
        warmups: performance.getEntriesByType('measure').filter(e=>e.name==='powermove:font-prewarm').map(e=>({at:e.startTime,ms:e.duration})),
        resources: performance.getEntriesByType('resource').filter(e=>e.name.includes('pm_wasm')).map(e=>e.toJSON()),
      }));
      rows.push({ pair, enabled, uiMs, editorMs, openMs, ...waits, ...ready, phases });
      expect(ready.failures).toEqual([]);
      expect(ready.probe.value).toBe(10);
    } finally { await app.close(); await rm(root, { recursive: true, force: true }); }
  }
  if (process.env.PM_FIXUPS2_EVIDENCE) {
    await mkdir(process.env.PM_FIXUPS2_EVIDENCE, { recursive: true });
    await writeFile(path.join(process.env.PM_FIXUPS2_EVIDENCE, 'boot-r2.json'), JSON.stringify(rows, null, 2));
  }
  console.log(JSON.stringify(rows));
});

// Separate from the five gate pairs: V8 sampling changes the timings it observes.
test('profile the editor-open path with each engine', async () => {
  test.skip(!process.env.PM_FIXUPS2_PROFILE, 'opt-in startup CPU profile');
  const evidence = process.env.PM_FIXUPS2_EVIDENCE ?? '/tmp/ne-fixups2-evidence';
  await mkdir(evidence, { recursive:true });
  for (const enabled of [false,true]) {
    const root = await profile();
    requireEngineDiskSpace();
    const app = await launchApp({ userData:root, env:{ POWERMOVE_NEW_ENGINE:enabled?'1':'0' } });
    try {
      const cdp = await app.page.context().newCDPSession(app.page);
      await cdp.send('Profiler.enable');
      await cdp.send('Profiler.setSamplingInterval', { interval:500 });
      await cdp.send('Profiler.start');
      await app.page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services.get('boot-evaluation-probe')));
      const before = await app.page.evaluate(() => (window as any).PM.EngineConnection?.status()??null);
      const engineBefore = await app.app.evaluate(async () => (globalThis as any).__powermoveNativeEngine?.engine?.report('interactive')??null);
      requireEngineDiskSpace();
      const started = performance.now();
      await app.openEditor();
      const openMs = performance.now()-started;
      const result = await cdp.send('Profiler.stop');
      const after = await app.page.evaluate(() => (window as any).PM.EngineConnection?.status()??null);
      const engineAfter = await app.app.evaluate(async () => (globalThis as any).__powermoveNativeEngine?.engine?.report('interactive')??null);
      await writeFile(path.join(evidence,`editor-r2-${enabled?'engine':'today'}-diagnostic.json`),JSON.stringify({openMs,before,after,engineBefore,engineAfter},null,2));
      await writeFile(path.join(evidence,`editor-r2-${enabled?'engine':'today'}.cpuprofile`),JSON.stringify(result.profile));
      await cdp.detach();
    } finally { await app.close(); await rm(root,{recursive:true,force:true}); }
  }
});
