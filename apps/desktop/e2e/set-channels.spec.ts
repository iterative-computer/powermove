import {expect,test} from './helpers/app';

test('Set Channels schedules four hidden sources and preserves channel, alpha, stage and saved-reference semantics',async({session},info)=>{
 await session.openEditor();const result=await session.page.evaluate(async()=>{const PM=(window as any).PM;PM.pause();PM.agentFrameCapture=true;PM.quality=1;PM.AutomaticPreview?.cancel();const out:any[]=[];
  for(const depth of [8,16,32]){if(!PM.GL.supportedColorDepths.includes(depth))continue;const p=PM.mkProject({name:'Four channel sources',w:32,h:32,dur:1});p.colorDepth=depth;p.backgroundFill={type:'none',stops:[],angle:0};
   const owner=PM.mkLayer('shader',{name:'Channel owner',d:{w:32,h:32,code:'void main(){fragColor=vec4(.1,.2,.3,1.);}'}},p),sources=['vec4(1.,0.,0.,1.)','vec4(0.,.5,0.,1.)','vec4(0.,0.,.25,1.)','vec4(1.)'].map((color,i)=>{const l=PM.mkLayer('shader',{name:'Source '+i,d:{w:32,h:32,code:`void main(){fragColor=${color};}`}},p);l.on=false;if(i===3)l.p.opacity.v=50;return l;});p.layers=[owner,...sources];PM.replaceProject(p);
   const f=PM.mkEffect('setChannels',undefined,owner);for(const[key,i]of ['red','green','blue','alpha'].map((key,i)=>[key,i]))f.inputs[key+'Layer']={layerId:sources[i].id,stage:'source'};owner.fx=[f];PM.touch();
   const render=()=>[...PM.GL.renderToPixels(0,32,32,{transparent:true,mblur:false})],pixel=(p:number[],x=16)=>p.slice((16*32+x)*4,(16*32+x)*4+4),base=render();
   const inversion=PM.mkEffect('invert',undefined,sources[2]);inversion.p.channel.v=3;sources[2].fx=[inversion];f.inputs.blueLayer.stage='effects';PM.touch();const effected=render();
   const mask=PM.mkMask('rect',p);mask.p.x.v=16;mask.p.y.v=16;mask.p.w.v=16;mask.p.h.v=32;mask.p.feather.v=0;sources[3].masks=[mask];f.inputs.alphaLayer.stage='masks';PM.touch();const masked=render();
   const snapshot=JSON.parse(JSON.stringify(PM.proj));PM.replaceProject(PM.hydrateProject(snapshot));const restored=render();
   const loaded=PM.proj.layers[0],ids=new Set(PM.proj.layers.map((l:any)=>l.id));PM.selectLayers([...ids]);PM.cmd('duplicate');const copy=PM.proj.layers.find((l:any)=>!ids.has(l.id)&&l.fx?.[0]?.type==='setChannels');
   out.push({depth,base:pixel(base),effected:pixel(effected),masked:pixel(masked,4),center:pixel(masked),roundtrip:JSON.stringify(masked)===JSON.stringify(restored),remapped:Object.values<any>(copy.fx[0].inputs).every(ref=>!ids.has(ref.layerId)),errors:[...PM.GL.errors]});
  }return out;});for(const r of result){expect(r.base).toEqual([128,64,32,128]);expect(r.effected).toEqual([128,64,96,128]);expect(r.masked).toEqual([0,0,0,0]);expect(r.center).toEqual([128,64,96,128]);expect(r.roundtrip).toBe(true);expect(r.remapped).toBe(true);expect(r.errors).toEqual([]);}await info.attach('four-source-channel-pixels',{body:JSON.stringify(result,null,2),contentType:'application/json'});expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Set Channels preserves signed HDR source values and defaults to the current image with None inputs',async({session})=>{
 await session.openEditor();const result=await session.page.evaluate(()=>{const PM=(window as any).PM;PM.pause();PM.agentFrameCapture=true;PM.quality=1;PM.AutomaticPreview?.cancel();const out:any[]=[];
  for(const depth of [16,32]){if(!PM.GL.supportedColorDepths.includes(depth))continue;const p=PM.mkProject({w:16,h:16,dur:1});p.colorDepth=depth;p.backgroundFill={type:'none',stops:[],angle:0};const owner=PM.mkLayer('shader',{d:{w:16,h:16,code:'void main(){fragColor=vec4(2.,-.5,.25,1.);}'}},p),alpha=PM.mkLayer('shader',{d:{w:16,h:16,code:'void main(){fragColor=vec4(.5);}'}},p);alpha.on=false;p.layers=[owner,alpha];PM.replaceProject(p);const f=PM.mkEffect('setChannels',undefined,owner);owner.fx=[f];PM.touch();const gl=PM.GL.gl,read=gl.readPixels;let value:number[]=[];gl.readPixels=function(...args:any[]){read.apply(this,args);if(args[5]===gl.FLOAT&&args[6] instanceof Float32Array)value=[...args[6].slice((8*16+8)*4,(8*16+8)*4+4)];};try{PM.GL.renderToPixels(0,16,16,{transparent:true,mblur:false});const neutral=value;f.inputs.alphaLayer={layerId:alpha.id,stage:'source'};PM.touch();PM.GL.renderToPixels(0,16,16,{transparent:true,mblur:false});out.push({depth,neutral,alpha:value});}finally{gl.readPixels=read;}
  }return out;});expect(result.length).toBeGreaterThan(0);for(const r of result){for(const[values,expected]of [[r.neutral,[2,-.5,.25,1]],[r.alpha,[1,-.25,.125,.5]]])for(let i=0;i<4;i++)expect(Math.abs(values[i]-expected[i])).toBeLessThan(.003);}expect(session.diagnostics.pageErrors).toEqual([]);
});


