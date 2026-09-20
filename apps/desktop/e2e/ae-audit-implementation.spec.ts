import {test,expect,chooseNativeMenu} from './helpers/app';
import path from 'node:path';
import {importFixture} from './helpers/media';
import {mkdir,readFile} from 'node:fs/promises';
const evidence='/private/tmp/powermove-ae-implementation-visual';
async function clean(page:any){await page.waitForFunction(()=>{const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');return viewer?.ov&&PM.GL?.gl;});await page.evaluate(()=>{const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');PM.replaceProject(PM.mkProject({name:'AE fundamentals QA',w:640,h:360,fps:30,dur:2,bg:'#000000'}));PM.setTime(0,{force:true});PM.selectLayers([]);viewer.fit=true;viewer.layout();});}
async function point(page:any,x:number,y:number){const b=await page.locator('#stage-inner').boundingBox(),z=await page.evaluate(()=>{const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');return viewer.shown;});return {x:b.x+x*z,y:b.y+y*z};}

test('editable Pen, canvas text, graph and preview controls render coherently',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);await mkdir(evidence,{recursive:true});
 await chooseNativeMenu(session,'Pen Tool (G)',()=>page.getByRole('button',{name:'Drawing tools',exact:true}).click());
 for(const [x,y] of [[100,100],[260,80],[300,230]]){const p=await point(page,x!,y!);await page.mouse.click(p.x,p.y);}
 const first=await point(page,100,100);await page.mouse.dblclick(first.x,first.y);
 expect(await page.evaluate(()=>{const PM=(window as any).PM,l=PM.proj.layers[0];return {count:l.d.paths[0].vertices.length,closed:l.d.paths[0].p.closed.v};})).toEqual({count:3,closed:true});
 await expect(page.getByRole('button',{name:'Edit Path vertices',exact:true})).toBeVisible();
 await page.screenshot({path:path.join(evidence,'paths.png')});
 const pathState=await page.evaluate(()=>{const PM=(window as any).PM;return PM.serialize();});expect(JSON.stringify(pathState)).toContain('vertices');
 await clean(page);await page.getByRole('button',{name:'Horizontal Type Tool (Command+T)',exact:true}).click();const p=await point(page,80,100);await page.mouse.click(p.x,p.y);
 const text=page.getByRole('textbox',{name:'Edit text on canvas'});await expect(text).toBeFocused();await text.fill('Motion stays editable');await page.keyboard.press('Meta+Enter');
 await page.evaluate(()=>{const PM=(window as any).PM;PM.Edit.apply({type:'set_property',target:PM.proj.layers[0].id,path:'c.size',value:36,preserveHandEdits:false});});
 expect(await page.evaluate(()=>(window as any).PM.proj.layers[0].d.text)).toMatchObject({v:'Motion stays editable'});
 await page.getByRole('button',{name:'Add animator',exact:true}).click();await expect(page.getByText('Animator 1',{exact:true})).toBeVisible();
 await page.screenshot({path:path.join(evidence,'text-animator.png')});
 await expect(page.getByRole('combobox',{name:'Preview resolution',exact:true})).toBeVisible();
 await page.waitForFunction(()=>(window as any).PM.quality===1);
 const full=await page.evaluate(()=>[(window as any).PM.GL.canvas.width,(window as any).PM.GL.canvas.height]);
 await page.locator('#preview-controls select').selectOption('0.25');
 await expect.poll(()=>page.evaluate(()=>[(window as any).PM.GL.canvas.width,(window as any).PM.GL.canvas.height])).toEqual(full.map(n=>Math.round(n*.25)));
 const geometry=await page.locator('#tl-head').evaluate(el=>{const r=el.getBoundingClientRect();return [...el.querySelectorAll('button,input,select')].filter((e:any)=>getComputedStyle(e).display!=='none').map(e=>({name:e.getAttribute('aria-label')||e.getAttribute('title'),right:e.getBoundingClientRect().right,bound:r.right}));});
 expect(geometry.every((r:any)=>r.right<=r.bound+1)).toBe(true);
 expect(session.diagnostics.pageErrors).toEqual([]);
});

test('track mattes produce reusable alpha and inverted alpha pixels and survive save',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);
 const results=await page.evaluate(()=>{const PM=(window as any).PM,matte=PM.mkLayer('shape',{name:'Matte',p:{'position.x':320,'position.y':180},d:{shape:'rect',w:100,h:100,color:'#ffffff',stroke:0}}),source=PM.mkLayer('solid',{name:'Fill',p:{'position.x':0,'position.y':0},d:{color:'#ff0000',w:640,h:360}});PM.proj.layers=[matte,source];PM.ProjectIndex.invalidate();PM.touch();PM.Edit.apply({type:'set_layer',target:source.id,patch:{matteSource:matte.id}});const read=()=>{const px=PM.GL.renderToPixels(0,640,360,{transparent:true,mblur:false});return {center:Array.from(px.slice((180*640+320)*4,(180*640+320)*4+4)),corner:Array.from(px.slice(0,4))};};const alpha=read();PM.Edit.apply({type:'set_property',target:source.id,path:'l.matteMode',value:'alpha-inverted',preserveHandEdits:false});const inverted=read();return {alpha,inverted,saved:PM.serialize()};});
 expect(results.alpha.center[3]).toBeGreaterThan(245);expect(results.alpha.corner[3]).toBe(0);expect(results.inverted.center[3]).toBeLessThan(10);expect(results.inverted.corner[3]).toBeGreaterThan(245);expect(JSON.stringify(results.saved)).toContain('matteSource');expect(session.diagnostics.pageErrors).toEqual([]);
});

