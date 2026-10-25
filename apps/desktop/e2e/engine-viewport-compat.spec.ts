import { cp, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

/** The view, navigation and gizmo contract in the v1.2.0 release tag. */
const MEMBERS = [
  'scene.getView', 'scene.setView', 'scene.onViewChange',
  'scene.getNavigationMode', 'scene.setNavigationMode', 'scene.onNavigationChange', 'scene.createNavigation', 'scene.frame',
  'scene.getMode', 'scene.setMode', 'scene.getSpace', 'scene.setSpace', 'scene.onGizmoChange', 'scene.createGizmo',
  'navigation.update', 'navigation.pointerDown', 'navigation.wheel', 'navigation.frame', 'navigation.active', 'navigation.cancel', 'navigation.dispose',
  'gizmo.update', 'gizmo.hover', 'gizmo.pointerDown', 'gizmo.active', 'gizmo.covers', 'gizmo.cancel', 'gizmo.dispose'
] as const;

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });
for (const [version, store] of [[1, false], [2, false], [3, false], [3, true]] as const) {
  test(`compiled released 3D view/navigation/gizmo probes, apiVersion ${version}${store ? ' full-access Store' : ' local'}`, async ({ session }) => {
    test.setTimeout(180_000);
    const provenance: Record<string, unknown> = {};
    const probes: Array<{ id: string; member: string }> = [];
    for (const member of MEMBERS) {
      const id = `vp-${member.replaceAll('.', '-').toLowerCase()}-v${version}${store ? '-store' : ''}`;
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
      await session.page.evaluate((member) => {
        const PM = (window as any).PM, api = PM.Kernel.api('corpus');
        PM.replaceProject(PM.mkProject({ name: 'Viewport corpus', w: 480, h: 270, fps: 30, dur: 3 }));
        api.scene3d.edit({ operation: 'add_object', object: { id: 'probe-cube', source: { primitive: 'box' } } });
        api.scene3d.edit({ operation: 'add_light', light: { id: 'probe-light', type: 'point', p: { x: -2, y: 3, z: 2 } } });
        api.scene3d.select(null, ['probe-cube']);
        api.scene3d.setView('editor');
      }, probe.member);
      if (probe.member === 'scene.frame' || probe.member === 'navigation.frame') {
        await session.page.waitForFunction(() => typeof (window as any).PM?.EngineDoc?.api?.objectBoxes3d === 'function', undefined, { timeout: 20_000 });
      }
      const result = await session.page.evaluate(async ({ id, member }) => {
        const PM = (window as any).PM, api = PM.Kernel.api('corpus');
        const result = await api.commands.run(`${id}.run`, member);
        const record = api.extensions.list().find((r: any) => r.id === id);
        return { ...result, health: record.health.state, enabled: record.enabled, trust: record.trust };
      }, probe);
      results.push({ ...probe, ...result });
    }
    console.log(JSON.stringify({ version, store, released3DMembers: results }));
    expect(results.filter((r) => !r.ok || r.health !== 'ok' || !r.enabled || r.apiVersion !== version || r.trust !== (store ? 'store-trusted' : 'local'))).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
    // The probe owns disposable documents and creates many 3D controllers;
    // exit the isolated process after assertions rather than exercise Save
    // decisions during fixture cleanup.
    session.app.process().kill('SIGKILL');
  });
}
