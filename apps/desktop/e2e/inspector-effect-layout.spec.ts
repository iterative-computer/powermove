import {test,expect} from './helpers/app';

for(const panelId of ['inspector','layer-effects'])for(const theme of ['light','dark'])test(`effect controls stay inside their rows without overlapping (${panelId}, ${theme})`,async({session},testInfo)=>{
  await session.openEditor();
  await session.app.evaluate(({BrowserWindow})=>{BrowserWindow.getAllWindows()[0]?.setContentSize(1400,1200);});
  const {page}=session;
  const ids=await page.evaluate(theme=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();PM.theme.apply(theme);
    PM.replaceProject(PM.mkProject({name:'Effect control layout',w:640,h:360,dur:3}));
    const layer=PM.mkLayer('solid',{name:'Effect layout',d:{w:320,h:180,color:'#FF6600'}});
    layer.fx=['gradient','blur','tint'].map(type=>PM.mkEffect(type,undefined,layer));
    for(const effect of layer.fx)PM.UIState.setFxOpen(effect,true);
    PM.proj.layers=[layer];PM.selectLayers(layer.id);PM.touch();PM.bus.emit('layers');PM.invalidate();
    return layer.fx.map((effect:any)=>effect.id);
  },theme);
  if(panelId==='layer-effects')await page.evaluate(()=>{(window as any).PM.Kernel.api('effect-layout-test').panels.open('layer-effects');});
  const inspector=page.locator(`#panel-${panelId}`);
  await expect(inspector.locator(`[data-effect-id="${ids[0]}"]`)).toBeVisible();
  for(const width of [300,380,480]){
    await inspector.evaluate((panel,width)=>{(panel.closest('.dock') as HTMLElement).style.flex=`0 0 ${width}px`;},width);
    await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    expect(await inspector.evaluate(panel=>Math.round(panel.getBoundingClientRect().width))).toBe(width);
    const layout=await inspector.locator('.fx-params .row.split').evaluateAll(rows=>rows.map(row=>{
      const rect=row.getBoundingClientRect(),field=row.querySelector(':scope > .vwrap')!;
      const fieldRect=field.getBoundingClientRect();
      const next=row.nextElementSibling?.getBoundingClientRect();
      const controls=[...field.querySelectorAll('.point-pad,.well,.visual-track,.pm-select,.color-field,.chip')];
      return {label:row.querySelector(':scope > .k')?.textContent,height:rect.height,point:!!row.querySelector('.point-field'),
        fieldInside:fieldRect.top>=rect.top-.5 && fieldRect.bottom<=rect.bottom+.5,
        controlsInside:controls.every(control=>{const r=control.getBoundingClientRect();return r.top>=rect.top-.5 && r.bottom<=rect.bottom+.5;}),
        nextInside:!next || next.top>=rect.bottom-.5};
    }));
    expect(layout.filter(row=>row.point)).toHaveLength(2);
    for(const row of layout){
      expect(row.fieldInside,`${width}px ${row.label}: value wrapper`).toBe(true);
      expect(row.controlsInside,`${width}px ${row.label}: controls`).toBe(true);
      expect(row.nextInside,`${width}px ${row.label}: following row`).toBe(true);
      expect(row.height).toBeGreaterThanOrEqual(32);
    }
    const gradient=inspector.locator('.fx-params').first();
    await gradient.scrollIntoViewIfNeeded();
    await gradient.screenshot({path:testInfo.outputPath(`effects-${theme}-${width}.png`)});
  }
  // The taller controls remain interactive, including the shared X/Y edits.
  const firstPoint=inspector.locator('.point-field').first();
  await firstPoint.getByRole('button',{name:/Start Of Ramp:.*Use arrow keys/}).focus();
  const before=await page.evaluate(id=>(window as any).PM.proj.layers[0].fx.find((effect:any)=>effect.id===id).p.startX.v,ids[0]);
  await page.keyboard.press('ArrowRight');
  await expect.poll(()=>page.evaluate(id=>(window as any).PM.proj.layers[0].fx.find((effect:any)=>effect.id===id).p.startX.v,ids[0])).toBeGreaterThan(before);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
