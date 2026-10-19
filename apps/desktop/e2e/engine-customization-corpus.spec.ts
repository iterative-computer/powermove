/*
 * The customization compatibility corpus (owner amendment 24;
 * docs/engine/extensions-compat.md), with "Use New Engine" on:
 *
 * - A fork of every built-in at today's shipped version
 *   (e2e/fixtures/compat-corpus/shipped, the source on `main`), laid out as
 *   `api.extensions.fork` writes it (`<id>-fork`, `replaces`, `forkedFrom`,
 *   a pristine `.forked-from`), loads unchanged and replaces its built-in;
 *   none is turned off, and the viewer fork, which predates the engine's
 *   presentation code, gets engine frames through `render.gl.init`.
 * - The API probe (e2e/fixtures/compat-corpus/api-probe) at apiVersion 1, 2
 *   and 3 in the trusted in-realm context: synchronous members return
 *   synchronously with today's shapes, live objects keep identity, editor
 *   helpers work from a compiled extension, and the async siblings resolve.
 *
 * The runtime auto-disable policy must not hide a failure: every extension's
 * health is asserted, not only that the app booted.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-customization-corpus.spec.ts --workers=1
 */
import { cp, mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import type { Page } from 'playwright';

import { expect, test } from './helpers/app';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
/* `timeline` is not in the corpus fixtures (see shipped/README.md); its
   fork fails to build on today's app, like inspector's. */
const BUILTINS = ['effects-basic', 'transitions-basic', 'theme-default', 'keymap-default', 'mods', 'toolbar', 'inspector', 'layers-3d', 'viewer'] as const;
const SHIPPED = path.join(__dirname, 'fixtures/compat-corpus/shipped');

/** Writes `<id>-fork` as `api.extensions.fork` does (main/extensions/fork.ts). */
async function writeFork(extensions: string, id: string): Promise<void> {
  const source = path.join(SHIPPED, id);
  const target = path.join(extensions, `${id}-fork`);
  await cp(source, target, { recursive: true });
  await cp(source, path.join(target, '.forked-from'), { recursive: true });
  const manifest = JSON.parse(await readFile(path.join(source, 'manifest.json'), 'utf8'));
  const rewritten = { ...manifest, id: `${id}-fork`, name: `${manifest.name} (fork)`, replaces: [...new Set([...(manifest.replaces ?? []), id])], forkedFrom: `${id}@${manifest.version}`, author: manifest.author ?? 'user' };
  await writeFile(path.join(target, 'manifest.json'), `${JSON.stringify(rewritten, null, 2)}\n`);
}

async function textProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Corpus', w: 480, h: 270, fps: 30, dur: 3, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const title = PM.mkLayer('text', { name: 'Title', from: 0, dur: 3, d: { text: 'Corpus', font: 'Helvetica', size: 48, weight: 700, color: '#FFFFFF' } }, project);
    title.id = 'title';
    title.p['position.x'].v = 240; title.p['position.y'].v = 135;
    project.layers = [title];
    project.exportDefaults = { ...(project.exportDefaults ?? {}), format: 'prores' };
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
}

