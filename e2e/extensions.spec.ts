import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { expect, test } from './helpers/app';

/* End-to-end proof of the extension pipeline: real files on disk → main-process
 * esbuild compile → app://powermove/ext/ serving → renderer kernel activation.
 * Uses its own userData seeding via the session's temp dir before relaunch. */

async function seedExtension(userData: string, id: string, indexTs: string, manifest?: Record<string, unknown>): Promise<void> {
  const dir = path.join(userData, 'extensions', id);
  await mkdir(dir, { recursive: true });
  await writeFile(
    path.join(dir, 'manifest.json'),
    JSON.stringify({
      id,
      name: `E2E ${id}`,
      version: '1.0.0',
      apiVersion: 1,
      description: 'e2e fixture',
      author: 'user',
      ...manifest
    })
  );
  await writeFile(path.join(dir, 'index.ts'), indexTs);
}

test.describe('@extensions user extensions load through the kernel', () => {
  test('replaces panel source in place without changing the document, window, or docking', async ({ session }) => {
    const source = (label: string) => `export default function activate(api) { api.panels.register({ id:'reload-panel', title:'Reload panel', build(body) { body.textContent = '${label}'; } }); }`;
    await seedExtension(session.userData, 'reload-panel', source('Before edit'));
    await session.relaunch();
    const { page, app } = session;
    await page.waitForFunction(() => Boolean((window as any).PM.PANELS['reload-panel']));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.WS.mutate((ws: any) => PM.Layout.addPanel(ws, 'reload-panel', 'right'));
      (window as any).__reloadDocument = document;
      (window as any).__reloadProject = PM.proj;
    });
    await expect(page.locator('#panel-reload-panel .body')).toHaveText('Before edit');
    const before = await page.evaluate(() => JSON.stringify((window as any).PM.WS.current));
    const pid = app.process().pid;
    await writeFile(path.join(session.userData, 'extensions/reload-panel/index.ts'), source('After edit'));
    await page.evaluate(async () => { await (window as any).powermove.extensions.reload({ id: 'reload-panel' }); });
    await expect(page.locator('#body #panel-reload-panel .body')).toHaveText('After edit');
    expect(await page.evaluate(() => JSON.stringify((window as any).PM.WS.current))).toBe(before);
    expect(await page.evaluate(() => (window as any).__reloadDocument === document && (window as any).__reloadProject === (window as any).PM.proj)).toBe(true);
    expect(app.process().pid).toBe(pid);
    expect(app.windows()).toHaveLength(1);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
  test('a user extension activates, contributes a command and an effect, and survives disable', async ({ session }) => {
    await seedExtension(
      session.userData,
      'e2e-probe',
      `import type { PowermoveAPI } from 'powermove';
export default function activate(api: PowermoveAPI) {
  (globalThis as any).__e2eProbe = (((globalThis as any).__e2eProbe as number) ?? 0) + 1;
  api.commands.register({ id: 'e2e.probe', label: 'E2E probe', category: 'Test', run: () => 'probe-ran' });
  api.effects.register({
    id: 'e2eglow', label: 'E2E Glow', group: 'Stylize',
    params: [{ k: 'amount', label: 'Amount', def: 1, min: 0, max: 10 }],
    frag: 'o = texture(u_tex, v_st) * (1.0 + u_amount * 0.01);'
  });
}
`
    );
    await session.relaunch();
    const { page } = session;

    // Wait for the kernel to finish booting extensions (compile is async in main).
    await page.waitForFunction(() => (globalThis as unknown as { __e2eProbe?: number }).__e2eProbe === 1, undefined, { timeout: 15_000 });

    const state = await page.evaluate(() => {
      const PM = (window as unknown as Record<string, any>).PM;
      return {
        commandRan: PM.cmd('e2e.probe'),
        effectKnown: Boolean(PM.FX && PM.FX['e2eglow']),
        recordHealthy: (PM.Kernel?.extensionRecords?.() ?? PM.Kernel?.loader?.records?.() ?? [])
          .some((r: any) => r.id === 'e2e-probe' && r.health?.state === 'ok')
      };
    });
    expect(state.commandRan).toBe('probe-ran');
    expect(state.effectKnown).toBe(true);

    // Disable through the bridge; the contribution must disappear.
    const afterDisable = await page.evaluate(async () => {
      const bridge = (window as unknown as Record<string, any>).powermove;
      await bridge.extensions.setEnabled({ id: 'e2e-probe', enabled: false });
      await new Promise((resolve) => setTimeout(resolve, 500));
      const PM = (window as unknown as Record<string, any>).PM;
      return { effectKnown: Boolean(PM.FX && PM.FX['e2eglow']) };
    });
    expect(afterDisable.effectKnown).toBe(false);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a broken extension is contained: the app boots and reports the failure', async ({ session }) => {
    await seedExtension(
      session.userData,
      'e2e-broken',
      `export default function activate() { throw new Error('e2e intentional failure'); }\n`
    );
    await session.relaunch();
    const { page } = session;

    // The app must be fully alive despite the failing extension.
    await page.waitForSelector('#body .dock', { timeout: 15_000 });
    await page.waitForFunction(
      () => {
        const PM = (window as unknown as Record<string, any>).PM;
        const records = PM?.Kernel?.extensionRecords?.() ?? PM?.Kernel?.loader?.records?.() ?? [];
        return records.some(
          (r: any) => r.id === 'e2e-broken' && r.health?.state && r.health.state !== 'ok' && r.health.state !== 'disabled'
        );
      },
      undefined,
      { timeout: 15_000 }
    );

    const health = await page.evaluate(() => {
      const PM = (window as unknown as Record<string, any>).PM;
      const records = PM.Kernel?.extensionRecords?.() ?? PM.Kernel?.loader?.records?.() ?? [];
      const record = records.find((r: any) => r.id === 'e2e-broken');
      return { state: record?.health?.state ?? null, editorAlive: Boolean(PM.proj) };
    });
    expect(health.editorAlive).toBe(true);
    expect(['activation-error', 'runtime-error', 'build-error']).toContain(health.state);
  });

  test('built-in mods panel is registered', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(
      () => {
        const PM = (window as unknown as Record<string, any>).PM;
        return Boolean(PM?.PANELS && PM.PANELS['mods']);
      },
      undefined,
      { timeout: 15_000 }
    );
  });
});
