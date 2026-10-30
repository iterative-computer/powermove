import { expect, test, repoRoot } from './helpers/app';
import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';

test.beforeEach(async({session})=>{await session.openEditor();await session.page.evaluate(()=>{
 const PM=(window as any).PM;PM.pause();PM.agentFrameCapture=true;PM.quality=1;PM.AutomaticPreview?.cancel();
 (window as any).__inputs={
  scene(depth=16){const p=PM.mkProject({name:'Layer inputs',w:64,h:64,dur:2});p.colorDepth=depth;p.backgroundFill={type:'none',stops:[],angle:0};
   const owner=PM.mkLayer('shape',{name:'Owner',d:{w:32,h:32,radius:0,color:'#FF0000'},p:{'position.x':32,'position.y':32}},p);
   const map=PM.mkLayer('shape',{name:'Hidden map',d:{w:16,h:16,radius:0,color:'#FFFFFF'},p:{'position.x':32,'position.y':32}},p);map.on=false;p.layers=[owner,map];PM.replaceProject(p);return{owner,map};},
  effect(owner:any,map:any,type='setMatte',values:any={},stage='source'){const f=PM.mkEffect(type,undefined,owner);f.inputs.map={layerId:map?.id??null,stage};for(const[k,v]of Object.entries(values))f.p[k].v=v;owner.fx=[f];PM.touch();return f;},
  render(size=64){const p=PM.GL.renderToPixels(0,size,size,{transparent:true,mblur:false});if(PM.GL.errors.size)throw Error(JSON.stringify([...PM.GL.errors]));return [...p];},
  pixel(p:number[],x:number,y:number,size=64){return p.slice(((size-1-y)*size+x)*4,((size-1-y)*size+x)*4+4);},
 };
});});

test('Set Matte samples hidden inputs, source/masks/effects stages, stretch and inversion on every precision',async({session},info)=>{
 const result=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__inputs,out:any[]=[];
 for(const depth of [8,16,32]){if(!PM.GL.supportedColorDepths.includes(depth))continue;const{owner,map}=s.scene(depth);
  s.effect(owner,map,'setMatte',{stretch:false});const centered=s.render();
  owner.fx[0].p.stretch.v=true;PM.touch();const stretched=s.render();
  const mask=PM.mkMask('rect',PM.proj);mask.p.w.v=8;mask.p.h.v=16;mask.p.feather.v=0;map.masks=[mask];
  map.fx=[PM.mkEffect('invert',undefined,map)];map.fx[0].p.channel.v=12;
  const f=s.effect(owner,map,'setMatte',{},'source');const source=s.render();f.inputs.map.stage='masks';PM.touch();const masked=s.render();f.inputs.map.stage='effects';PM.touch();const effected=s.render();
  f.inputs.map.stage='source';f.p.invert.v=true;PM.touch();const inverted=s.render();
  out.push({depth,center:s.pixel(centered,32,32),edge:s.pixel(centered,20,32),stretch:s.pixel(stretched,20,32),source:s.pixel(source,20,32),mask:s.pixel(masked,20,32),effect:s.pixel(effected,32,32),inverted:s.pixel(inverted,32,32),leaks:PM.GL.pool.filter((f:any)=>f.busy).length});
 }return out;});
 for(const r of result){expect(r.center).toEqual([255,0,0,255]);expect(r.edge).toEqual([0,0,0,0]);expect(r.stretch).toEqual([255,0,0,255]);expect(r.source).toEqual([255,0,0,255]);expect(r.mask).toEqual([0,0,0,0]);expect(r.effect).toEqual([0,0,0,0]);expect(r.inverted).toEqual([0,0,0,0]);}
 await info.attach('layer-input-pixels',{body:JSON.stringify(result,null,2),contentType:'application/json'});expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Displacement Map produces signed offsets, preserves alpha, supports wrap and neutral Off',async({session})=>{
 const result=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__inputs,out:any[]=[];
 for(const depth of [8,16,32]){if(!PM.GL.supportedColorDepths.includes(depth))continue;const{owner,map}=s.scene(depth);map.d.w=64;map.d.h=64;
  const original=s.render();const f=s.effect(owner,map,'displacementMap',{behavior:0,horizontal:0,vertical:8,maxHorizontal:8,expand:true});const shifted=s.render();
  f.p.horizontal.v=8;PM.touch();const neutral=s.render();f.p.horizontal.v=0;f.p.wrap.v=true;f.p.expand.v=false;PM.touch();const wrap=s.render();
  out.push({depth,left:s.pixel(shifted,10,32),right:s.pixel(shifted,44,32),original:s.pixel(original,32,32),neutral:JSON.stringify(original)===JSON.stringify(neutral),wrap:s.pixel(wrap,44,32),outside:s.pixel(wrap,0,0)});
 }return out;});for(const r of result){expect(r.left).toEqual([255,0,0,255]);expect(r.right).toEqual([0,0,0,0]);expect(r.neutral).toBe(true);expect(r.wrap).toEqual([255,0,0,255]);expect(r.outside).toEqual([0,0,0,0]);}expect(session.diagnostics.pageErrors).toEqual([]);
});