test('four-source channel controls use the native grouped layout in both themes',async({session},info)=>{
 await session.openEditor();await session.page.evaluate(()=>{const PM=(window as any).PM;PM.pause();PM.agentFrameCapture=true;PM.quality=1;PM.AutomaticPreview?.cancel();const p=PM.mkProject({name:'Channel controls',w:32,h:32,dur:1}),owner=PM.mkLayer('shape',{name:'Channel owner',d:{w:32,h:32,color:'#FF0000'}},p),source=PM.mkLayer('shape',{name:'Hidden channels',d:{w:32,h:32,color:'#FFFFFF'}},p);source.on=false;p.layers=[owner,source];PM.replaceProject(p);const f=PM.mkEffect('setChannels',undefined,owner);owner.fx=[f];PM.selectLayers(owner.id);PM.UIState.setFxOpen(f,true);PM.touch();PM.bus.emit('layers');PM.invalidate();});
 const panel=session.page.locator('#panel-layer-effects');
 for(const channel of ['Red','Green','Blue','Alpha']){await panel.locator('summary').filter({hasText:new RegExp('^'+channel)}).click();await expect(panel.getByRole('combobox',{name:channel+' Layer',exact:true})).toBeVisible();await expect(panel.getByRole('combobox',{name:channel+' Channel',exact:true})).toBeVisible();}
 await panel.getByRole('combobox',{name:'Red Layer',exact:true}).click();await session.page.getByRole('option',{name:'Hidden channels',exact:true}).click();await expect(session.page.locator('.pm-menu')).toHaveCount(0);
 const ref=await session.page.evaluate(()=>{const PM=(window as any).PM;return PM.proj.layers[0].fx[0].inputs.redLayer;});expect(ref.stage).toBe('source');expect(ref.layerId).toBeTruthy();
 for(const theme of ['light','dark']){await session.page.evaluate(theme=>(window as any).PM.theme.apply(theme),theme);await panel.screenshot({path:info.outputPath(`channel-controls-${theme}.png`)});}
 expect(session.diagnostics.pageErrors).toEqual([]);
});
