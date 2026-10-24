import { chooseNativeMenu, expect, test } from './helpers/app';
import type { Page } from '@playwright/test';

async function disclosurePoint(page: Page, name: string) {
  await page.waitForFunction(name => (window as any).PM.Kernel.services.get('timeline').rows.some((row: any) => row.L.name === name), name);
  await page.evaluate(name => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    const index = timeline.rows.findIndex((row: any) => row.kind === 'layer' && row.L.name === name);
    const box = timeline.cv.getBoundingClientRect();
    timeline.scrollY = Math.max(0, index * timeline.row - (box.height - timeline.ruler) / 2);
    PM.invalidate('timeline');
  }, name);
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  return page.evaluate(name => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const index = timeline.rows.findIndex((row: any) => row.kind === 'layer' && row.L.name === name);
    const row = timeline.rows[index], box = timeline.cv.getBoundingClientRect();
    return { x: box.x + 64 + (row.depth || 0) * 12, y: box.y + timeline.ruler + index * timeline.row - timeline.scrollY + timeline.row / 2 };
  }, name);
}

test('compact layer rows keep names centered and parenting in dedicated controls',async({session})=>{
  await session.openEditor();
  const {page}=session;
  await page.waitForFunction(()=>{ const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  await page.evaluate(()=>{
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const p=PM.mkProject({w:640,h:360,dur:5});
    const names=['Info glyph','Info circle','Info button','Keyboard shortcut','Search placeholder','Search handle','Search lens','Search outer edge'];
    p.layers=names.map((name,index)=>PM.mkLayer([0,3,4].includes(index)?'text':'shape',{name},p));
    const outer=PM.mkLayer('group',{name:'Terminal'},p),inner=PM.mkLayer('group',{name:'Nested'},p),child=PM.mkLayer('text',{name:'Prompt'},p);
    inner.group=outer.id;child.group=inner.id;p.layers.push(outer,inner,child);
    PM.replaceProject(p);PM.theme.apply('dark');PM.selectLayers([p.layers[3].id,p.layers[7].id]);timeline.scrollY=0;PM.invalidate();
  });
  await page.waitForFunction(()=>{ const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.length===9; });
  const rows=()=>page.evaluate(()=>{ const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); const r=timeline.cv.getBoundingClientRect();return {x:r.left,y:r.top,gut:timeline.gut,row:timeline.row,ruler:timeline.ruler,height:r.height};});
  const box=await rows();
  await page.screenshot({path:'/tmp/powermove-timeline-clean-rows.png',clip:{x:box.x,y:box.y,width:box.gut,height:Math.min(box.height,box.ruler+9*box.row)}});
  // The lower half of a name is a layer selection target, not a hidden parent menu.
  await page.mouse.click(box.x+110,box.y+box.ruler+box.row/2+9);
  expect(await page.evaluate(()=>(window as any).PM.firstSel().name)).toBe('Info glyph');
  await expect(page.getByRole('menu')).toHaveCount(0);
  await chooseNativeMenu(session, 'Info circle', () => page.mouse.click(box.x+box.gut-12,box.y+box.ruler+box.row/2));
  expect(await page.evaluate(()=>{const PM=(window as any).PM;return PM.L(PM.firstSel().parent).name;})).toBe('Info circle');
  await page.mouse.move(box.x+box.gut-30,box.y+box.ruler+box.row/2);await page.mouse.down();
  await page.mouse.move(box.x+110,box.y+box.ruler+box.row*2.5,{steps:8});await page.mouse.up();
  expect(await page.evaluate(()=>{const PM=(window as any).PM;return PM.L(PM.firstSel().parent).name;})).toBe('Info button');
  // Nested disclosure buttons follow the indentation of their badges and names.
  const outer = await disclosurePoint(page, 'Terminal');
  await page.mouse.click(outer.x, outer.y);
  await page.waitForFunction(()=>{ const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.some((r:any)=>r.L.name==='Nested'); });
  const inner = await disclosurePoint(page, 'Nested');
  await page.mouse.click(inner.x, inner.y);
  await page.waitForFunction(()=>{ const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.some((r:any)=>r.L.name==='Prompt'); });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