test('reference edits reject cycles atomically and survive persistence, deletion/Undo, copy and None',async({session})=>{
 const r=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__inputs,{owner,map}=s.scene();const f=s.effect(owner,map);
  const before=s.render();PM.replaceProject(PM.hydrateProject(JSON.parse(JSON.stringify(PM.proj))));const roundtrip=s.render(),a=PM.L(owner.id),b=PM.L(map.id),saved=a.fx[0];
  const cycle=PM.Edit.apply({type:'set_effect',target:a.id,effect:saved.id,patch:{inputs:{map:{layerId:a.id,stage:'effects'}}}},{origin:'inspector'});
  const retained=PM.L(a.id).fx[0].inputs.map;
  const sourceSelf=PM.Edit.apply({type:'set_effect',target:a.id,effect:saved.id,patch:{inputs:{map:{layerId:a.id,stage:'source'}}}},{origin:'inspector'});
  PM.Edit.apply({type:'set_effect',target:a.id,effect:saved.id,patch:{inputs:{map:{layerId:b.id,stage:'source'}}}},{origin:'inspector'});
  PM.selectLayers([a.id,b.id]);PM.cmd('duplicate');const copies=PM.proj.layers.filter((l:any)=>![a.id,b.id].includes(l.id)),copyOwner=copies.find((l:any)=>l.fx.length),copyMap=copies.find((l:any)=>!l.fx.length);
  PM.Edit.apply({type:'delete_layers',targets:[b.id]},{origin:'inspector'});let missing='';try{s.render();}catch(e:any){missing=e.message;}PM.hist.undo();
  const restored=s.render();const none=PM.Edit.apply({type:'set_effect',target:a.id,effect:saved.id,patch:{inputs:{map:{layerId:null,stage:'source'}}}},{origin:'inspector'});s.render();
  return{same:JSON.stringify(before)===JSON.stringify(roundtrip),cycle,retained,sourceSelf,remap:copyOwner.fx[0].inputs.map.layerId===copyMap.id,missing,restored:restored.some((v:number)=>v>0),none};
 });expect(r.same).toBe(true);expect(r.cycle.ok).toBe(false);expect(r.cycle.message).toContain('cycle');expect(r.retained.stage).toBe('source');expect(r.sourceSelf.ok).toBe(true);expect(r.remap).toBe(true);expect(r.missing).toContain('missing input layer');expect(r.restored).toBe(true);expect(r.none.ok).toBe(true);expect(session.diagnostics.pageErrors).toEqual([]);
});

