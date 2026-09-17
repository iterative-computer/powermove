import {test,expect} from './helpers/app';

const INSPECTOR='[data-svelte-panel="inspector"]';

async function clean(page:any){
 await page.waitForFunction(()=>{const PM=(window as any).PM;return PM?.Kernel?.services.get('viewer')&&PM.GL?.gl;});
 await page.evaluate(()=>{const PM=(window as any).PM;PM.replaceProject(PM.mkProject({name:'Bulk property QA',w:640,h:360,fps:30,dur:2,bg:'#000000'}));PM.setTime(0,{force:true});PM.selectLayers([]);});
}

/** Hex casing is a picker detail, not a fan-out one. */
const colors=(page:any)=>page.evaluate(()=>{const PM=(window as any).PM;return PM.proj.layers.map((l:any)=>String(PM.resolveContent(l,0).color).toLowerCase());});

/** A shape leads the selection so the inspector renders the shared "Fill" row. */
test('one fill edit repaints every selected layer that has a fill, across layer types',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);
 await page.evaluate(()=>{const PM=(window as any).PM,
  shape=PM.mkLayer('shape',{name:'Shape',p:{'position.x':160,'position.y':180},d:{shape:'rect',w:80,h:80,color:'#ff8000'}}),
  text=PM.mkLayer('text',{name:'Text',p:{'position.x':320,'position.y':180},d:{text:'Hi',size:40,color:'#112233'}}),
  solid=PM.mkLayer('solid',{name:'Solid',p:{'position.x':480,'position.y':180},d:{w:80,h:80,color:'#445566'}});
  PM.proj.layers=[shape,text,solid];PM.ProjectIndex.invalidate();PM.touch();
  PM.selectLayers([shape.id,text.id,solid.id]);PM.invalidate();});

 await expect(page.locator(INSPECTOR)).toContainText('3 layers selected');
 const fill=page.locator(INSPECTOR).getByRole('button',{name:'Fill',exact:true});
 await expect(fill).toContainText('Mixed');

 await fill.click();
 const hex=page.getByRole('textbox',{name:'Fill hex value'});
 await hex.click();await hex.fill('#00ccff');await hex.press('Enter');

 expect(await colors(page)).toEqual(['#00ccff','#00ccff','#00ccff']);
 await expect(fill).toContainText('#00CCFF');

 await page.evaluate(()=>(window as any).PM.hist.undo());
 expect(await colors(page)).toEqual(['#ff8000','#112233','#445566']);
 expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a shared dimension edit reaches mixed types while type-private fields stay put',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);
 await page.evaluate(()=>{const PM=(window as any).PM,
  shape=PM.mkLayer('shape',{name:'Shape',p:{'position.x':160,'position.y':180},d:{shape:'rect',w:80,h:80,color:'#ff8000'}}),
  solid=PM.mkLayer('solid',{name:'Solid',p:{'position.x':320,'position.y':180},d:{w:120,h:120,color:'#445566'}});
  PM.proj.layers=[shape,solid];PM.ProjectIndex.invalidate();PM.touch();
  PM.selectLayers([shape.id,solid.id]);PM.invalidate();});

 const width=page.locator(INSPECTOR).getByRole('spinbutton',{name:'Width',exact:true});
 await width.click();await width.fill('200');await width.press('Enter');

 const state=await page.evaluate(()=>{const PM=(window as any).PM;return PM.proj.layers.map((l:any)=>{const c=PM.resolveContent(l,0);return {type:l.type,w:c.w,shape:c.shape};});});
 expect(state.map((s:any)=>s.w)).toEqual([200,200]);
 expect(state[1].shape).toBeUndefined();
 expect(session.diagnostics.pageErrors).toEqual([]);
});
