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
import { execFileSync } from 'node:child_process';
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { Page } from 'playwright';

import { EDITOR_HELPER_EXPORTS } from '../src/shared/extension-runtime';
import { expect, launchApp, test } from './helpers/app';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
/* `timeline` is not a checked-in fixture (see shipped/README.md): its fork
   is written from git history (main at 470eeaf3) at test time. */
const BUILTINS = ['effects-basic', 'transitions-basic', 'theme-default', 'keymap-default', 'mods', 'toolbar', 'timeline', 'inspector', 'layers-3d', 'viewer'] as const;
/* Today's shipped inspector and timeline import app modules outside their
   folders (`../../renderer/src/...`), which the extension compiler refuses
   for any extension outside the app, on today's app as with the engine:
   their forks fail to build the same way on today's app. */
const FAIL_TODAY = ['inspector', 'timeline'];
const SHIPPED = path.join(__dirname, 'fixtures/compat-corpus/shipped');

/** Writes `<id>-fork` as `api.extensions.fork` does (main/extensions/fork.ts). */
async function writeFork(extensions: string, id: string): Promise<void> {
  const target = path.join(extensions, `${id}-fork`);
  if (id === 'timeline') {
    await materialize(SHIPPED_COMMIT, id, target);
    await materialize(SHIPPED_COMMIT, id, path.join(target, '.forked-from'));
  } else {
    await cp(path.join(SHIPPED, id), target, { recursive: true });
    await cp(path.join(SHIPPED, id), path.join(target, '.forked-from'), { recursive: true });
  }
  const manifest = JSON.parse(await readFile(path.join(target, '.forked-from', 'manifest.json'), 'utf8'));
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

/** The commit `api.extensions.fork` first shipped in (2026-09-13): the oldest fork shape. */
const OLDEST_FORK_COMMIT = '155481d0';
/** `main` before the native engine: today's shipped built-ins. */
const SHIPPED_COMMIT = '470eeaf3';
const REPO = path.resolve(__dirname, '../../..');

/** Writes `apps/desktop/src/extensions/<id>` as of `commit` (from git history) to `target`, without tests. */
async function materialize(commit: string, id: string, target: string): Promise<void> {
  await mkdir(target, { recursive: true });
  const tar = execFileSync('git', ['-C', REPO, 'archive', '--format=tar', commit, `apps/desktop/src/extensions/${id}`], { maxBuffer: 64 * 1024 * 1024 });
  execFileSync('tar', ['-x', '-C', target, '--strip-components=5', '--exclude=*.test.ts'], { input: tar });
}

/** Waits until every id is active; on a timeout, prints their records (health, trust, errors). */
async function waitActive(page: Page, ids: string[], check: 'active' | 'command' = 'active'): Promise<void> {
  try {
    await page.waitForFunction(({ ids, check }) => ids.every((id) => check === 'active'
      ? (window as any).PM?.Kernel?.loader?.activeIds?.().includes(id)
      : (window as any).PM?.Kernel?.commands?.get(`${id}.run`)) && Boolean((window as any).PM?.EngineRender), { ids, check }, { timeout: 60_000 });
  } catch (error) {
    const records = await page.evaluate((ids) => (window as any).PM?.Kernel?.api('corpus')?.extensions?.list?.().filter((r: any) => ids.includes(r.id)).map((r: any) => ({ id: r.id, trust: r.trust, enabled: r.enabled, health: r.health })), ids).catch(() => null);
    console.log('not active:', JSON.stringify(records));
    throw error;
  }
}

const EVENTS = ['scene3d:selection', 'inspector:changed', 'project:changed', 'selection', 'time', 'transport', 'fonts', 'layout', 'theme:changed', 'extension:loaded', 'extension:unloaded', 'extensions:ready', 'extensions:changed', 'frame:rendered', 'overlay', 'invalidate'];

/** Whether a viewer replacement is on and shows the composition: with the
 *  engine, its presentation is up and the frame reads back through the old
 *  viewer's own `render.gl.context`; today, the compositor's canvas is in the
 *  document and holds the frame. */
async function viewerShows(page: Page, id: string, engine: boolean): Promise<{ works: boolean; detail: unknown }> {
  await page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.loader), undefined, { timeout: 60_000 });
  await expect.poll(() => page.evaluate((id) => (window as any).PM.Kernel.api('corpus').extensions.list().find((r: any) => r.id === id)?.health?.state ?? 'missing', id), { timeout: 60_000 }).toBe('ok');
  const active = await page.evaluate(() => (window as any).PM.Kernel.loader.activeIds());
  expect(active).toContain(id);
  expect(active).not.toContain('viewer');
  await textProject(page);
  if (engine) await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
  await page.waitForTimeout(1500);
  return page.evaluate((engine) => {
    const PM = (window as any).PM;
    const api = PM.Kernel.api('corpus');
    const gl: WebGL2RenderingContext | null = engine ? api.render.gl.context : PM.GL?.gl ?? null;
    const canvas = gl?.canvas as HTMLCanvasElement | undefined;
    let white = 0;
    if (gl) {
      const px = new Uint8Array(gl.drawingBufferWidth * gl.drawingBufferHeight * 4);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.readPixels(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight, gl.RGBA, gl.UNSIGNED_BYTE, px);
      for (let i = 0; i < px.length; i += 4) if (px[i]! > 200 && px[i + 1]! > 200 && px[i + 2]! > 200) white++;
    }
    const presenting = engine ? !!PM.EngineCanvas?.presenting : true;
    const connected = !!canvas?.isConnected;
    return { works: !!gl && connected && presenting && white > 50, detail: { context: !!gl, connected, presenting, white, bound: !!PM.EngineCanvas?.bound, canvases: [...document.querySelectorAll('#stage canvas')].map((c) => c.id) } };
  }, engine);
}

