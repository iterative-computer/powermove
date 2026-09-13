import { expect, test } from './helpers/app';

test.beforeEach(async ({session}) => {
  await session.openEditor();
  await session.page.waitForFunction(() => { const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'); return Boolean(PM?.GL?.gl && timeline?.cv); });
  await session.page.evaluate(() => {
    const PM=(window as any).PM;
    const project=PM.mkProject({name:'Group transforms',w:640,h:360,dur:5,fps:30,bg:'#FFFFFF'});
    const a=PM.mkLayer('shape',{name:'Orange card',d:{w:100,h:100,color:'#EF683B'},p:{'position.x':250,'position.y':180}},project);
    const b=PM.mkLayer('shape',{name:'Blue card',d:{w:100,h:100,color:'#4274EC'},p:{'position.x':390,'position.y':180}},project);
    project.layers=[a,b];PM.replaceProject(project);PM.setTime(0,{force:true});
    PM.Edit.apply({type:'group_layers',targets:[a.id,b.id],name:'Cards'},{origin:'agent'});
    const viewer=PM.Kernel.services.get('viewer');
    viewer.fit=false;viewer.zoom=.7;viewer.pan=[0,0];viewer.layout();PM.invalidate();
    (window as any).groupOriginal=JSON.stringify([a.p,b.p]);
  });
});

test('group Properties transform the rendered members and animate with one Undo',async({session})=>{
  const {page}=session;
  const set=async(name:string,value:string)=>{const field=page.getByRole('spinbutton',{name,exact:true});await field.click();await field.fill(value);await field.press('Enter');};
  await expect(page.getByRole('spinbutton',{name:'Position X',exact:true})).toHaveValue('320px');
  await set('Position X','400');
  expect(await page.evaluate(()=>{const PM=(window as any).PM;return PM.worldMatrix(PM.proj.layers.find((l:any)=>l.name==='Orange card'),0)[4];})).toBeCloseTo(330);
  await set('Scale X','150');
  await expect(page.getByRole('spinbutton',{name:'Scale Y',exact:true})).toHaveValue('150%');
  await set('Rotation','30');
  await set('Opacity','60');
  const proof=await page.evaluate(()=>{
    const PM=(window as any).PM,g=PM.firstSel(),a=PM.proj.layers.find((l:any)=>l.name==='Orange card');
    const canvas=PM.renderFrameTo(0,640,360),ctx=canvas.getContext('2d'),m=PM.worldMatrix(a,0);
    return {pixel:Array.from(ctx.getImageData(Math.round(m[4]),Math.round(m[5]),1,1).data),opacity:PM.worldOpacity(a,0),membersUnchanged:JSON.stringify(PM.proj.layers.filter((l:any)=>l.type!=='group').map((l:any)=>l.p))===(window as any).groupOriginal,selected:g.type};
  });
  expect(proof.membersUnchanged).toBe(true);expect(proof.selected).toBe('group');expect(proof.opacity).toBe(.6);
  expect(proof.pixel[0]).toBeGreaterThan(proof.pixel[2]!);expect(proof.pixel[2]).toBeLessThan(220);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  await expect(page.getByRole('spinbutton',{name:'Opacity',exact:true})).toHaveValue('100%');
  await page.getByRole('button',{name:'Animate Position X',exact:true}).click();
  await page.evaluate(()=>(window as any).PM.setTime(2));
  await set('Position X','480');
  expect(await page.evaluate(()=>{const PM=(window as any).PM;return PM.firstSel().p['position.x'].kf.map((k:any)=>[k.t,k.v]);})).toEqual([[0,400],[2,480]]);
  await page.waitForFunction(()=>{const PM=(window as any).PM,timeline=PM.Kernel.services.get('timeline');return timeline.rows.some((row:any)=>row.kind==='prop'&&row.L.id===PM.firstSel().id&&row.key==='position.x');});
  await page.screenshot({path:'/tmp/powermove-group-transforms-review.png'});
  const strip=await page.evaluate(()=>{const PM=(window as any).PM,timeline=PM.Kernel.services.get('timeline'),r=timeline.cv.getBoundingClientRect();return {x:r.left+timeline.gut+timeline.pps*.5,y:r.top+timeline.ruler+timeline.row/2,dx:timeline.pps*.5};});
  await page.mouse.move(strip.x,strip.y);await page.mouse.down();await page.mouse.move(strip.x+strip.dx,strip.y,{steps:8});await page.mouse.up();
  expect(await page.evaluate(()=>{const PM=(window as any).PM;return {starts:PM.proj.layers.map((l:any)=>l.from),value:PM.ev(PM.firstSel(),'position.x',2.5)};})).toEqual({starts:[.5,.5,.5],value:480});
  await page.evaluate(()=>(window as any).PM.hist.undo());
  const pixels=()=>page.evaluate(()=>{const PM=(window as any).PM,c=PM.renderFrameTo(1,640,360);return Array.from(c.getContext('2d').getImageData(0,0,640,360).data);});
  const grouped=await pixels();
  await page.getByRole('button',{name:'Ungroup',exact:true}).click();
  expect(await page.evaluate(()=>(window as any).PM.proj.layers.some((l:any)=>l.type==='group'))).toBe(false);
  expect(await pixels()).toEqual(grouped);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  expect(await page.evaluate(()=>{const PM=(window as any).PM,g=PM.proj.layers.find((l:any)=>l.type==='group');return {keys:g?.p['position.x'].kf.length,members:PM.proj.layers.filter((l:any)=>l.group===g?.id).length};})).toEqual({keys:2,members:2});

  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('canvas drag and keyboard nudge edit only group channels',async({session})=>{
  const {page}=session;
  const drag=await page.evaluate(()=>{
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer'),r=viewer.stage.getBoundingClientRect();
    return {x:r.left+r.width/2-60*viewer.shown,y:r.top+r.height/2,dx:40*viewer.shown};
  });
  await page.keyboard.down('Meta');
  await page.mouse.move(drag.x,drag.y);await page.mouse.down();await page.mouse.move(drag.x+drag.dx,drag.y,{steps:8});await page.mouse.up();
  await page.keyboard.up('Meta');
  const check=()=>page.evaluate(()=>{const PM=(window as any).PM;return {group:PM.firstSel().type,x:PM.ev(PM.firstSel(),'position.x',0),unchanged:JSON.stringify(PM.proj.layers.filter((l:any)=>l.type!=='group').map((l:any)=>l.p))===(window as any).groupOriginal};});
  expect(await check()).toEqual({group:'group',x:360,unchanged:true});
  await page.evaluate(()=>(window as any).PM.hist.undo());
  expect(await check()).toEqual({group:'group',x:320,unchanged:true});
  await page.keyboard.press('ArrowRight');
  expect(await check()).toEqual({group:'group',x:321,unchanged:true});
  await page.evaluate(()=>{const PM=(window as any).PM,g=PM.firstSel(),child=PM.proj.layers.find((l:any)=>l.type==='shape');PM.selectLayers([child.id,g.id]);});
  const rotation=page.getByRole('spinbutton',{name:'Rotation',exact:true});await rotation.click();await rotation.fill('15');await rotation.press('Enter');
  expect(await page.evaluate(()=>{const PM=(window as any).PM;return {rotation:PM.ev(PM.proj.layers.find((l:any)=>l.type==='group'),'rotation',0),unchanged:JSON.stringify(PM.proj.layers.filter((l:any)=>l.type!=='group').map((l:any)=>l.p))===(window as any).groupOriginal};})).toEqual({rotation:15,unchanged:true});
  expect(session.diagnostics.pageErrors).toEqual([]);
});