test('precompose preserves cross-boundary animated rig pixels at several frames',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);
 const differences=await page.evaluate(()=>{const PM=(window as any).PM,parent=PM.mkLayer('null',{p:{'position.x':100,rotation:20}}),child=PM.mkLayer('shape',{parent:parent.id,p:{'position.x':100,'position.y':100},d:{shape:'rect',w:100,h:60,color:'#ffffff',stroke:0}});child.parent=parent.id;parent.p.rotation.kf=[PM.KF(0,0),PM.KF(2,60)];child.from=.3;child.dur=1.7;PM.proj.layers=[child,parent];PM.ProjectIndex.invalidate();PM.touch();const times=[.4,1,1.8],render=(t:number)=>PM.GL.renderToPixels(t,320,180,{transparent:true,mblur:false}),before=times.map(render);if(!before.some((px:any)=>px.some((v:number)=>v>0)))throw new Error('Reference frames must contain pixels');PM.hist.do('Precompose',()=>PM.precompose([child.id],'Nested rig'));return times.map((t:number,i:number)=>{const after=render(t);let delta=0;for(let j=0;j<after.length;j++)delta+=Math.abs(after[j]-before[i][j]);return delta/after.length;});});
 expect(differences.every((v:number)=>v<1)).toBe(true);expect(session.diagnostics.pageErrors).toEqual([]);
});


test('multiple graph curves, velocity editing and a cached preview retain editable animation',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);
 await page.evaluate(()=>{const PM=(window as any).PM,timeline=PM.Kernel.services.get('timeline');const a=PM.mkLayer('shape',{name:'A',p:{'position.x':160,'position.y':180},d:{w:60,h:60,color:'#ff8000'}}),b=PM.mkLayer('shape',{name:'B',p:{'position.x':420,'position.y':180},d:{w:60,h:60,color:'#5599ff'}});PM.proj.layers=[a,b];PM.ProjectIndex.invalidate();for(const l of [a,b]){PM.setKeyOn(l.p['position.y'],0,100,'ease',30);PM.setKeyOn(l.p['position.y'],1,240,'ease',30);}PM.touch();PM.selectLayers([a.id,b.id]);PM.sel.chan='position.y';timeline.reveal(a,['position.y']);timeline.reveal(b,['position.y']);PM.sel.keys=PM.proj.layers.flatMap((l:any)=>l.p['position.y'].kf.map((k:any)=>k.i));PM.invalidate();});
 await expect(page.locator('.color-field').first()).toContainText('Mixed');
 await page.getByRole('button',{name:'Graph editor (Shift+F3)',exact:true}).click();
 await page.waitForFunction(()=>{const PM=(window as any).PM,timeline=PM.Kernel.services.get('timeline');return timeline._graph?.series?.length>=2;});
 await page.evaluate(()=>{const PM=(window as any).PM;PM.sel.keys=PM.proj.layers.flatMap((l:any)=>l.p['position.y'].kf.map((k:any)=>k.i));});
 const graphKey=await page.evaluate(()=>{const PM=(window as any).PM,timeline=PM.Kernel.services.get('timeline'),p=timeline._graph.points.find((p:any)=>p.x>timeline.gut+8);return {x:p.x,y:p.y};});
 await page.locator('#tl-canvas').click({button:'right',position:graphKey});
 await page.getByRole('menuitem',{name:'Keyframe Velocity…',exact:true}).click();await page.getByRole('spinbutton',{name:'Outgoing speed',exact:true}).fill('80');await page.getByRole('button',{name:'Apply',exact:true}).click();
 expect(await page.evaluate(()=>(window as any).PM.proj.layers.map((l:any)=>l.p['position.y'].kf[0].outEase.speed))).toEqual([80,80]);
 await page.screenshot({path:path.join(evidence,'multi-graph.png')});
 await page.evaluate(()=>(window as any).PM.theme.apply('dark'));await session.app.evaluate(({BrowserWindow})=>BrowserWindow.getAllWindows()[0]!.setSize(1100,800));await page.screenshot({path:path.join(evidence,'multi-graph-dark-narrow.png')});
 const overflow=await page.locator('#tl-head').evaluate(el=>{const bounds=el.getBoundingClientRect();return [...el.querySelectorAll('button,select,input')].some(e=>{const b=e.getBoundingClientRect();return b.width>0&&b.right>bounds.right+1;});});expect(overflow).toBe(false);
 await page.locator('#preview-controls select').selectOption('0.25');
 await page.evaluate(()=>(window as any).PM.Preview.cache());await page.waitForFunction(()=>(window as any).PM.Preview.active);
 expect(await page.evaluate(()=>(window as any).PM.Preview.count)).toBe(60);
 await page.evaluate(()=>(window as any).PM.Preview.stop());
 expect(await page.evaluate(()=>(window as any).PM.Preview.active)).toBe(false);
 await page.evaluate(()=>{const PM=(window as any).PM;PM.hist.undo();});
 expect(await page.evaluate(()=>(window as any).PM.proj.layers[0].p['position.y'].kf[0].outEase.speed)).not.toBe(80);
 expect(session.diagnostics.pageErrors).toEqual([]);
});