/** The probe's project: a keyed text, a shape, a group with a child, image,
 *  video and audio clips and a model asset, all imported for real. */
async function probeProject(page: Page): Promise<Record<string, string>> {
  const read = async (name: string) => (await readFile(path.join(__dirname, 'fixtures', name))).toString('base64');
  const media = { png: await read('still-red.png'), mp4: await read('h264-aac.mp4'), wav: await read('tone.wav') };
  return page.evaluate(async (media) => {
    const PM = (window as any).PM;
    const bytes = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const project = PM.mkProject({ name: 'Probe', w: 480, h: 270, fps: 30, dur: 3, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    project.exportDefaults = { ...(project.exportDefaults ?? {}), format: 'prores' };
    PM.replaceProject(project);
    const image = await PM.assets.add(new File([bytes(media.png)], 'red.png', { type: 'image/png' }), { silent: true });
    const video = await PM.assets.add(new File([bytes(media.mp4)], 'clip.mp4', { type: 'video/mp4' }), { silent: true });
    const audio = await PM.assets.add(new File([bytes(media.wav)], 'tone.wav', { type: 'audio/wav' }), { silent: true });
    // A glTF model (today's `object3d` is built for glTF; OBJ keeps `mesh`):
    // an OBJ with its material and texture packs into a GLB.
    const obj = 'mtllib tri.mtl\nv -1 -1 0\nv 1 -1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt .5 1\nusemtl Paint\nf 1/1 2/2 3/3\n';
    const packed = await PM.Kernel.api('corpus').scene3d.prepareImport([new File([obj], 'tri.obj', { type: 'model/obj' }), new File(['newmtl Paint\nKd 1 1 1\nmap_Kd red.png\n'], 'tri.mtl'), new File([bytes(media.png)], 'red.png', { type: 'image/png' })]);
    const model = await PM.assets.add(packed, { silent: true });
    const p = PM.proj;
    const title = PM.mkLayer('text', { name: 'Title', from: 0, dur: 3, d: { text: 'Corpus probe', font: 'Helvetica', size: 40, weight: 700, color: '#FFFFFF' } }, p);
    title.p['position.x'] = PM.P(240, { kf: [PM.KF(0, 200), PM.KF(2, 280)] });
    title.p['position.y'].v = 80;
    const shape = PM.mkLayer('shape', { name: 'Box', from: 0, dur: 3, d: { shape: 'rect', w: 60, h: 40, color: '#33CC66', stroke: 0 } }, p);
    shape.p['position.x'].v = 80; shape.p['position.y'].v = 200;
    const group = PM.mkLayer('group', { name: 'Group', from: 0, dur: 3 }, p);
    const child = PM.mkLayer('shape', { name: 'Child', from: 0, dur: 3, d: { shape: 'ellipse', w: 30, h: 30, color: '#3366CC', stroke: 0 } }, p);
    child.group = group.id;
    child.p['position.x'].v = 400; child.p['position.y'].v = 200;
    const picture = PM.mkLayer('image', { name: 'Picture', from: 0, dur: 3, d: { asset: image.id, w: 16, h: 16 } }, p);
    picture.p['position.x'].v = 30; picture.p['position.y'].v = 30;
    const clip = PM.mkLayer('video', { name: 'Clip', from: 2.5, dur: 0.5, d: { asset: video.id, w: 320, h: 180 } }, p);
    const sound = PM.mkLayer('audio', { name: 'Tone', from: 0, dur: 1, d: { asset: audio.id, gain: 1, trim: 0, fadeIn: 0, fadeOut: 0 } }, p);
    // Two small shader layers in a corner: one compiles, one does not
    // (render.gl.compileError and uiState.getShaderMeta are probed on them).
    const shaderOk = PM.mkLayer('shader', { name: 'Shader ok', from: 0, dur: 3, d: { code: 'void main(){ fragColor = vec4(uv, 0.5, 1.0); }', w: 12, h: 12 } }, p);
    shaderOk.p['position.x'].v = 462; shaderOk.p['position.y'].v = 10;
    const shaderBroken = PM.mkLayer('shader', { name: 'Shader broken', from: 0, dur: 3, d: { code: 'void main(){ fragColor = vec4(probeUndeclared, 1.0); }', w: 12, h: 12 } }, p);
    shaderBroken.p['position.x'].v = 474; shaderBroken.p['position.y'].v = 10;
    p.layers = [title, shape, group, child, picture, clip, sound, shaderOk, shaderBroken];
    PM.ProjectIndex?.invalidate?.();
    const three = null as any;
    PM.touch();
    PM.Projects?.put?.(PM.proj);
    return { text: title.id, shape: shape.id, group: group.id, child: child.id, image: picture.id, video: clip.id, audio: sound.id, three: three?.id, modelAsset: model.id, shaderOk: shaderOk.id, shaderBroken: shaderBroken.id };
  }, media);
}

/** Today's 16 events, each from what causes it in the editor. */
async function triggerEvents(page: Page, fixture: Record<string, string>): Promise<void> {
  await page.evaluate(async (fixture) => {
    const PM = (window as any).PM;
    const api = PM.Kernel.api('corpus');
    const tick = () => new Promise((r) => setTimeout(r, 50));
    api.project.select([fixture.text]);                      // selection
    api.transport.setTime(0.4);                               // time, invalidate, frame:rendered, overlay
    api.transport.play(); await tick(); api.transport.pause(); // transport
    api.project.apply({ type: 'set_property', target: fixture.text, path: 'opacity', value: 100 }); // project:changed
    PM.Fonts?.setSystemFamilies?.(PM.Fonts.system);           // fonts
    PM.bus.emit('layout');                                    // layout (a panel resize)
    const section = api.inspector.registerSection({ id: 'corpus-probe', title: 'Probe', build: () => {} }); // inspector:changed
    section.dispose();
    const theme = api.theme.register({ id: 'corpus-probe-theme', name: 'Probe', scheme: 'dark', tokens: {} });
    const previous = api.theme.active();
    api.theme.activate('corpus-probe-theme');                 // theme:changed
    api.theme.activate(previous);
    theme.dispose();
    if (fixture.three) { api.project.select([fixture.three]); await tick(); }
    api.scene3d.select?.(fixture.three ?? null, fixture.three ? [fixture.three] : []); // scene3d:selection
    await tick();
  }, fixture);
  // extension:unloaded / extension:loaded / extensions:changed: turn an extension off and on.
  await page.evaluate(async () => {
    const api = (window as any).PM.Kernel.api('corpus');
    await api.extensions.setEnabled('events-toggle', false);
  });
  await page.waitForFunction(() => !(window as any).PM.Kernel.loader.activeIds().includes('events-toggle'), undefined, { timeout: 30_000 });
  await page.evaluate(async () => {
    const api = (window as any).PM.Kernel.api('corpus');
    await api.extensions.setEnabled('events-toggle', true);
  });
  await page.waitForFunction(() => (window as any).PM.Kernel.loader.activeIds().includes('events-toggle'), undefined, { timeout: 30_000 });
  await page.evaluate(() => (window as any).PM.setTime(0.5));
  await page.waitForTimeout(800);
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
    await expect.poll(async () => (await inactive()).map((x) => x.id).sort(), { timeout: 60_000 }).toEqual(FAIL_TODAY).catch(async (error) => {
      console.log(JSON.stringify(await inactive()));
      throw error;
    });
    for (const { record } of await inactive()) {
      expect(record).toMatchObject({ state: 'build-error' });
      expect(String((record as any).error)).toContain('Import escapes extension directory: ../../renderer/src/');
    }
    const state = await page.evaluate(({ ids, fail }) => {
      const PM = (window as any).PM;
      const records = PM.Kernel.api('corpus').extensions.list();
      const active: string[] = PM.Kernel.loader.activeIds();
      return {
        replacedStillActive: ids.filter((id) => active.includes(id) && !fail.includes(id)),
        unhealthy: records.filter((r: any) => ids.some((id) => r.id === `${id}-fork` && !fail.includes(id)) && (r.health?.state !== 'ok' || r.enabled === false)).map((r: any) => ({ id: r.id, health: r.health, enabled: r.enabled }))
      };
    }, { ids: [...BUILTINS], fail: FAIL_TODAY });
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
    const after = await page.evaluate(({ ids, fail }) => (window as any).PM.Kernel.api('corpus').extensions.list().filter((r: any) => ids.some((id) => r.id === `${id}-fork` && !fail.includes(id)) && r.health?.state !== 'ok').map((r: any) => r.id), { ids: [...BUILTINS], fail: FAIL_TODAY });
    expect(after).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('the API probe at apiVersion 1, 2 and 3 trusted, and at 3 as a full-access Store install, gets today\'s behavior from every member', async ({ session }) => {
    test.setTimeout(240_000);
    const extensions = path.join(session.userData, 'extensions');
    const provenance: Record<string, unknown> = {};
    // The trusted in-realm context at apiVersion 1, 2 and 3, and a Store
    // install that declares full-access (permissions exist from apiVersion 3
    // on: at 1 and 2 a Store install is sandboxed, the next test).
    for (const [version, store] of [[1, false], [2, false], [3, false], [3, true]] as const) {
      {
        const id = `api-probe-v${version}${store ? '-store' : ''}`;
        const target = path.join(extensions, id);
        await cp(path.join(__dirname, 'fixtures/compat-corpus/api-probe'), target, { recursive: true });
        const manifest = JSON.parse(await readFile(path.join(target, 'manifest.json'), 'utf8'));
        await writeFile(path.join(target, 'manifest.json'), JSON.stringify({ ...manifest, id, apiVersion: version, ...(store ? { permissions: ['full-access'] } : {}) }, null, 2));
        // A Store install someone else published, which the user trusted with full access.
        if (store) provenance[id] = { localId: id, envKey: `repo-${id}`, trusted: true, origin: { repoId: `repo-${id}`, releaseId: 'release-1', coordinate: `someone/${id}`, version: manifest.version, treeSha: 'tree', commitSha: 'commit', ownerPublisherId: 'someone-else' } };
      }
    }
    await writeFile(path.join(session.userData, 'extensions-provenance.json'), JSON.stringify(provenance));
    // An extension to turn off and on (extension:loaded / unloaded / extensions:changed).
    await mkdir(path.join(extensions, 'events-toggle'), { recursive: true });
    await writeFile(path.join(extensions, 'events-toggle/manifest.json'), JSON.stringify({ id: 'events-toggle', name: 'Events toggle', version: '1.0.0', apiVersion: 1, entry: 'index.ts', author: 'user' }));
    await writeFile(path.join(extensions, 'events-toggle/index.ts'), "export default function activate(): void {}\n");
    const IDS = ['api-probe-v1', 'api-probe-v2', 'api-probe-v3', 'api-probe-v3-store'];
    await session.relaunch();
    await session.openEditor();
    const { page } = session;
    await waitActive(page, IDS);
    const trust = await page.evaluate((ids) => (window as any).PM.Kernel.api('corpus').extensions.list().filter((r: any) => ids.includes(r.id)).map((r: any) => [r.id, r.trust, r.health?.state]), IDS);
    console.log(JSON.stringify(trust));
    for (const [id, level, health] of trust) {
      expect(health, id).toBe('ok');
      expect(level, id).toBe(String(id).endsWith('-store') ? 'store-trusted' : 'local');
    }
    const fixture = await probeProject(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
    await triggerEvents(page, fixture);
    const results = await page.evaluate(async ({ ids, fixture }) => {
      const api = (window as any).PM.Kernel.api('corpus');
      const out: Record<string, unknown> = {};
      for (const id of ids) out[id] = await api.commands.run(`${id}.run`, fixture);
      return out;
    }, { ids: IDS, fixture: { ...fixture, helperNames: [...EDITOR_HELPER_EXPORTS] } });
    for (const id of IDS) {
      const probe = results[id] as any;
      console.log(id, JSON.stringify(probe?.events), JSON.stringify(Object.fromEntries(Object.entries(probe?.out ?? {}).filter(([, r]: any) => !r.ok))));
      const version = Number(/v(\d)/.exec(id)![1]);
      expect(probe?.apiVersion, `${id} ran`).toBe(version);
      const failed = Object.entries(probe.out as Record<string, { ok: boolean; detail?: unknown }>).filter(([, r]) => !r.ok).map(([name, r]) => `${name}: ${JSON.stringify(r.detail)}`);
      expect(failed, id).toEqual([]);
      // Every one of today's 16 events reached the extension.
      expect(probe.events, `${id}: events seen`).toEqual([...EVENTS].sort());
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('the API probe in the Store sandbox at apiVersion 1, 2 and 3 keeps the sandbox\'s Promise semantics, with frames from the engine', async ({ session }) => {
    test.setTimeout(180_000);
    const extensions = path.join(session.userData, 'extensions');
    const provenance: Record<string, unknown> = {};
    const IDS = [1, 2, 3].map((v) => `api-probe-sandbox-v${v}`);
    for (const [i, id] of IDS.entries()) {
      const target = path.join(extensions, id);
      await cp(path.join(__dirname, 'fixtures/compat-corpus/api-probe-sandbox'), target, { recursive: true });
      const manifest = JSON.parse(await readFile(path.join(target, 'manifest.json'), 'utf8'));
      // `permissions` exist from apiVersion 3 on; at 1 and 2 a Store install declares none.
      const { permissions, ...rest } = manifest;
      await writeFile(path.join(target, 'manifest.json'), JSON.stringify({ ...rest, id, apiVersion: i + 1, ...(i === 2 ? { permissions: [...(permissions ?? []), 'project:write'] } : {}) }, null, 2));
      provenance[id] = { localId: id, envKey: `repo-${id}`, origin: { repoId: `repo-${id}`, releaseId: 'release-1', coordinate: `someone/${id}`, version: manifest.version, treeSha: 'tree', commitSha: 'commit', ownerPublisherId: 'someone-else' } };
    }
    await writeFile(path.join(session.userData, 'extensions-provenance.json'), JSON.stringify(provenance));
    await session.relaunch();
    await session.openEditor();
    const { page } = session;
    await waitActive(page, IDS, 'command');
    await textProject(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await page.evaluate(() => (window as any).PM.setTime(0.5));
    await page.waitForTimeout(1000);
    for (const [i, id] of IDS.entries()) {
      const probe = await page.evaluate(async (id) => {
        const entry = (window as any).PM.Kernel.commands.get(`${id}.run`);
        return await Promise.race([Promise.resolve(entry.run({ t: 0.5 })), new Promise((resolve) => setTimeout(() => resolve('timeout'), 30_000))]);
      }, id) as any;
      expect(probe, id).not.toBe('timeout');
      console.log(id, JSON.stringify({ seen: probe.seen, failed: Object.entries(probe.out).filter(([, r]: any) => !r.ok) }));
      expect(probe.apiVersion, id).toBe(i + 1);
      const failed = Object.entries(probe.out as Record<string, { ok: boolean; detail?: unknown }>).filter(([, r]) => !r.ok).map(([name, r]) => `${name}: ${JSON.stringify(r.detail)}`);
      expect(failed, id).toEqual([]);
      // The sandbox's snapshot is the engine's frame, as the editor's own capture.
      const same = await page.evaluate(async (url) => {
        const decode = async (u: string) => { const image = new Image(); image.src = u; await image.decode(); const c = document.createElement('canvas'); c.width = image.width; c.height = image.height; c.getContext('2d')!.drawImage(image, 0, 0); return c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data; };
        const editor = (window as any).PM.Kernel.api('corpus').render.snapshot(0.5, 240);
        const [a, b] = [await decode(url), await decode(editor)];
        let max = 0; for (let k = 0; k < a.length; k++) max = Math.max(max, Math.abs(a[k]! - b[k]!));
        return { max, size: a.length === b.length };
      }, probe.snapshotUrl);
      expect(same, `${id}: snapshot`).toEqual({ max: 0, size: true });
      expect(Number(probe.seen['project:changed'] ?? 0), `${id}: events reach the sandbox`).toBeGreaterThan(0);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('the oldest fork shapes the loader accepts work on the engine as on today\'s app: a fork made by the first fork tool, and a replaces-only override', async ({ session }) => {
    test.setTimeout(300_000);
    // The viewer as it shipped when api.extensions.fork first existed (155481d0,
    // viewer@1.0.0), written as that fork tool wrote it; and the shape before
    // the fork tool: a folder that only declares `replaces`.
    const shapes = async (extensions: string, shape: 'fork' | 'replaces'): Promise<string> => {
      const id = shape === 'fork' ? 'viewer-fork' : 'my-viewer';
      const target = path.join(extensions, id);
      await materialize(OLDEST_FORK_COMMIT, 'viewer', target);
      const manifest = JSON.parse(await readFile(path.join(target, 'manifest.json'), 'utf8'));
      expect(manifest.version).toBe('1.0.0');
      if (shape === 'fork') {
        await materialize(OLDEST_FORK_COMMIT, 'viewer', path.join(target, '.forked-from'));
        await writeFile(path.join(target, 'manifest.json'), `${JSON.stringify({ ...manifest, id, name: `${manifest.name} (fork)`, replaces: ['viewer'], forkedFrom: 'viewer@1.0.0', author: 'user' }, null, 2)}\n`);
      } else {
        await writeFile(path.join(target, 'manifest.json'), `${JSON.stringify({ ...manifest, id, name: 'My viewer', replaces: ['viewer'], author: 'user' }, null, 2)}\n`);
      }
      return id;
    };
    const results: Record<string, { engine: unknown; today: unknown }> = {};
    for (const shape of ['fork', 'replaces'] as const) {
      const extensions = path.join(session.userData, 'extensions');
      await rm(extensions, { recursive: true, force: true });
      await mkdir(extensions, { recursive: true });
      const id = await shapes(extensions, shape);
      await session.relaunch();
      await session.openEditor();
      const engine = await viewerShows(session.page, id, true);
      // Today's app, the same folder, "Use New Engine" off.
      const profile = await mkdtemp(path.join(os.tmpdir(), 'powermove-today-viewer-'));
      await shapes(path.join(profile, 'extensions'), shape);
      const off = await launchApp({ userData: profile, env: {} });
      let today: { works: boolean; detail: unknown };
      try {
        await off.openEditor();
        today = await viewerShows(off.page, id, false);
      } finally {
        await off.close();
        await rm(profile, { recursive: true, force: true });
      }
      results[shape] = { engine, today };
      console.log(shape, JSON.stringify(results[shape]));
    }
    for (const shape of ['fork', 'replaces']) {
      const { engine, today } = results[shape] as any;
      // Works on the engine whenever it works on today's app.
      expect(engine.works, `${shape}: engine ${JSON.stringify(engine.detail)} / today ${JSON.stringify(today.detail)}`).toBe(today.works);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('an override by id of every built-in contribution wins on the engine, draws its own programs as today, and gives way when disposed', async ({ session }) => {
    test.setTimeout(240_000);
    await cp(path.join(__dirname, 'fixtures/compat-corpus/override-builtins'), path.join(session.userData, 'extensions/override-builtins'), { recursive: true });
    await session.relaunch();
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => (window as any).PM?.Kernel?.loader?.activeIds?.().includes('override-builtins') && Boolean((window as any).PM?.EngineRender), undefined, { timeout: 60_000 });
    const health = await page.evaluate(() => (window as any).PM.Kernel.api('corpus').extensions.list().find((r: any) => r.id === 'override-builtins')?.health?.state);
    expect(health).toBe('ok');
    const state = await page.evaluate(() => (window as any).PM.Kernel.api('corpus').commands.run('override-builtins.state'));
    console.log(JSON.stringify(Object.fromEntries(Object.entries(state.overridden).map(([k, v]: any) => [k, v.length]))));
    // Every kind of built-in contribution has an override by id.
    for (const kind of ['effects', 'transitions', 'commands', 'panels', 'keybindings', 'themes', 'layers', 'inspector', 'palette', 'services']) {
      expect(state.overridden[kind]?.length ?? 0, kind).toBeGreaterThan(0);
    }
    // Each override is the one the registries resolve to.
    const winners = await page.evaluate((overridden) => {
      const K = (window as any).PM.Kernel;
      const registry: Record<string, any> = { effects: K.effects, transitions: K.transitions, commands: K.commands, panels: K.panels, keybindings: K.keybindings, themes: K.themes, layers: K.layerTypes, inspector: K.inspectorSections };
      const losing: string[] = [];
      for (const [kind, ids] of Object.entries(overridden as Record<string, string[]>)) {
        if (!registry[kind]) continue;
        for (const id of ids) if (registry[kind].topEntry(id)?.ownerId !== 'override-builtins') losing.push(`${kind}:${id}`);
      }
      return losing;
    }, state.overridden);
    expect(winners).toEqual([]);
    // A built-in command id runs the override's command.
    await page.evaluate(() => (window as any).PM.Kernel.api('corpus').commands.run('timeline.revealAll'));
    expect((await page.evaluate(() => (window as any).PM.Kernel.api('corpus').commands.run('override-builtins.state'))).hits['timeline.revealAll']).toBe(1);
    // Effects and transitions: the engine draws the override's program, as today's compositor does.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Overrides', w: 160, h: 90, fps: 30, dur: 2, bg: '#203040' });
      project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
      const layer = PM.mkLayer('solid', { name: 'Colorful', from: 0, dur: 2, d: { color: '#E83020', w: 120, h: 60 } }, project);
      layer.id = 'colorful';
      project.layers = [layer];
      PM.replaceProject(project);
      PM.Projects?.put?.(PM.proj);
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    const drawn = await page.evaluate(async (overridden) => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('corpus');
      const layer = PM.proj.layers[0];
      const px = (c: HTMLCanvasElement) => c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
      const cmp = (a: Uint8ClampedArray, b: Uint8ClampedArray) => { let max = 0, over1 = 0; for (let i = 0; i < a.length; i++) { if (i % 4 === 3) continue; const d = Math.abs(a[i]! - b[i]!); if (d > max) max = d; if (d > 1) over1++; } return { max, over1 }; };
      const frames = (set: () => void, t: number) => { set(); PM.touch(); const engine = px(api.render.renderFrameTo(t, 160, 90, { mblur: false })); const today = px(PM.renderFrameTo(t, 160, 90, { mblur: false })); return { engine, today }; };
      const rows: any[] = [];
      const plain = frames(() => { layer.fx = []; layer.transitionIn = null; }, 1);
      for (const id of overridden.effects as string[]) {
        const fx = PM.mkEffect(id);
        const { engine, today } = frames(() => { layer.fx = [fx]; layer.transitionIn = null; }, 1);
        rows.push({ kind: 'effect', id, vsToday: cmp(engine, today), vsPlain: cmp(engine, plain.engine), todayVsPlain: cmp(today, plain.today), fx: JSON.parse(JSON.stringify(fx)), engine });
      }
      for (const id of overridden.transitions as string[]) {
        const { engine, today } = frames(() => { layer.fx = []; layer.transitionIn = { type: id, dur: 1, p: {} }; }, 0.5);
        rows.push({ kind: 'transition', id, vsToday: cmp(engine, today), engine });
      }
      // The built-ins' own programs, after disposing the overrides.
      await api.commands.run('override-builtins.toggle');
      for (const row of rows) {
        const { engine } = frames(() => { if (row.kind === 'effect') { layer.fx = [PM.mkEffect(row.id)]; layer.transitionIn = null; } else { layer.fx = []; layer.transitionIn = { type: row.id, dur: 1, p: {} }; } }, row.kind === 'effect' ? 1 : 0.5);
        row.vsBuiltin = cmp(row.engine, engine);
        delete row.engine;
      }
      const K = PM.Kernel;
      const restored = (overridden.effects as string[]).every((id) => K.effects.topEntry(id)?.ownerId === 'effects-basic');
      await api.commands.run('override-builtins.toggle');
      return { rows, restored };
    }, state.overridden);
    console.log(JSON.stringify(drawn.rows));
    expect(drawn.restored).toBe(true);
    const total = drawn.rows.length;
    expect(total).toBe(state.overridden.effects.length + state.overridden.transitions.length);
    for (const row of drawn.rows) {
      expect(row.vsToday.max, `${row.kind} ${row.id}: engine vs today, with the override`).toBeLessThanOrEqual(4);
      expect(row.vsToday.over1, `${row.kind} ${row.id}: engine vs today, with the override`).toBeLessThanOrEqual(160 * 90 * 3 * 0.001);
    }
    // The override's program drew (its output marked green), not the built-in's.
    for (const row of drawn.rows) expect(row.vsBuiltin.max, `${row.kind} ${row.id}: the override's program drew`).toBeGreaterThan(8);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
