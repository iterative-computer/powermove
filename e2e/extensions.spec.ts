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
    // An initial refresh and a watcher refresh can legitimately activate the
    // same extension twice before this assertion observes it.
    await page.waitForFunction(() => Number((globalThis as unknown as { __e2eProbe?: number }).__e2eProbe) >= 1, undefined, { timeout: 15_000 });

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

  test('timeline replacements swap with the built-in timeline without corrupting the live panel', async ({ session }) => {
    await seedExtension(
      session.userData,
      'e2e-timeline-replacement',
      `export default function activate(api) {
  api.panels.register({
    id: 'timeline', title: 'Replacement Timeline', headless: true, flush: true, noscroll: true,
    size: 340, moveSlot: '#replacement-head',
    build(body) {
      const head = document.createElement('div');
      head.id = 'replacement-head';
      const marker = document.createElement('div');
      marker.dataset.timelineReplacement = 'active';
      marker.textContent = 'Replacement timeline';
      body.replaceChildren(head, marker);
    }
  });
}`,
      { replaces: ['timeline'] }
    );
    await session.relaunch();
    const { page, app } = session;
    await expect(page.locator('#body #panel-timeline [data-timeline-replacement="active"]')).toBeVisible();
    const before = await page.evaluate(() => {
      const PM = (window as any).PM;
      (window as any).__replacementToggleDocument = document;
      (window as any).__replacementToggleProject = PM.proj;
      return { dock: PM.Layout.findPanel(PM.WS.current, 'timeline')?.dock?.id };
    });
    const pid = app.process().pid;

    for (let cycle = 0; cycle < 2; cycle += 1) {
      await page.evaluate(async () => {
        await (window as any).powermove.extensions.setEnabled({ id: 'e2e-timeline-replacement', enabled: false });
      });
      await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.loader.activeIds())).toContain('timeline');
      await expect(page.locator('#body #panel-timeline #tl-canvas')).toHaveCount(1);
      await expect(page.locator('#body #panel-timeline .panel-move-handle')).toHaveCount(1);

      await page.evaluate(async () => {
        await (window as any).powermove.extensions.setEnabled({ id: 'e2e-timeline-replacement', enabled: true });
      });
      await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.loader.activeIds())).toContain('e2e-timeline-replacement');
      await expect(page.locator('#body #panel-timeline [data-timeline-replacement="active"]')).toBeVisible();
      await expect(page.locator('#body #panel-timeline .panel-move-handle')).toHaveCount(1);
    }

    const after = await page.evaluate(() => {
      const PM = (window as any).PM;
      return {
        dock: PM.Layout.findPanel(PM.WS.current, 'timeline')?.dock?.id,
        sameDocument: (window as any).__replacementToggleDocument === document,
        sameProject: (window as any).__replacementToggleProject === PM.proj
      };
    });
    expect(after).toEqual({ dock: before.dock, sameDocument: true, sameProject: true });
    expect(app.process().pid).toBe(pid);
    expect(app.windows()).toHaveLength(1);
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

  test('turning the built-in timeline off and on restores its live panel in place', async ({ session }) => {
    const { page, app } = session;
    await expect(page.locator('#panel-timeline')).toBeVisible();
    const pid = app.process().pid;
    const before = await page.evaluate(() => {
      const PM = (window as any).PM;
      (window as any).__timelineToggleDocument = document;
      (window as any).__timelineToggleProject = PM.proj;
      return {
        dock: PM.Layout.findPanel(PM.WS.current, 'timeline')?.dock?.id,
        active: PM.Kernel.loader.activeIds().includes('timeline')
      };
    });
    expect(before.active).toBe(true);

    await page.evaluate(async () => {
      await (window as any).powermove.extensions.setEnabled({ id: 'timeline', enabled: false });
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.loader.activeIds().includes('timeline'))).toBe(false);

    await page.evaluate(async () => {
      await (window as any).powermove.extensions.setEnabled({ id: 'timeline', enabled: true });
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.loader.activeIds().includes('timeline'))).toBe(true);
    await expect(page.locator('#panel-timeline')).toBeVisible();
    await expect(page.locator('#panel-timeline #tl-canvas')).toHaveCount(1);
    await expect(page.locator('#panel-timeline .panel-move-handle')).toHaveCount(1);

    const after = await page.evaluate(() => {
      const PM = (window as any).PM;
      return {
        dock: PM.Layout.findPanel(PM.WS.current, 'timeline')?.dock?.id,
        sameDocument: (window as any).__timelineToggleDocument === document,
        sameProject: (window as any).__timelineToggleProject === PM.proj,
        transportCount: document.querySelectorAll('#panel-timeline .tl-transport').length
      };
    });
    expect(after).toEqual({ dock: before.dock, sameDocument: true, sameProject: true, transportCount: 1 });
    expect(app.process().pid).toBe(pid);
    expect(app.windows()).toHaveLength(1);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