test('native layer controls and Ramp point fields fit a narrow Inspector in both themes',async({session},info)=>{
 await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__inputs,{owner,map}=s.scene();s.effect(owner,map);owner.fx.push(PM.mkEffect('gradient',undefined,owner));PM.selectLayers(owner.id);for(const f of owner.fx)PM.UIState.setFxOpen(f,true);PM.agentFrameCapture=false;PM.touch();PM.bus.emit('layers');PM.invalidate();});
 const inspector=session.page.locator('#panel-layer-effects');await expect(inspector.getByRole('combobox',{name:'Take Matte From Layer',exact:true})).toBeAttached();
 await inspector.locator('[data-color-key="map"] select').nth(0).selectOption({label:'None'}, {force:true});
 expect(await session.page.evaluate(()=>(window as any).PM.proj.layers[0].fx[0].inputs.map.layerId)).toBeNull();
 await inspector.locator('[data-color-key="map"] select').nth(0).selectOption({label:'Hidden map'}, {force:true});
 await inspector.locator('[data-color-key="map"] select').nth(1).selectOption({label:'Masks'}, {force:true});
 expect(await session.page.evaluate(()=>(window as any).PM.proj.layers[0].fx[0].inputs.map.stage)).toBe('masks');
 for(const theme of ['light','dark']){
  await session.page.evaluate(theme=>{const PM=(window as any).PM;PM.theme.apply(theme);const el=document.querySelector<HTMLElement>('#panel-layer-effects')!;el.style.width='260px';el.style.maxWidth='260px';el.style.minWidth='260px';},theme);
  const geometry=await inspector.locator('.point-field').evaluateAll(elements=>elements.map(el=>{const box=el.getBoundingClientRect();return{width:box.width,overflow:el.scrollWidth-el.clientWidth,clipped:[...el.querySelectorAll('input')].map(input=>input.scrollWidth-input.clientWidth),children:[...el.querySelectorAll('input,button,.well')].map(c=>{const b=c.getBoundingClientRect();return{left:b.left-box.left,right:b.right-box.right,width:b.width};})};}));
  await info.attach(`point-geometry-${theme}`,{body:JSON.stringify(geometry),contentType:'application/json'});await inspector.screenshot({path:info.outputPath(`layer-input-inspector-${theme}.png`)});
  expect(geometry.length).toBeGreaterThan(0);for(const field of geometry){expect(field.overflow).toBeLessThanOrEqual(1);for(const overflow of field.clipped)expect(overflow).toBeLessThanOrEqual(1);for(const child of field.children){expect(child.left).toBeGreaterThanOrEqual(-1);expect(child.right).toBeLessThanOrEqual(1);expect(child.width).toBeGreaterThan(4);}}
  await inspector.locator('.point-field').first().scrollIntoViewIfNeeded();await inspector.screenshot({path:info.outputPath(`ramp-points-${theme}.png`)});
 }expect(session.diagnostics.pageErrors).toEqual([]);
});

test('input surfaces are released on budget failure, cancellation and context-loss diagnostics',async({session})=>{
 const r=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__inputs,{owner,map}=s.scene();s.effect(owner,map);s.render();const gl=PM.GL.gl,budget=PM.Memory.budget,draw=gl.drawArrays;
  const baseline=PM.GL.pool.filter((f:any)=>f.busy).length;let budgetError='',cancelError='',lossError='';
  PM.Memory.budget=(kind:string)=>kind==='framebuffers'?1:budget.call(PM.Memory,kind);
  try{s.render();}catch(e:any){budgetError=e.message;}finally{PM.Memory.budget=budget;}
  const controller=new AbortController();gl.drawArrays=function(...args:any[]){const result=draw.apply(this,args);controller.abort();return result;};
  try{PM.GL.renderToPixels(0,64,64,{transparent:true,mblur:false,signal:controller.signal});}catch(e:any){cancelError=e.message;}finally{gl.drawArrays=draw;}
  gl.drawArrays=function(...args:any[]){const result=draw.apply(this,args);PM.GL.contextLost=true;return result;};
  try{s.render();}catch(e:any){lossError=e.message;}finally{gl.drawArrays=draw;PM.GL.contextLost=false;}
  const recovered=s.render();return{budgetError,cancelError,lossError,baseline,busy:PM.GL.pool.filter((f:any)=>f.busy).length,recovered:recovered.some((v:number)=>v>0)};
 });expect(r.budgetError).toContain('memory budget');expect(r.cancelError).toContain('cancelled');expect(r.lossError).toContain('context lost');expect(r.busy).toBe(r.baseline);expect(r.recovered).toBe(true);expect(session.diagnostics.pageErrors).toEqual([]);
});

