import {test,expect} from './helpers/app';
test.beforeEach(async ({session})=>{await session.openEditor();});
test('3D groups transform flat children together, including nested groups, with editable animation and Undo',async({session})=>{
 const {page}=session;
 await page.waitForFunction(()=>Boolean((window as any).PM?.GL?.gl));
 await page.evaluate(()=>{
  const PM=(window as any).PM,project=PM.mkProject({name:'3D group review',w:640,h:360,dur:3,bg:'#101018'});
  const a=PM.mkLayer('shape',{name:'Left card',d:{w:130,h:140,color:'#FF6040'},p:{'position.x':240,'position.y':180}},project);
  const b=PM.mkLayer('shape',{name:'Right card',d:{w:130,h:140,color:'#4060FF'},p:{'position.x':400,'position.y':180}},project);
  project.layers=[a,b];PM.replaceProject(project);PM.setTime(0,{force:true});
  PM.Edit.apply({type:'group_layers',targets:[a.id,b.id],name:'Cards'});const inner=PM.firstSel();
  PM.Edit.apply({type:'group_layers',targets:[inner.id],name:'3D rig'});PM.invalidate();
 });
 await page.getByRole('button',{name:'3D layer',exact:true}).click();
 const result=await page.evaluate(()=>{
  const PM=(window as any).PM,g=PM.firstSel();
  const pixels=(time:number)=>{const cv=PM.renderFrameTo(time,640,360);return Array.from(cv.getContext('2d').getImageData(0,0,640,360).data);};
  const before=pixels(0);
  const edit=PM.Edit.apply([{type:'set_property',target:g.id,path:'rotation.y',value:40},{type:'set_property',target:g.id,path:'position.z',value:100},{type:'set_property',target:g.id,path:'rotation.x',value:0,time:0,mode:'keyframe'},{type:'set_property',target:g.id,path:'rotation.x',value:25,time:2,mode:'keyframe'}],{label:'Tilt group'});
  const after=pixels(0),later=pixels(2);PM.setTime(0,{force:true});
  const viewer=PM.Kernel.services.get('viewer'),children=PM.proj.layers.filter((l:any)=>l.type==='shape'),box=viewer.resolveSelectionGeometry();
  const picked=children.every((l:any)=>{const p=viewer.layerWorldPivot(l,0);return PM.GL.pick(p.x,p.y,0)?.id===l.id;});
  const undo=PM.hist.undo(),undone=pixels(0);PM.hist.redo();PM.selectLayers(g.id);PM.invalidate();
  return {edit,changed:JSON.stringify(before)!==JSON.stringify(after),animated:JSON.stringify(after)!==JSON.stringify(later),undo,restored:JSON.stringify(before)===JSON.stringify(undone),switches:children.map((l:any)=>!!l.threeD),picked,box:box.corners.every((p:any)=>Number.isFinite(p.x)&&Number.isFinite(p.y)),errors:[...PM.GL.errors.entries()]};
 });
 expect(result.edit.ok).toBe(true);expect(result.changed).toBe(true);expect(result.animated).toBe(true);expect(result.undo).toBe(true);expect(result.restored).toBe(true);expect(result.switches).toEqual([false,false]);expect(result.picked).toBe(true);expect(result.box).toBe(true);expect(result.errors).toEqual([]);
 const drag=await page.evaluate(()=>{const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer'),p=viewer.layerWorldPivot(PM.firstSel(),0),r=viewer.inner.getBoundingClientRect();return {x:r.left+p.x*viewer.shown,y:r.top+p.y*viewer.shown};});
 await page.mouse.move(drag.x,drag.y);await page.mouse.down();await page.mouse.move(drag.x+40,drag.y-20,{steps:8});await page.mouse.up();
 const moved=await page.evaluate(()=>{const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer'),p=viewer.layerWorldPivot(PM.firstSel(),0),r=viewer.inner.getBoundingClientRect();return {type:PM.firstSel().type,x:r.left+p.x*viewer.shown,y:r.top+p.y*viewer.shown};});
 expect(moved.type).toBe('group');expect(moved.x-drag.x).toBeCloseTo(40,0);expect(moved.y-drag.y).toBeCloseTo(-20,0);
 await page.screenshot({path:'/tmp/powermove-25d-groups-review.png'});
 const mask=await page.evaluate(()=>{
  const PM=(window as any).PM,child=PM.proj.layers.find((l:any)=>l.name==='Right card');
  const count=()=>{const cv=PM.renderFrameTo(0,640,360),px=cv.getContext('2d').getImageData(0,0,640,360).data;let n=0;for(let i=0;i<px.length;i+=4)if(px[i+2]>px[i]+50 && px[i+2]>100)n++;return n;};
  const before=count(),mask=PM.mkMask('rect');mask.p.w.v=40;mask.p.h.v=40;mask.p.feather.v=0;child.masks=[mask];PM.touch();
  return {before,after:count(),errors:[...PM.GL.errors.entries()]};
 });
 expect(mask.after).toBeGreaterThan(100);expect(mask.after).toBeLessThan(mask.before/3);expect(mask.errors).toEqual([]);
 expect(session.diagnostics.pageErrors).toEqual([]);
});
