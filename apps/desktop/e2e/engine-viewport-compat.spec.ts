import { cp, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

const MEMBERS = ['attach', 'state', 'onChange', 'inContext', 'operators', 'run',
  'update', 'pointerDown', 'objectPress', 'hover', 'cursorAt', 'wheel', 'contextMenu',
  'startTransform', 'armBoxSelect', 'armZoomBorder', 'startWalk', 'extrude', 'takeBoxSelect',
  'modalActive', 'pointer', 'covers', 'glyphsIn', 'active', 'cancel', 'dispose'];

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });
for (const [version, store] of [[1, false], [2, false], [3, false], [3, true]] as const) {
  test(`compiled per-member scene3d viewport probes, apiVersion ${version}${store ? ' full-access Store' : ' local'}`, async ({ session }) => {
    test.setTimeout(180_000);
    const apiSource = await readFile(path.join(__dirname, '../src/renderer/src/kernel/api.ts'), 'utf8');
    const declared = ['Scene3DViewportAPI', 'Scene3DViewportController'].flatMap((name) =>
      [...apiSource.split(`export interface ${name} {`)[1]!.split('\n}')[0]!.matchAll(/^  (?:readonly )?(\w+)(?:\(|:)/gm)].map((m) => m[1]!));
    expect([...MEMBERS].sort()).toEqual(declared.sort());
    const provenance: Record<string, unknown> = {};
    const probes: Array<{ id: string; member: string }> = [];
    for (const member of MEMBERS) {
      const id = `vp-${member.toLowerCase()}-v${version}${store ? '-store' : ''}`;
      const target = path.join(session.userData, 'extensions', id);
      await cp(path.join(__dirname, 'fixtures/compat-corpus/viewport-probe'), target, { recursive: true });
      await writeFile(path.join(target, 'manifest.json'), JSON.stringify({ id, name: `${member} probe`, version: '1.0.0', apiVersion: version, entry: 'index.ts', author: 'user', ...(store ? { permissions: ['full-access'] } : {}) }));
      if (store) provenance[id] = { localId: id, envKey: `repo-${id}`, trusted: true, origin: { repoId: `repo-${id}`, releaseId: 'release-1', coordinate: `someone/${id}`, version: '1.0.0', treeSha: 'tree', commitSha: 'commit', ownerPublisherId: 'someone-else' } };
      probes.push({ id, member });
    }
    await writeFile(path.join(session.userData, 'extensions-provenance.json'), JSON.stringify(provenance));
    await session.relaunch(); await session.openEditor();
    await session.page.waitForFunction((ids) => ids.every((id) => (window as any).PM?.Kernel?.loader?.activeIds?.().includes(id)), probes.map((p) => p.id), { timeout: 60_000 });
    await session.page.waitForFunction(() => !!(window as any).PM.EngineRender);
    const results = [];
    for (const probe of probes) {
      await session.page.evaluate(() => {
        const PM = (window as any).PM, api = PM.Kernel.api('corpus');
        PM.replaceProject(PM.mkProject({ name: 'Viewport corpus', w: 480, h: 270, fps: 30, dur: 3 }));
        api.scene3d.edit({ operation: 'add_object', object: { id: 'probe-cube', source: { primitive: 'box' } } });
        api.scene3d.edit({ operation: 'add_light', light: { id: 'probe-light', type: 'point', p: { x: -2, y: 3, z: 2 } } });
        api.scene3d.setView('editor');
      });
      const result = await session.page.evaluate(async ({ id, member }) => {
        const PM = (window as any).PM, api = PM.Kernel.api('corpus');
        const result = await api.commands.run(`${id}.run`, member);
        const record = api.extensions.list().find((r: any) => r.id === id);
        return { ...result, health: record.health.state, enabled: record.enabled, trust: record.trust };
      }, probe);
      results.push({ ...probe, ...result });
    }
    console.log(JSON.stringify({ version, store, viewportMembers: results }));
    expect(results.filter((r) => !r.ok || r.health !== 'ok' || !r.enabled || r.apiVersion !== version || r.trust !== (store ? 'store-trusted' : 'local'))).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
    // The probe owns disposable documents and creates many 3D viewports;
    // exit the isolated process after assertions rather than exercise Save
    // decisions during fixture cleanup.
    session.app.process().kill('SIGKILL');
  });
}
