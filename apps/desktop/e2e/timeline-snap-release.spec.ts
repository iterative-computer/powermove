import {test,expect} from './helpers/app';

test('a snapped playhead survives release jitter and mouse-up while Shift remains held',async({session})=>{
  await session.openEditor();const page=session.page;
  await page.evaluate(()=>{
    const PM=(window as any).PM,p=PM.mkProject({name:'Snap release',dur:8,fps:30});PM.pause();PM.AutomaticPreview?.cancel?.();
    p.layers=[PM.mkLayer('solid',{name:'Off-frame edge',from:2.013,dur:3},p)];PM.replaceProject(p);
    const timeline=PM.Kernel.services.get('timeline');timeline.pps=100;timeline.scrollT=0;PM.invalidate('timeline');
  });
  await page.evaluate(()=>new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r()))));
  const point=await page.evaluate(()=>{const t=(window as any).PM.Kernel.services.get('timeline'),b=t.cv.getBoundingClientRect();return {x:b.left+t.gut,y:b.top+t.ruler-6};});
  await page.mouse.move(point.x+193,point.y);await page.mouse.down();await page.keyboard.down('Shift');
  await page.mouse.move(point.x+205,point.y,{steps:4});
  const preview=await page.evaluate(()=>{const PM=(window as any).PM,t=PM.Kernel.services.get('timeline');return {time:PM.time,guide:t.snapGuide};});
  expect(preview.guide).toBeCloseTo(2+1/30,9);
  await page.mouse.move(point.x+204,point.y);
  expect(await page.evaluate(()=>(window as any).PM.time)).toBeCloseTo(2+1/30,9);
  await page.mouse.up();
  const after=await page.evaluate(()=>({time:(window as any).PM.time,guide:(window as any).PM.Kernel.services.get('timeline').snapGuide}));
  await page.keyboard.up('Shift');
  expect(after.time).toBeCloseTo(2+1/30,9);expect(after.guide).toBeNull();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a snapped layer bar retains exact timing through release jitter, mouse-up, Undo and Redo',async({session})=>{
  await session.openEditor();const page=session.page;
  const id=await page.evaluate(()=>{
    const PM=(window as any).PM,p=PM.mkProject({name:'Layer snap release',dur:8,fps:30});PM.pause();PM.AutomaticPreview?.cancel?.();
    const moving=PM.mkLayer('solid',{name:'Moving',from:1,dur:1},p),target=PM.mkLayer('solid',{name:'Target',from:4,dur:1},p);p.layers=[moving,target];
    PM.replaceProject(p);PM.selectLayers([moving.id]);const t=PM.Kernel.services.get('timeline');t.setMode('layers');t.pps=100;t.scrollT=0;PM.invalidate('timeline');return moving.id;
  });
  await page.evaluate(()=>new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r()))));
  const point=await page.evaluate(id=>{const t=(window as any).PM.Kernel.services.get('timeline'),b=t.cv.getBoundingClientRect(),row=t.rows.findIndex((r:any)=>r.kind==='layer'&&r.L.id===id);return {x:b.left+t.gut+150,y:b.top+t.ruler+row*t.row-t.scrollY+t.row*.75};},id);
  const from=()=>page.evaluate(id=>(window as any).PM.L(id).from,id);
  await page.mouse.move(point.x,point.y);await page.mouse.down();await page.keyboard.down('Shift');
  await page.mouse.move(point.x+304,point.y,{steps:8});expect(await from()).toBe(4);
  await page.mouse.move(point.x+302,point.y);expect(await from()).toBe(4);
  await page.mouse.up();expect(await from()).toBe(4);await page.keyboard.up('Shift');expect(await from()).toBe(4);
  await page.evaluate(()=>(window as any).PM.hist.undo());expect(await from()).toBe(1);
  await page.evaluate(()=>(window as any).PM.hist.redo());expect(await from()).toBe(4);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('snapped keyframes retain their time on release with Shift held',async({session})=>{
  await session.openEditor();const page=session.page;
  const id=await page.evaluate(()=>{
    const PM=(window as any).PM,p=PM.mkProject({name:'Keyframe snap release',dur:8,fps:30}),layer=PM.mkLayer('solid',{name:'Keyframes',dur:8},p);p.layers=[layer];PM.replaceProject(p);
    PM.setKey(layer,'opacity',1,10);PM.setKey(layer,'opacity',6,90);PM.setTime(4,{force:true});PM.selectLayers([layer.id]);
    const t=PM.Kernel.services.get('timeline');t.setMode('layers');t.pps=100;t.scrollT=0;t.reveal(layer,['opacity']);PM.bus.emit('layers');PM.invalidate('timeline');return layer.id;
  });
  await page.waitForFunction(id=>(window as any).PM.Kernel.services.get('timeline').rows.some((r:any)=>r.kind==='prop'&&r.L.id===id&&r.key==='opacity'),id);
  const point=await page.evaluate(id=>{const t=(window as any).PM.Kernel.services.get('timeline'),b=t.cv.getBoundingClientRect(),row=t.rows.findIndex((r:any)=>r.kind==='prop'&&r.L.id===id&&r.key==='opacity');return {x:b.left+t.gut+100,y:b.top+t.ruler+row*t.row-t.scrollY+t.row/2};},id);
  const time=()=>page.evaluate(id=>(window as any).PM.L(id).p.opacity.kf[0].t,id);
  await page.mouse.move(point.x,point.y);await page.mouse.down();await page.keyboard.down('Shift');await page.mouse.move(point.x+304,point.y,{steps:8});expect(await time()).toBe(4);
  await page.mouse.move(point.x+302,point.y);expect(await time()).toBe(4);
  await page.mouse.up();expect(await time()).toBe(4);await page.keyboard.up('Shift');expect(await time()).toBe(4);
  await page.evaluate(()=>(window as any).PM.hist.undo());expect(await time()).toBe(1);await page.evaluate(()=>(window as any).PM.hist.redo());expect(await time()).toBe(4);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