test('float layer inputs preserve signed HDR color and fractional alpha through transformed sampling',async({session})=>{
 const r=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__inputs,gl=PM.GL.gl,out:any[]=[];
  for(const depth of [16,32]){if(!PM.GL.supportedColorDepths.includes(depth))continue;const{owner,map}=s.scene(depth);owner.type='shader';owner.d={w:32,h:32,code:'void main(){fragColor=vec4(-.25,2.,.5,1.);}'};owner.p['position.x'].v=16;owner.p['position.y'].v=16;map.p.opacity.v=50;
   for(const type of ['setMatte','displacementMap']){s.effect(owner,map,type,type==='displacementMap'?{horizontal:8,vertical:8}:{});let sample:number[]=[];const read=gl.readPixels;gl.readPixels=function(...args:any[]){read.apply(this,args);if(args[5]===gl.FLOAT&&args[6] instanceof Float32Array)sample=[...args[6].slice((32*64+32)*4,(32*64+32)*4+4)];};try{s.render();}finally{gl.readPixels=read;}out.push({depth,type,sample});}
  }return out;
 });expect(r.length).toBeGreaterThan(0);for(const result of r){const expected=result.type==='setMatte'?[-.125,1,.25,.5]:[-.25,2,.5,1];for(let c=0;c<4;c++)expect(Math.abs(result.sample[c]!-expected[c]!)).toBeLessThan(.003);}expect(session.diagnostics.pageErrors).toEqual([]);
});

test('hidden referenced video is prepared for reverse seeks and native export uses the general frame path',async({session})=>{
 test.setTimeout(60_000);
 const root=await mkdtemp(path.join(os.tmpdir(),'powermove-layer-video-'));
 try{
  const file=path.join(root,'map.webm');const raw=Buffer.concat([32,96,160,224].map(v=>Buffer.alloc(32*32*3,v)));
  execFileSync(path.join(repoRoot,'node_modules/ffmpeg-static/ffmpeg'),['-v','error','-f','rawvideo','-pix_fmt','rgb24','-s','32x32','-r','4','-i','pipe:0','-c:v','libvpx-vp9','-lossless','1',file],{input:raw});
  await session.app.evaluate(({ipcMain})=>{const state={chunks:[] as number[][],started:0,finished:false};(globalThis as any).__inputExport=state;for(const n of ['start','write','finish','cancel'])ipcMain.removeHandler(`render:${n}`);ipcMain.handle('render:start',()=>{state.started++;return'layer-input';});ipcMain.handle('render:write',(_e,args)=>{state.chunks.push([...args.data]);});ipcMain.handle('render:finish',()=>{state.finished=true;return{path:'/test/layer-input'};});ipcMain.handle('render:cancel',()=>{});});
  const r=await session.page.evaluate(async(bytes)=>{const PM=(window as any).PM,s=(window as any).__inputs,{owner,map}=s.scene();PM.proj.dur=.5;PM.proj.fps=4;owner.dur=.5;map.dur=.5;
   const asset=await PM.assets.add(new File([new Uint8Array(bytes)],'map.webm',{type:'video/webm'}),{silent:true});map.type='video';map.d={asset:asset.id,w:32,h:32,speed:-1,trim:.75,embeddedAudio:false};map.on=false;
   const f=s.effect(owner,map,'setMatte',{channel:4}),samples:any[]=[],expected:number[]=[];
   for(const time of [0,.25,0]){await PM.prepareFrame(time);const p=PM.GL.renderToPixels(time,64,64,{opaque:true,topDownOpaque:true,mblur:false});samples.push({time,prepared:PM.preparedVideoFrames.has(`${map.id}@${time}`),pixel:[...p.slice((32*64+32)*4,(32*64+32)*4+4)]});if(samples.length<3)expected.push(...p);}
   let optimized=0;const factory=PM.GL.createOpaqueReadback;PM.GL.createOpaqueReadback=(...args:any[])=>{optimized++;return factory(...args);};let result;try{result=await PM.Export.run({format:'mp4',alpha:false,scale:1,fps:4,range:'all',quality:'draft',audio:false,mblur:false});}finally{PM.GL.createOpaqueReadback=factory;}
   f.inputs.map.layerId='deleted-map';PM.touch();const missing=await PM.Export.run({format:'mp4',alpha:false,scale:1,fps:4,range:'all',quality:'draft',audio:false,mblur:false});return{samples,expected,result,optimized,missing};
  },[...await readFile(file)]);
  const delivered=await session.app.evaluate(()=>(globalThis as any).__inputExport);
  expect(r.result).toEqual({cancelled:false});expect(delivered.started).toBe(1);expect(delivered.finished).toBe(true);expect(delivered.chunks.flat()).toEqual(r.expected);expect(r.optimized).toBe(0);expect(r.missing.error).toContain('missing input layer');
  for(const sample of r.samples){expect(sample.prepared).toBe(true);expect(Math.abs(sample.pixel[0]-(sample.time===0?224:160))).toBeLessThan(5);expect(sample.pixel.slice(1)).toEqual([0,0,255]);}expect(session.diagnostics.pageErrors).toEqual([]);
 }finally{await rm(root,{recursive:true,force:true});}
});

