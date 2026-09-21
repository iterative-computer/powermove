import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('a real mod supplies import defaults and Properties controls with automatic cleanup', async ({ session }) => {
  const dir = path.join(session.userData, 'extensions', 'import-anchor-presets');
  await mkdir(dir, { recursive: true });
  await writeFile(path.join(dir, 'manifest.json'), JSON.stringify({ id: 'import-anchor-presets', name: 'Import anchor presets', version: '1.0.0', apiVersion: 1, contributes: ['media', 'inspector'] }));
  await writeFile(path.join(dir, 'index.ts'), `import type { PowermoveAPI } from 'powermove';
export default function activate(api: PowermoveAPI) {
  api.media.registerImportDefaults({ anchor: { x: 0.5, y: 0.5 } });
  api.inspector.registerSection({ id: 'anchor-presets', title: 'Anchor presets', after: 'transform',
    build(target, { layerIds }) {
      const button = document.createElement('button');
      button.textContent = 'Apply anchor preset';
      button.onclick = () => api.project.apply(layerIds.map(target => ({type:'set_property',target,path:'anchor.x',value:12,preserveHandEdits:false})), {label:'Anchor preset'});
      target.append(button);
      return () => button.remove();
    }
  });
}`);
  await session.relaunch();
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => (window as any).PM.Kernel.api('mod-probe').media.getImportDefaults()?.anchor.x === 0.5);
  const imported = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const old = PM.mkLayer('text');
    old.p['anchor.x'].v = 70;
    old.p['anchor.x'].kf = PM.normalizeKeyframes([{ i:'old-key',t:0,v:90,e:'linear' }], 70, PM.proj.fps);
    PM.proj.layers.push(old); PM.ProjectIndex.invalidate();
    const before = JSON.parse(JSON.stringify(old.p));
    await PM.importFiles([new File(['<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect x="10" y="20" width="20" height="40" fill="red"/></svg>'], 'asymmetric.svg', { type:'image/svg+xml' })]);
    const layer = PM.proj.layers.find((l: any) => l.id !== old.id);
    const bounds = PM.GL.bounds(layer, 0);
    (window as any).__importedModLayer = layer.id;
    return { oldBefore: before, oldAfter: JSON.parse(JSON.stringify(PM.L(old.id).p)), id:layer.id,
      anchor:[layer.p['anchor.x'].v, layer.p['anchor.y'].v],
      center:[(bounds.x0+bounds.x1)/2,(bounds.y0+bounds.y1)/2],
      expectedPosition:[PM.proj.w/2+(bounds.x0+bounds.x1)/2,PM.proj.h/2+(bounds.y0+bounds.y1)/2],
      position:[layer.p['position.x'].v,layer.p['position.y'].v] };
  });
  expect(imported.oldAfter).toEqual(imported.oldBefore);
  expect(imported.anchor).toEqual(imported.center);
  expect(imported.position).toEqual(imported.expectedPosition);
  const section = page.locator('[data-inspector-section="anchor-presets"]');
  await expect(section.getByRole('button', { name: 'Apply anchor preset' })).toBeVisible();
  await section.getByRole('button').click();
  expect(await page.evaluate(id => (window as any).PM.L(id).p['anchor.x'].v, imported.id)).toBe(12);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await page.evaluate(id => (window as any).PM.L(id).p['anchor.x'].v, imported.id)).toBe(imported.center[0]);
  await page.evaluate(async () => { await (window as any).powermove.extensions.setEnabled({ id:'import-anchor-presets', enabled:false }); });
  await expect(section).toHaveCount(0);
  await page.waitForFunction(() => (window as any).PM.Kernel.api('mod-probe').media.getImportDefaults() === null);
  expect(await page.evaluate(id => (window as any).PM.L(id).p['anchor.x'].v, imported.id)).toBe(imported.center[0]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