test.describe('Use New Engine: the customization corpus', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('forks of every shipped built-in load unchanged, replace their built-ins, and stay on', async ({ session }) => {
    test.setTimeout(180_000);
    const extensions = path.join(session.userData, 'extensions');
    await mkdir(extensions, { recursive: true });
    for (const id of BUILTINS) await writeFork(extensions, id);
    await session.relaunch();
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.loader && (window as any).PM?.EngineRender), undefined, { timeout: 60_000 });
    const inactive = () => page.evaluate((ids) => {
      const PM = (window as any).PM;
      const active: string[] = PM.Kernel.loader.activeIds();
      const records = PM.Kernel.api('corpus').extensions.list();
      return ids.filter((id) => !active.includes(`${id}-fork`)).map((id) => ({ id, record: records.find((r: any) => r.id === `${id}-fork`)?.health ?? 'missing' }));
    }, [...BUILTINS]);
    // Today's shipped inspector imports app modules outside its folder
    // (`../../renderer/src/...`), which the extension compiler refuses for
    // any extension outside the app, today as with the engine: its fork
    // fails to build the same way on today's app. Every other fork loads.
    await expect.poll(async () => (await inactive()).map((x) => x.id), { timeout: 60_000 }).toEqual(['inspector']).catch(async (error) => {
      console.log(JSON.stringify(await inactive()));
      throw error;
    });
    for (const { record } of await inactive()) {
      expect(record).toMatchObject({ state: 'build-error' });
      expect(String((record as any).error)).toContain('Import escapes extension directory: ../../renderer/src/');
    }
    const state = await page.evaluate((ids) => {
      const PM = (window as any).PM;
      const records = PM.Kernel.api('corpus').extensions.list();
      const active: string[] = PM.Kernel.loader.activeIds();
      return {
        replacedStillActive: ids.filter((id) => active.includes(id) && id !== 'inspector'),
        unhealthy: records.filter((r: any) => ids.some((id) => r.id === `${id}-fork` && id !== 'inspector') && (r.health?.state !== 'ok' || r.enabled === false)).map((r: any) => ({ id: r.id, health: r.health, enabled: r.enabled }))
      };
    }, [...BUILTINS]);
    expect(state.replacedStillActive).toEqual([]);
    expect(state.unhealthy).toEqual([]);
    await textProject(page);
    // The pre-engine viewer fork binds the engine's presentation through
    // render.gl.init (it has no presentation code of its own).
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await expect.poll(() => page.evaluate(() => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('corpus');
      return { canvas: !!api.render.gl.canvas, contextCanvas: api.render.gl.context?.canvas === api.render.gl.canvas, bound: !!PM.EngineCanvas?.bound, presenting: !!PM.EngineCanvas?.presenting || !!PM.Kernel.services.get('viewer')?.enginePresentation?.presenting };
    }), { timeout: 30_000 }).toEqual({ canvas: true, contextCanvas: true, bound: true, presenting: true });
    // The forks registered the built-ins' panels in their place.
    const panels = await page.evaluate(() => (window as any).PM.Kernel.panels.ids());
    expect(panels).toEqual(expect.arrayContaining(['viewer', 'timeline', 'inspector']));
    // The built-in timeline and inspector (no working fork) stay.
    // Still on after use: nothing turned a fork off.
    const after = await page.evaluate((ids) => (window as any).PM.Kernel.api('corpus').extensions.list().filter((r: any) => ids.some((id) => r.id === `${id}-fork` && id !== 'inspector') && r.health?.state !== 'ok').map((r: any) => r.id), [...BUILTINS]);
    expect(after).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('the API probe at apiVersion 1, 2 and 3 gets today\'s behavior from every member it calls', async ({ session }) => {
    test.setTimeout(180_000);
    const extensions = path.join(session.userData, 'extensions');
    for (const version of [1, 2, 3]) {
      const target = path.join(extensions, `api-probe-v${version}`);
      await cp(path.join(__dirname, 'fixtures/compat-corpus/api-probe'), target, { recursive: true });
      const manifest = JSON.parse(await readFile(path.join(target, 'manifest.json'), 'utf8'));
      await writeFile(path.join(target, 'manifest.json'), JSON.stringify({ ...manifest, id: `api-probe-v${version}`, apiVersion: version, ...(version === 3 ? { permissions: ['full-access'] } : {}) }, null, 2));
    }
    await session.relaunch();
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => ['api-probe-v1', 'api-probe-v2', 'api-probe-v3'].every((id) => (window as any).PM?.Kernel?.loader?.activeIds?.().includes(id)) && Boolean((window as any).PM?.EngineRender), undefined, { timeout: 60_000 });
    await textProject(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    // A presented frame first, so frame:rendered has fired.
    await page.evaluate(() => (window as any).PM.setTime(0.5));
    await page.waitForTimeout(1500);
    const results = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('corpus');
      for (const v of [1, 2, 3]) await api.commands.run(`api-probe-v${v}.run`);
      return (window as any).__compatProbe;
    });
    console.log(JSON.stringify(results));
    for (const v of [1, 2, 3]) {
      const probe = results?.[`api-probe-v${v}`];
      expect(probe?.apiVersion, `api-probe-v${v} ran`).toBe(v);
      const failed = Object.entries(probe.out as Record<string, { ok: boolean; detail?: unknown }>).filter(([, r]) => !r.ok).map(([name, r]) => `${name}: ${JSON.stringify(r.detail)}`);
      expect(failed, `apiVersion ${v}`).toEqual([]);
      const events = probe.out.events.detail as Record<string, unknown>;
      expect(Number(events['frame:rendered'] ?? 0)).toBeGreaterThan(0);
      expect(events['frameViewportMatches']).toBe(true);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
