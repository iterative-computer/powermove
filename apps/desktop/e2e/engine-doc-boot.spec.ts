import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, launchApp, test } from './helpers/app';

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

// The same synchronous apiVersion-1 call in activate(), through real disk
// compilation and the production loader, with each launch's actual engine.
test('an extension evaluates a property during activate without a boot-window failure', async () => {
  const root = await profile();
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

test('measure five alternating boot pairs', async () => {
  test.skip(!process.env.PM_FIXUPS2_MEASURE, 'opt-in paired measurements');
  test.setTimeout(240_000);
  const rows = [];
  for (let pair = 0; pair < 5; pair++) for (const enabled of [false, true]) {
    const root = await profile();
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
    const app = await launchApp({ userData:root, env:{ POWERMOVE_NEW_ENGINE:enabled?'1':'0' } });
    try {
      const cdp = await app.page.context().newCDPSession(app.page);
      await cdp.send('Profiler.enable');
      await cdp.send('Profiler.setSamplingInterval', { interval:500 });
      await cdp.send('Profiler.start');
      await app.page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services.get('boot-evaluation-probe')));
      const before = await app.page.evaluate(() => (window as any).PM.EngineConnection?.status()??null);
      const engineBefore = await app.app.evaluate(async () => (globalThis as any).__powermoveNativeEngine?.engine?.report('interactive')??null);
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