test('offline frame preparation gives duplicated video instances their distinct source times',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);await importFixture(page,'h264-aac.mp4');
 await page.waitForFunction(()=>(window as any).PM.proj.layers.some((l:any)=>l.type==='video'));
 const result=await page.evaluate(async()=>{const PM=(window as any).PM,a=PM.proj.layers.find((l:any)=>l.type==='video');a.from=0;a.dur=2;a.d.trim=PM.P(0);a.d.speed=PM.P(1);const b=structuredClone(a);b.id=PM.uid('l');b.d.trim=PM.P(1);PM.proj.layers=[a,b];PM.ProjectIndex.invalidate();PM.touch();PM.agentFrameCapture=true;try{await PM.prepareFrame(.25);}finally{PM.agentFrameCapture=false;}const read=(l:any)=>{const cv=PM.preparedVideoFrames.get(l.id+'@0.25');return [...cv.getContext('2d').getImageData(cv.width/2,cv.height/2,1,1).data];};return {a:read(a),b:read(b),count:PM.preparedVideoFrames.size};});
 expect(result.count).toBe(2);expect(result.a[0]).toBeGreaterThan(220);expect(result.a[1]).toBeLessThan(30);expect(result.b[1]).toBeGreaterThan(220);expect(result.b[0]).toBeLessThan(30);expect(session.diagnostics.pageErrors).toEqual([]);
});