test('native export sees stage, source and provider changes after each held encoder acknowledgement',async({session})=>{
 test.setTimeout(60_000);
 await session.app.evaluate(({ipcMain})=>{const state={chunks:[] as number[][],held:0};(globalThis as any).__inputAck=state;for(const n of ['start','write','finish','cancel'])ipcMain.removeHandler(`render:${n}`);ipcMain.handle('render:start',()=> 'input-ack');ipcMain.handle('render:write',async(_e,args)=>{state.chunks.push([...args.data]);if(state.chunks.length<=3){state.held=state.chunks.length;await new Promise<void>(resolve=>{(globalThis as any).__releaseInputAck=resolve;});state.held=0;}});ipcMain.handle('render:finish',()=>({path:'/test/input-ack'}));ipcMain.handle('render:cancel',()=>{});});
 await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__inputs,{owner,map}=s.scene(8);PM.proj.dur=1;PM.proj.fps=4;owner.dur=map.dur=1;const f=s.effect(owner,map);const mask=PM.mkMask('rect',PM.proj);mask.p.w.v=8;mask.p.h.v=16;mask.p.feather.v=0;map.masks=[mask];PM.touch();
  const options={opaque:true,topDownOpaque:true,mblur:false};(window as any).__ackExpected=[...PM.GL.renderToPixels(0,64,64,options)];(window as any).__ackIds={owner:owner.id,map:map.id,effect:f.id};(window as any).__inputAckJob=PM.Export.run({format:'mp4',alpha:false,scale:1,fps:4,range:'all',quality:'draft',audio:false,mblur:false});
 });
 try{for(const checkpoint of [1,2,3]){
   await expect.poll(()=>session.app.evaluate(()=>(globalThis as any).__inputAck.held)).toBe(checkpoint);
   await session.page.evaluate(checkpoint=>{const PM=(window as any).PM,ids=(window as any).__ackIds,owner=PM.L(ids.owner),map=PM.L(ids.map);
    if(checkpoint===1){const result=PM.Edit.apply({type:'set_effect',target:owner.id,effect:ids.effect,patch:{inputs:{map:{layerId:map.id,stage:'masks'}}}},{origin:'inspector'});if(!result.ok)throw Error(result.message);}
    else if(checkpoint===2){map.masks[0].p.w.v=16;map.p.opacity.v=50;PM.touch();}
    else{const def=PM.Kernel.effects.get('setMatte');PM.Kernel.registerEffect('input-ack-provider',{...def,frag:def.frag.replace('a=clamp(a,0.,1.);','a=clamp(a*.25,0.,1.);')});PM.touch();}
    (window as any).__ackExpected.push(...PM.GL.renderToPixels(checkpoint/4,64,64,{opaque:true,topDownOpaque:true,mblur:false}));
   },checkpoint);
   await session.app.evaluate(()=>(globalThis as any).__releaseInputAck());
  }
  expect(await session.page.evaluate(()=>(window as any).__inputAckJob)).toEqual({cancelled:false});
  const expected=await session.page.evaluate(()=>(window as any).__ackExpected),chunks=await session.app.evaluate(()=>(globalThis as any).__inputAck.chunks);
  expect(chunks.flat()).toEqual(expected);expect(new Set(chunks.map((p:number[])=>p.reduce((sum,v)=>sum+v,0))).size).toBe(4);expect(session.diagnostics.pageErrors).toEqual([]);
 }finally{await session.app.evaluate(()=>(globalThis as any).__releaseInputAck?.());}
});


