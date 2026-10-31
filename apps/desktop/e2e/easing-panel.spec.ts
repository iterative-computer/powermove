import { expect, test, chooseNativeMenu } from './helpers/app';

for (const theme of ['light','dark']) {
  test(`Easing panel edits standard and multi-keyframe curves, library and graph in ${theme}`, async ({session}) => {
    await session.openEditor();
    const id = await session.page.evaluate(theme => {
      const PM=(window as any).PM; PM.theme.apply(theme);
      const layer=PM.mkLayer('solid',{name:'Easing verification',from:0,dur:5});PM.proj.layers=[layer];PM.ProjectIndex.invalidate();
      PM.animate(layer,'position.x',[{t:0,v:0},{t:2,v:200}]);PM.selectLayers(layer.id);
      PM.sel.keys=layer.p['position.x'].kf.map((k:any)=>k.i);PM.bus.emit('sel');PM.touch();PM.hist.clear();
      PM.WS.mutate((ws:any)=>PM.Layout.addPanel(ws,'easing','right'));return layer.id;
    },theme);
    const panel=session.page.locator('#panel-easing');await expect(panel).toBeVisible();
    await expect(panel.getByRole('button',{name:/^Apply /})).toHaveCount(25);
    const keys=()=>session.page.evaluate(id=>(window as any).PM.L(id).p['position.x'].kf,id);
    await panel.getByRole('button',{name:'Apply Ease 80',exact:true}).click();
    expect((await keys())[0].outEase.influence).toBeCloseTo(80);
    await panel.getByRole('button',{name:'Apply Overshoot',exact:true}).click();
    expect(await keys()).toHaveLength(4);expect((await keys()).some((k:any)=>k.v>200)).toBe(true);
    await panel.getByRole('button',{name:'Apply Bounce 2',exact:true}).click();
    expect(await keys()).toHaveLength(7);
    await session.page.evaluate(()=>(window as any).PM.hist.undo());expect(await keys()).toHaveLength(4);
    await session.page.evaluate(()=>{const PM=(window as any).PM;PM.sel.keys=PM.L(PM.sel.layers[0]).p['position.x'].kf.map((k:any)=>k.i);PM.bus.emit('sel');});
    await panel.getByRole('button',{name:'Copy keyframes',exact:true}).click();
    await expect(panel.getByRole('button',{name:'Paste easing',exact:true})).toBeEnabled();
    await panel.getByRole('button',{name:'Input coordinates',exact:true}).click();
    await expect(panel.getByRole('spinbutton',{name:'out handle X'})).toBeVisible();
    await panel.getByRole('button',{name:'Save easing preset',exact:true}).click();
    const dialog=session.page.getByRole('dialog');await expect(dialog).toBeVisible();
    await dialog.getByRole('textbox',{name:'Save easing preset'}).fill('My easing');await dialog.getByRole('button',{name:'Save',exact:true}).click();
    await expect(panel.getByRole('button',{name:'Apply My easing',exact:true})).toBeVisible();
    await session.page.evaluate(() => { const PM=(window as any).PM; PM.WS.mutate((ws:any) => { const right=ws.layout.docks.find((d:any)=>d.id==='right'); if(right) right.panels=right.panels.filter((p:any)=>p.id==='easing'); }); });
    await panel.screenshot({path:`/tmp/powermove-easing-panel-${theme}.png`});
    await session.page.screenshot({path:`/tmp/powermove-easing-${theme}.png`});
    await panel.getByRole('button',{name:'New preset group',exact:true}).click();
    await dialog.getByRole('textbox',{name:'New preset group'}).fill('Favorites');await dialog.getByRole('button',{name:'Save',exact:true}).click();
    await expect(panel.getByRole('tab',{name:'Favorites'})).toBeVisible();
    await chooseNativeMenu(session,'Panel Settings',()=>panel.getByRole('button',{name:'More easing tools'}).click());
    await expect(panel.getByText('Layout',{exact:true})).toBeVisible();
    const overflow=await panel.evaluate(el=>({scroll:el.scrollWidth,client:el.clientWidth}));expect(overflow.scroll).toBeLessThanOrEqual(overflow.client+1);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}

test('graph editing auto-applies as one undoable gesture and imports extended presets',async({session})=>{
  await session.openEditor();
  const id=await session.page.evaluate(()=>{
    const PM=(window as any).PM,layer=PM.mkLayer('solid',{name:'Graph edit',from:0,dur:4});PM.proj.layers=[layer];PM.ProjectIndex.invalidate();
    PM.animate(layer,'position.x',[{t:0,v:0},{t:2,v:100}]);PM.selectLayers(layer.id);PM.sel.keys=layer.p['position.x'].kf.map((k:any)=>k.i);PM.bus.emit('sel');PM.hist.clear();
    PM.WS.mutate((ws:any)=>{PM.Layout.addPanel(ws,'easing','right'); const right=ws.layout.docks.find((d:any)=>d.id==='right');right.panels=right.panels.filter((p:any)=>p.id==='easing');});return layer.id;
  });
  const panel=session.page.locator('#panel-easing');await expect(panel).toBeVisible();
  await panel.getByRole('button',{name:'Auto apply',exact:true}).click();
  const graph=panel.locator('.easing-graph'),box=(await graph.boundingBox())!;
  const first=await graph.locator('circle.hit').first().boundingBox();expect(first).toBeTruthy();
  await session.page.mouse.move(first!.x+first!.width/2,first!.y+first!.height/2);await session.page.mouse.down();
  await session.page.mouse.move(box.x+box.width*.38,box.y+box.height*.7,{steps:6});await session.page.mouse.up();
  const history=await session.page.evaluate(()=>{const PM=(window as any).PM;return {count:PM.hist.list().length,keys:PM.L(PM.sel.layers[0]).p['position.x'].kf};});
  expect(history.count).toBe(1);expect(history.keys[0].outInterp).toBe('bezier');
  await session.page.evaluate(()=>(window as any).PM.hist.undo());
  expect(await session.page.evaluate(id=>(window as any).PM.L(id).p['position.x'].kf[0].outInterp,id)).toBe('linear');
  const external={name:'Imported spring',type:'extended',color:'#FFF',presetData:[{time:0,value:0},{time:.5,value:1.3},{time:1,value:1}].map(k=>({...k,intTypeIn:6613,intTypeOut:6613,tempEaseIn:[{speed:0,influence:33}],tempEaseOut:[{speed:0,influence:33}]}))};
  await panel.locator('input[type=file]').first().setInputFiles({name:'Imported.kease',mimeType:'application/json',buffer:Buffer.from(JSON.stringify(external))});
  await expect(panel.getByRole('button',{name:'Apply Imported spring',exact:true})).toBeVisible();
  await panel.getByRole('button',{name:'Apply Imported spring',exact:true}).click();
  const keys=await session.page.evaluate(id=>(window as any).PM.L(id).p['position.x'].kf,id);expect(keys).toHaveLength(3);expect(keys[1].v).toBeCloseTo(130);
  await chooseNativeMenu(session,'Panel Settings',()=>panel.getByRole('button',{name:'More easing tools'}).click());
  await panel.getByRole('combobox',{name:'Layout',exact:true}).click();
  await session.page.getByRole('option',{name:'Toggle Views',exact:true}).click();
  await expect(panel.getByRole('button',{name:'Toggle graph and library'})).toBeVisible();
  await panel.getByRole('button',{name:'Toggle graph and library'}).click();await expect(graph).toHaveCount(0);
  await expect(panel.getByRole('button',{name:'Apply Imported spring',exact:true})).toBeVisible();
  const saved=await session.page.evaluate(()=>JSON.parse(localStorage.getItem('pm.easing.library.v1')!));expect(saved.preferences.view).toBe('toggle');expect(saved.groups.at(-1).presets[0].name).toBe('Imported spring');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('vector presets and curved path cleanup work through the native panel and playback',async({session})=>{
  await session.openEditor();
  const id=await session.page.evaluate(()=>{
    const PM=(window as any).PM,layer=PM.mkLayer('solid',{name:'Curved route',from:0,dur:4});PM.proj.layers=[layer];PM.ProjectIndex.invalidate();
    PM.animate(layer,'position.x',[{t:0,v:0},{t:2,v:200}]);PM.animate(layer,'position.y',[{t:0,v:0},{t:2,v:200}]);
    PM.Edit.apply({type:'set_easing',keyframes:layer.p['position.y'].kf.map((k:any)=>k.i),curve:[1/3,0,2/3,1/3]});
    PM.selectLayers(layer.id);PM.sel.keys=layer.p['position.x'].kf.map((k:any)=>k.i);PM.bus.emit('sel');PM.hist.clear();
    PM.WS.mutate((ws:any)=>{PM.Layout.addPanel(ws,'easing','right');const right=ws.layout.docks.find((d:any)=>d.id==='right');right.panels=right.panels.filter((p:any)=>p.id==='easing');});return layer.id;
  });
  const panel=session.page.locator('#panel-easing');await expect(panel).toBeVisible();
  const before=await session.page.evaluate(id=>JSON.stringify((window as any).PM.L(id).p['position.y'].kf),id);
  await chooseNativeMenu(session,'Clean Motion Path',()=>panel.getByRole('button',{name:'More easing tools'}).click());
  const curve=await session.page.evaluate(id=>{
    const PM=(window as any).PM,layer=PM.L(id);
    return {keys:JSON.stringify(layer.p['position.y'].kf),samples:Array.from({length:61},(_,i)=>[PM.ev(layer,'position.x',i/30),PM.ev(layer,'position.y',i/30)]),expr:layer.p['position.y'].expr};
  },id);
  expect(curve.keys).toBe(before);expect(curve.expr).toContain('motionPath(');
  for(const [x,y] of curve.samples)expect(y).toBeCloseTo(x*x/200,7);
  const selected=await session.page.evaluate(()=>{const PM=(window as any).PM;return PM.sel.layers.map((id:string)=>PM.L(id).name);});
  expect(selected[0]).toContain('Path progress');
  await panel.getByRole('button',{name:'Apply Ease in',exact:true}).click();
  const retimed=await session.page.evaluate(id=>{const PM=(window as any).PM,layer=PM.L(id);return {keys:JSON.stringify(layer.p['position.y'].kf),x:PM.ev(layer,'position.x',1),y:PM.ev(layer,'position.y',1)};},id);
  expect(retimed.keys).toBe(before);expect(retimed.y).toBeCloseTo(retimed.x*retimed.x/200,7);expect(retimed.x).not.toBeCloseTo(curve.samples[30][0],3);

  await session.page.evaluate(()=>{const PM=(window as any).PM;PM.setTime(0,{force:true});PM.play();});
  await expect.poll(()=>session.page.evaluate(()=>(window as any).PM.time)).toBeGreaterThan(.15);
  const playback=await session.page.evaluate(id=>{const PM=(window as any).PM;PM.pause();const layer=PM.L(id);return [PM.ev(layer,'position.x',PM.time),PM.ev(layer,'position.y',PM.time),PM.expressionErrors.get(layer.p['position.y'])];},id);
  expect(playback[1]).toBeCloseTo(playback[0]**2/200,7);expect(playback[2]).toBeUndefined();
  const vector={name:'Vector import',presetData:[[0,50,5],[60,25,5],[100,0,5]].map((value,i)=>({time:i/2,value,intTypeIn:6613,intTypeOut:6613,tempEaseIn:value.map(()=>({speed:0,influence:40})),tempEaseOut:value.map(()=>({speed:0,influence:60}))}))};
  await panel.locator('input[type=file]').first().setInputFiles({name:'Vector.kease',mimeType:'application/json',buffer:Buffer.from(JSON.stringify(vector))});
  for(const axis of ['X','Y','Z'])await expect(panel.getByRole('button',{name:`Apply Vector import · ${axis}`,exact:true})).toBeVisible();
  const saved=await session.page.evaluate(()=>JSON.parse(localStorage.getItem('pm.easing.library.v1')!));expect(saved.groups.at(-1).presets).toHaveLength(3);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