test('a drawn mask, luma matte and live text selector change pixels and undo',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);
 await page.evaluate(()=>{const PM=(window as any).PM,l=PM.mkLayer('solid',{d:{w:640,h:360,color:'#ffffff'}});PM.proj.layers=[l];PM.ProjectIndex.invalidate();PM.selectLayers(l.id);PM.touch();PM.invalidate();});
 await chooseNativeMenu(session,'Pen Tool (G)',()=>page.getByRole('button',{name:'Drawing tools',exact:true}).click());for(const [x,y] of [[100,100],[300,100],[300,240],[100,240],[100,100]]){const p=await point(page,x!,y!);await page.mouse.click(p.x,p.y);}
 const mask=await page.evaluate(()=>{const PM=(window as any).PM,l=PM.proj.layers[0],px=PM.GL.renderToPixels(0,640,360,{transparent:true,mblur:false});return {count:l.masks[0].path.vertices.length,center:px[(180*640+200)*4+3],corner:px[3],saved:PM.serialize()};});expect(mask.count).toBe(4);expect(mask.center).toBeGreaterThan(245);expect(mask.corner).toBe(0);expect(JSON.stringify(mask.saved)).toContain('vertices');
 await clean(page);
 const output=await page.evaluate(()=>{const PM=(window as any).PM,l=PM.mkLayer('text',{p:{'position.x':70,'position.y':100},d:{text:'Live type',size:48,color:'#ffffff'}});PM.proj.layers=[l];PM.ProjectIndex.invalidate();PM.touch();PM.selectLayers(l.id);const render=()=>PM.GL.renderToPixels(0,640,360,{transparent:true,mblur:false});const before=render();PM.Edit.mutate('Add selector',()=>{l.d.animators=[{id:'a',p:Object.fromEntries(Object.entries({unit:'characters',start:0,end:100,offset:0,smoothness:0,x:0,y:0,rotation:0,scale:100,opacity:0,tracking:0}).map(([k,v])=>[k,PM.P(v)]))}];PM.touch();});const after=render();PM.hist.undo();const restored=render();return {before:before.reduce((a:number,v:number)=>a+v,0),after:after.reduce((a:number,v:number)=>a+v,0),restored:restored.reduce((a:number,v:number)=>a+v,0)};});expect(output.before).toBeGreaterThan(10000);expect(output.after).toBe(0);expect(output.restored).toBe(output.before);
 const luma=await page.evaluate(()=>{const PM=(window as any).PM,m=PM.mkLayer('solid',{name:'Gray matte',d:{w:640,h:360,color:'#808080'}}),l=PM.mkLayer('solid',{name:'Fill',d:{w:640,h:360,color:'#ffffff'}});PM.proj.layers=[m,l];PM.ProjectIndex.invalidate();PM.touch();PM.Edit.apply([{type:'set_layer',target:l.id,patch:{matteSource:m.id}},{type:'set_property',target:l.id,path:'l.matteMode',value:'luma',preserveHandEdits:false}]);const a=PM.GL.renderToPixels(0,16,16,{transparent:true,mblur:false})[3];PM.Edit.apply({type:'set_property',target:l.id,path:'l.matteMode',value:'luma-inverted',preserveHandEdits:false});return [a,PM.GL.renderToPixels(0,16,16,{transparent:true,mblur:false})[3]];});expect(luma[0]).toBeGreaterThan(115);expect(luma[0]).toBeLessThan(140);expect(luma[0]+luma[1]).toBeCloseTo(255,0);expect(session.diagnostics.pageErrors).toEqual([]);
});

test('queued ProRes delivery retains alpha, fractional rate and the current project',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);const file=path.join(session.userData,'queued.mov');await session.app.evaluate(({dialog},file)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath:file});},file);
 const result=await page.evaluate(async()=>{const PM=(window as any).PM;PM.proj.w=64;PM.proj.h=64;PM.proj.dur=3/(24000/1001);PM.proj.work=[0,PM.proj.dur];PM.proj.layers=[PM.mkLayer('shape',{p:{'position.x':32,'position.y':32},d:{w:24,h:24,color:'#ff0000',stroke:0}})];PM.ProjectIndex.invalidate();PM.touch();PM.time=.04;const original=PM.proj;PM.Export.enqueue({format:'prores',scale:1,fps:24000/1001,range:'all',quality:'high',mblur:false,alpha:true,audio:false});await PM.Export.runQueue();return {same:PM.proj===original,time:PM.time,status:PM.Export.queue[0].status,error:PM.Export.queue[0].error};});expect(result).toEqual({same:true,time:.04,status:'complete',error:''});const bytes=await readFile(file);expect(bytes.length).toBeGreaterThan(1000);expect(bytes.toString('latin1')).toContain('ap4h');await page.getByRole('button',{name:'Close',exact:true}).click();await page.evaluate(async()=>{await (window as any).PM.flushProject();if(!await (window as any).PM.prepareToClose())throw Error('Close unexpectedly cancelled');});await session.relaunch();await session.page.waitForFunction(()=>(window as any).PM.Export?.queue?.[0]?.status==='complete');expect(session.diagnostics.pageErrors).toEqual([]);
});


test('output presets restore settings and persist across desktop sessions',async({session})=>{
 await session.openEditor();
 const {page}=session;await clean(page);await page.evaluate(()=>{const PM=(window as any).PM;PM.Export.remember({format:'prores',scale:.5,fps:24000/1001,range:'work',quality:'high',mblur:false,alpha:true,audio:false});PM.Export.dialog();});
 await page.getByRole('textbox',{name:'Render preset name',exact:true}).fill('Alpha master');await page.getByRole('button',{name:'Save',exact:true}).click();await page.getByRole('button',{name:'Cancel',exact:true}).click();
 await page.evaluate(async()=>{const PM=(window as any).PM;await PM.store.flush();});await session.relaunch();
 expect(await session.page.evaluate(()=>(window as any).PM.store.get('renderPresets',[])[0])).toMatchObject({name:'Alpha master',options:{format:'prores',scale:.5,alpha:true,fps:24000/1001}});expect(session.diagnostics.pageErrors).toEqual([]);
});