test('hidden group inputs keep their bounds, child mattes and real local adjustment stack',async({session})=>{
 const r=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__inputs,{owner,map}=s.scene();
  const group=PM.mkLayer('group',{name:'Hidden group',p:{'position.x':0,'position.y':0}},PM.proj);group.on=false;map.group=group.id;map.on=true;PM.proj.layers=[owner,group,map];
  const f=s.effect(owner,group);const source=s.render();
  const matte=PM.mkLayer('shape',{name:'Child matte',d:{w:8,h:16,radius:0,color:'#FFFFFF'},p:{'position.x':32,'position.y':32}},PM.proj);matte.group=group.id;map.matteSource=matte.id;PM.proj.layers.push(matte);PM.touch();const masked=s.render();
  const adjustment=PM.mkLayer('adjustment',{name:'Group adjustment'},PM.proj);adjustment.group=group.id;adjustment.fx=[PM.mkEffect('invert',undefined,adjustment)];adjustment.fx[0].p.channel.v=0;f.p.channel.v=4;PM.proj.layers.splice(2,0,adjustment);PM.touch();const adjusted=s.render();
  const result={source:s.pixel(source,20,32),center:s.pixel(masked,32,32),edge:s.pixel(masked,20,32),adjusted:s.pixel(adjusted,32,32)};
  f.inputs.map.stage='effects';group.fx=[PM.mkEffect('blur',undefined,group)];group.fx[0].p.amount.v=0;PM.touch();s.render();return result;
 });expect(r.source).toEqual([255,0,0,255]);expect(r.center).toEqual([255,0,0,255]);expect(r.edge).toEqual([0,0,0,0]);expect(r.adjusted).toEqual([0,0,0,0]);expect(session.diagnostics.pageErrors).toEqual([]);
});


test('real native H264 output preserves the rendered map-effect frames',async({session})=>{
 test.setTimeout(60_000);const output=path.join(session.userData,'layer-effects.mp4');
 await session.app.evaluate(({dialog},file)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath:file});},output);
 const r=await session.page.evaluate(async()=>{const PM=(window as any).PM,s=(window as any).__inputs,{owner,map}=s.scene(8);PM.proj.dur=1;PM.proj.fps=4;owner.dur=map.dur=1;map.p.opacity.v=75;
  const matte=s.effect(owner,map,'setMatte',{stretch:false}),displacement=PM.mkEffect('displacementMap',undefined,owner);displacement.inputs.map={layerId:map.id,stage:'source'};displacement.p.maxHorizontal.kf=[PM.KF(0,0),PM.KF(1,8)];displacement.p.vertical.v=8;owner.fx.push(displacement);PM.touch();
  const expected:number[]=[];for(const time of [0,.25,.5,.75]){await PM.prepareFrame(time);expected.push(...PM.GL.renderToPixels(time,64,64,{opaque:true,topDownOpaque:true,mblur:false}));}
  const result=await PM.Export.run({format:'mp4',alpha:false,scale:1,fps:4,range:'all',quality:'high',audio:false,mblur:false});return{result,expected};
 });expect(r.result).toEqual({cancelled:false});
 const pixels=execFileSync(path.join(repoRoot,'node_modules/ffmpeg-static/ffmpeg'),['-v','error','-i',output,'-pix_fmt','rgba','-f','rawvideo','pipe:1']);expect(pixels.length).toBe(r.expected.length);
 const error=pixels.reduce((sum,v,i)=>sum+Math.abs(v-r.expected[i]!),0)/pixels.length;expect(error).toBeLessThan(4);expect(pixels.subarray(0,64*64*4).equals(pixels.subarray(-64*64*4))).toBe(false);expect(session.diagnostics.pageErrors).toEqual([]);
});
