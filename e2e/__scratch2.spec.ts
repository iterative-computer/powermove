import { test } from './helpers/app';
test('drag probe 2', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.WS.mutate((w: any) => { const c = w.layout.docks.find((d: any) => d.id === 'center'); if (!c.panels.some((p: any) => p.id === 'shader')) c.panels.push({ id: 'shader', size: 320 }); });
    (window as any).__log = [];
    (window as any).__calls = { activate: 0, mutate: 0, apply: 0 };
    const A = PM.WS.activate.bind(PM.WS); PM.WS.activate = (...a: any[]) => { (window as any).__calls.activate++; return A(...a); };
    (window as any).__mutStacks = [];
    const M = PM.WS.mutate.bind(PM.WS); PM.WS.mutate = (...a: any[]) => { if (((window as any).__mutStacks as string[]).length < 6) (window as any).__mutStacks.push((new Error().stack || '').split('\n').slice(1, 10).join(' <- ').replace(/app:\/\/powermove\/assets\/index-[^:]*/g, 'B')); (window as any).__calls.mutate++; return M(...a); };
    const AP = PM.Layout.apply.bind(PM.Layout); PM.Layout.apply = (...a: any[]) => { (window as any).__calls.apply++; return AP(...a); };
    const orig = PM.drag;
    PM.drag = (e: any, opts: any) => { (window as any).__log.push('drag-start'); return orig(e, { ...opts, move: (dx: number, dy: number, ev: any) => { (window as any).__log.push(['move', dy]); return opts.move(dx, dy, ev); }, up: (...a: any[]) => { (window as any).__log.push('up'); return opts.up?.(...a); }, cancel: (...a: any[]) => { (window as any).__log.push('cancel'); return opts.cancel?.(...a); } }); };
  });
  await page.waitForSelector('#panel-shader');
  const sp = page.locator('#dock-center .splitter.h').nth(1);
  const box = (await sp.boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  await page.mouse.move(x, y); await page.mouse.down();
  await page.mouse.move(x, y - 120, { steps: 3 });
  const before = await page.evaluate(() => {
    const PM = (window as any).PM;
    const instSpec = PM.panelInst['shader'].spec;
    const wsSpec = PM.WS.current.layout.docks.find((d: any) => d.id === 'center').panels.find((p: any) => p.id === 'shader');
    return { center: PM.WS.current.layout.docks.find((d: any) => d.id === 'center').panels.map((x: any) => [x.id, x.size, x.flex, x.collapsed]), same: instSpec === wsSpec, instSize: instSpec?.size, wsSize: wsSpec?.size,
      stacks: (window as any).__mutStacks.slice(0, 2), t: document.getElementById('panel-timeline')?.getBoundingClientRect().height, s: document.getElementById('panel-shader')?.getBoundingClientRect().height };
  });
  console.log('PRE-UP ' + JSON.stringify(before));
  await page.mouse.up();
  const justAfter = await page.evaluate(() => ({ t: document.getElementById('panel-timeline')?.getBoundingClientRect().height, s: document.getElementById('panel-shader')?.getBoundingClientRect().height }));
  console.log('POST-UP ' + JSON.stringify(justAfter));
  await page.waitForTimeout(200);
  const out = await page.evaluate(() => ({
    log: (window as any).__log,
    instT: Boolean((window as any).PM.panelInst['timeline']?.el?.isConnected),
    instS: Boolean((window as any).PM.panelInst['shader']?.el?.isConnected),
    t: document.getElementById('panel-timeline')?.getBoundingClientRect().height,
    s: document.getElementById('panel-shader')?.getBoundingClientRect().height,
    splitters: document.querySelectorAll('#dock-center .splitter.h').length
  }));
  console.log('OUT ' + JSON.stringify(out));
});
