import { expect, test } from './helpers/app';

test.describe('@groups editable timeline groups', () => {
  test('grouping preserves rendered pixels, time, parent rigs, save data and Undo', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl && (window as any).PM?.TL?.rows));
    const result = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Grouping', w: 320, h: 180, fps: 30, dur: 6, bg: '#FFFFFF' });
      const a = PM.mkLayer('shape', { name: 'Card', from: 1, dur: 4, d: { w: 100, h: 80, color: '#EC6340' }, p: { 'position.x': 100, 'position.y': 90 } }, project);
      const b = PM.mkLayer('shape', { name: 'Badge', from: 2, dur: 3, d: { w: 60, h: 60, color: '#386BDC' }, p: { 'position.x': 170, 'position.y': 90 } }, project);
      project.layers = [a, b]; PM.replaceProject(project); PM.setTime(2.5, {raw:true,force:true});
      const pixels = () => { const canvas=PM.renderFrameTo(2.5,320,180); return Array.from(canvas.getContext('2d').getImageData(0,0,320,180).data); };
      const before=pixels();
      const response=PM.Edit.apply({type:'group_layers',targets:[a.id,b.id],name:'Design'},{origin:'agent',label:'Group layers'});
      const group=PM.firstSel(); const after=pixels(); const saved=JSON.parse(PM.serialize()).proj;
      const undo=PM.hist.undo(); const undone=PM.proj.layers.length; PM.hist.redo();
      PM.selectLayers(group.id); PM.invalidate();
      return {ok:response.ok,type:group.type,comps:Object.keys(PM.proj.comps).length, equal:JSON.stringify(before)===JSON.stringify(after), saved: saved.layers.filter((l:any)=>l.group===group.id).length, undo,undone, times:PM.proj.layers.filter((l:any)=>l.type!=='group').map((l:any)=>l.from)};
    });
    expect(result).toEqual({ok:true,type:'group',comps:0,equal:true,saved:2,undo:true,undone:2,times:[1,2]});
    await expect(page.locator('[data-inspector-layer]')).toBeVisible();
    await page.waitForFunction(() => (window as any).PM.TL.rows.length === 3);
    const twirl = await page.evaluate(() => { const T=(window as any).PM.TL,r=T.cv.getBoundingClientRect(); return {x:r.left+64,y:r.top+T.ruler+T.row/2-T.scrollY}; });
    await page.mouse.click(twirl.x,twirl.y);
    await page.waitForFunction(() => (window as any).PM.TL.rows.length === 1);
    await page.mouse.click(twirl.x,twirl.y);
    await page.waitForFunction(() => (window as any).PM.TL.rows.length === 3);
    await page.screenshot({path:'/tmp/powermove-groups-review.png'});
    const clip = await page.evaluate(() => { const T=(window as any).PM.TL,r=T.cv.getBoundingClientRect();return{x:r.left+T.gut+(1.5-T.scrollT)*T.pps,y:r.top+T.ruler+T.row/2-T.scrollY,delta:T.pps/2}; });
    await page.mouse.move(clip.x,clip.y); await page.mouse.down(); await page.mouse.move(clip.x+clip.delta,clip.y,{steps:8}); await page.mouse.up();
    expect(await page.evaluate(() => (window as any).PM.proj.layers.filter((layer:any)=>layer.type!=='group').map((layer:any)=>layer.from))).toEqual([1.5,2.5]);
    await page.evaluate(() => (window as any).PM.hist.undo());
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('canvas and timeline open the same compact layer menu', async ({ session }) => {
    const {page}=session;
    await page.waitForFunction(()=>Boolean((window as any).PM?.GL?.gl && (window as any).PM?.TL?.cv));
    await page.evaluate(()=>{
      const PM=(window as any).PM;
      const project=PM.mkProject({w:640,h:360,dur:5});
      const layer=PM.mkLayer('shape',{name:'Menu card',d:{w:300,h:200}},project);
      project.layers=[layer];PM.replaceProject(project);PM.setTime(1,{force:true});PM.selectLayers(layer.id);PM.invalidate();
    });
    const coords=await page.evaluate(()=>{
      const PM=(window as any).PM,V=PM.Viewer,rect=V.stage.getBoundingClientRect();
      return {x:rect.left+rect.width/2,y:rect.top+rect.height/2};
    });
    await page.mouse.click(coords.x,coords.y,{button:'right'});
    await expect(page.getByRole('menuitem',{name:/^Group layers/})).toBeVisible();
    const canvasItems=await page.getByRole('menuitem').allTextContents();
    expect(canvasItems.join(' ')).not.toContain('Shy layer');
    expect(canvasItems.join(' ')).not.toContain('Motion blur');
    await page.getByRole('menuitem',{name:'Lock',exact:true}).click();
    await page.mouse.click(coords.x,coords.y,{button:'right'});
    await page.getByRole('menuitem',{name:'Unlock',exact:true}).click();
    expect(await page.evaluate(()=>(window as any).PM.firstSel().lock)).toBe(false);
    await page.waitForFunction(()=>(window as any).PM.TL.rows.some((row:any)=>row.L?.name==='Menu card'));
    const row=await page.evaluate(()=>{const PM=(window as any).PM,T=PM.TL,r=T.cv.getBoundingClientRect();return {x:r.left+110,y:r.top+T.ruler+T.row/2-T.scrollY};});
    await page.mouse.click(row.x,row.y,{button:'right'});
    expect(await page.getByRole('menuitem').allTextContents()).toEqual(canvasItems);
    await page.getByRole('menuitem',{name:/^Group layers/}).click();
    await page.waitForFunction(()=>(window as any).PM.firstSel()?.type==='group');
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('pickwhip parents to a timeline row with no pose jump and one Undo',async({session})=>{
    const {page}=session;
    await page.waitForFunction(()=>Boolean((window as any).PM?.TL?.cv));
    const ids=await page.evaluate(()=>{
      const PM=(window as any).PM,p=PM.mkProject({w:640,h:360,dur:5});
      const child=PM.mkLayer('shape',{name:'Child',p:{'position.x':100,'position.y':80}},p),parent=PM.mkLayer('null',{name:'Parent rig',p:{'position.x':200,'position.y':120,rotation:30}},p);
      p.layers=[child,parent]; PM.replaceProject(p);PM.selectLayers(child.id);PM.setTime(1,{force:true});PM.invalidate();
      return {child:child.id,parent:parent.id,before:PM.worldMatrix(child,1)};
    });
    const whip=page.getByRole('button',{name:'Pick parent layer'});
    await expect(whip).toBeVisible();
    await whip.scrollIntoViewIfNeeded();
    const start=await whip.boundingBox();
    await page.waitForFunction((id)=>(window as any).PM.TL.rows.some((row:any)=>row.kind==='layer'&&row.L.id===id),ids.parent);
    const end=await page.evaluate((id)=>{const PM=(window as any).PM,T=PM.TL,r=T.cv.getBoundingClientRect(),index=T.rows.findIndex((row:any)=>row.kind==='layer'&&row.L.id===id);return{x:r.left+110,y:r.top+T.ruler+index*T.row+T.row/2-T.scrollY};},ids.parent);
    await page.mouse.move(start!.x+start!.width/2,start!.y+start!.height/2);await page.mouse.down();await page.mouse.move(end.x,end.y,{steps:10});
    await expect(page.locator('body > svg text')).toHaveText('Parent to Parent rig');
    await page.mouse.up();
    await page.screenshot({path:'/tmp/powermove-parenting-review.png'});
    const result=await page.evaluate((id)=>{const PM=(window as any).PM,child=PM.L(id);const result={parent:child.parent,pose:PM.worldMatrix(child,1)};PM.hist.undo();return {...result,undone:PM.L(id).parent};},ids.child);
    expect(result.parent).toBe(ids.parent);result.pose.forEach((v:number,i:number)=>expect(v).toBeCloseTo(ids.before[i],6));expect(result.undone).toBeNull();
    const parentCell = await page.evaluate(() => {const T=(window as any).PM.TL,r=T.cv.getBoundingClientRect();return{x:r.left+T.gut-12,y:r.top+T.ruler+T.row/2-T.scrollY};});
    await page.mouse.click(parentCell.x,parentCell.y);
    await page.getByRole('menuitem',{name:'Parent rig',exact:true}).click();
    expect(await page.evaluate(id=>(window as any).PM.L(id).parent,ids.child)).toBe(ids.parent);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
