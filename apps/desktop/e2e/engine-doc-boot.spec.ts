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
      await app.page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services.get('boot-evaluation-probe')));
      const ready = await app.page.evaluate(() => {
        const PM = (window as any).PM;
        return { stats: PM.EngineDoc?.stats(), failures: PM.EngineDoc?.failures() ?? [], probe: PM.Kernel.services.get('boot-evaluation-probe') };
      });
      await app.openEditor();
      rows.push({ pair, enabled, uiMs, editorMs: performance.now() - started, ...ready });
      expect(ready.failures).toEqual([]);
      expect(ready.probe.value).toBe(10);
    } finally { await app.close(); await rm(root, { recursive: true, force: true }); }
  }
  if (process.env.PM_FIXUPS2_EVIDENCE) {
    await mkdir(process.env.PM_FIXUPS2_EVIDENCE, { recursive: true });
    await writeFile(path.join(process.env.PM_FIXUPS2_EVIDENCE, 'boot.json'), JSON.stringify(rows, null, 2));
  }
  console.log(JSON.stringify(rows));
});
