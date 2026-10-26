import {expect,test} from './helpers/app';

test.beforeEach(async({session})=>{await session.openEditor();await session.page.evaluate(()=>{
 const PM=(window as any).PM;PM.pause();PM.agentFrameCapture=true;PM.quality=1;PM.AutomaticPreview?.cancel();
 (window as any).__refinement={
  scene(depth=16,w=16,h=16){const p=PM.mkProject({name:'Shadow and Ramp',w:64,h:64,dur:2});p.colorDepth=depth;p.backgroundFill={type:'none',stops:[],angle:0};const l=PM.mkLayer('shape',{d:{w,h,radius:0,color:'#FF0000'},p:{'position.x':24,'position.y':24}},p);p.layers=[l];PM.replaceProject(p);return l;},
  fx(l:any,type:string,values:Record<string,unknown>={},version?:number){const fx=PM.mkEffect(type,version,l);for(const[k,v]of Object.entries(values))fx.p[k].v=v;l.fx=[fx];PM.touch();return fx;},
  render(size=64){const p=PM.GL.renderToPixels(0,size,size,{transparent:true,mblur:false});if(PM.GL.errors.size)throw Error(JSON.stringify([...PM.GL.errors]));return [...p];},
  pixel(p:number[],x:number,y:number,size=64){return p.slice(((size-1-y)*size+x)*4,((size-1-y)*size+x)*4+4);},
 };
});});

test('Drop Shadow has exact zero-softness offsets, Shadow Only, source alpha and neutral opacity',async({session},testInfo)=>{
 const results=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__refinement,out:any[]=[];
  for(const depth of [8,16,32]){if(!PM.GL.supportedColorDepths.includes(depth))continue;
   const l=s.scene(depth),source=s.render();
   s.fx(l,'shadow',{angle:90,distance:16,softness:0,opacity:100,shadowOnly:true,color:'#FFFFFF'});const shifted=s.render();
   let error=0;for(let y=0;y<64;y++)for(let x=0;x<64;x++){const expected=x>=16?s.pixel(source,x-16,y)[3]:0;error=Math.max(error,Math.abs(s.pixel(shifted,x,y)[3]-expected));}
   s.fx(l,'shadow',{angle:90,distance:16,softness:0,opacity:50});const combined=s.render();
   s.fx(l,'shadow',{angle:0,distance:16,softness:0,opacity:50,shadowOnly:true,color:'#00FF00'});const up=s.render();
   s.fx(l,'shadow',{opacity:0});const neutral=s.render();
   s.fx(l,'shadow',{opacity:0,shadowOnly:true});const empty=s.render();
   out.push({depth,error,source:s.pixel(combined,24,24),shadow:s.pixel(combined,40,24),up:s.pixel(up,24,8),outside:s.pixel(up,0,0),neutral:JSON.stringify(neutral)===JSON.stringify(source),empty:empty.every((v:number)=>v===0)});
  }return out;});
 for(const r of results){expect(r.error).toBeLessThanOrEqual(1);expect(r.source).toEqual([255,0,0,255]);expect(r.shadow).toEqual([0,0,0,128]);expect(r.up).toEqual([0,128,0,128]);expect(r.outside).toEqual([0,0,0,0]);expect(r.neutral).toBe(true);expect(r.empty).toBe(true);}
 await testInfo.attach('shadow-offset-results',{body:JSON.stringify(results,null,2),contentType:'application/json'});expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Drop Shadow direction and softness follow rotated and scaled layer coordinates',async({session})=>{
 const result=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__refinement,l=s.scene();
  s.fx(l,'shadow',{angle:90,distance:16,opacity:50,shadowOnly:true,color:'#00FF00'});l.p.rotation.v=90;PM.touch();const rotated=s.render();
  l.p.rotation.v=0;l.p['scale.x'].v=200;PM.touch();const scaled=s.render();
  l.p['scale.x'].v=100;s.fx(l,'shadow',{distance:0,softness:8,opacity:100,shadowOnly:true,color:'#FFFFFF'});const soft=s.render();
  return{down:s.pixel(rotated,24,40),right:s.pixel(rotated,40,24),scaled:s.pixel(scaled,56,24),softCenter:s.pixel(soft,24,24)[3],softOutside:s.pixel(soft,34,24)[3],far:s.pixel(soft,48,24)[3]};
 });expect(result.down).toEqual([0,128,0,128]);expect(result.right).toEqual([0,0,0,0]);expect(result.scaled).toEqual([0,128,0,128]);expect(result.softCenter).toBeGreaterThan(220);expect(result.softOutside).toBeGreaterThan(5);expect(result.softOutside).toBeLessThan(100);expect(result.far).toBe(0);expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Gradient Ramp resolves source-sized points and renders linear, radial, degenerate and original-blend modes',async({session},testInfo)=>{
 const results=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__refinement,out:any[]=[];
  for(const depth of [8,16,32]){if(!PM.GL.supportedColorDepths.includes(depth))continue;
   const l=s.scene(depth,32,16);l.p['position.x'].v=32;l.p['position.y'].v=32;PM.touch();const source=s.render(),fx=s.fx(l,'gradient'),vertical=s.render();
   const defaults=[fx.p.startX.v,fx.p.startY.v,fx.p.endX.v,fx.p.endY.v];
   s.fx(l,'gradient',{startX:0,startY:0,endX:32,endY:0,startColor:'#FF0000',endColor:'#0000FF'});const horizontal=s.render();
   s.fx(l,'gradient',{startX:16,startY:8,endX:24,endY:8,rampShape:1,startColor:'#FF0000',endColor:'#0000FF'});const radial=s.render();
   s.fx(l,'gradient',{startX:3,startY:7,endX:3,endY:7,startColor:'#00FF00'});const degenerate=s.render();
   s.fx(l,'gradient',{blendWithOriginal:100});const bypass=s.render();
   out.push({depth,defaults,top:s.pixel(vertical,32,25),bottom:s.pixel(vertical,32,38),horizontal:s.pixel(horizontal,20,32),radial:s.pixel(radial,32,32),far:s.pixel(radial,45,32),degenerate:s.pixel(degenerate,32,32),outside:s.pixel(horizontal,0,0),bypass:JSON.stringify(source)===JSON.stringify(bypass)});
  }return out;});
 for(const r of results){expect(r.defaults).toEqual([16,0,16,16]);for(const [actual,expected]of [[r.top,[24,24,24,255]],[r.bottom,[231,231,231,255]],[r.horizontal,[219,0,36,255]],[r.radial,[232,0,23,255]]] as const)for(let c=0;c<4;c++)expect(Math.abs(actual[c]!-expected[c]!)).toBeLessThanOrEqual(1);expect(r.far).toEqual([0,0,255,255]);expect(r.degenerate).toEqual([0,255,0,255]);expect(r.outside).toEqual([0,0,0,0]);expect(r.bypass).toBe(true);}
 await testInfo.attach('ramp-independent-results',{body:JSON.stringify(results,null,2),contentType:'application/json'});expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Ramp points stay in layer pixels through transforms, hydration and source resizing',async({session})=>{
 const result=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__refinement,l=s.scene(16,32,16);l.p['position.x'].v=32;l.p['position.y'].v=32;
  const fx=s.fx(l,'gradient',{startX:0,startY:0,endX:32,endY:0,startColor:'#FF0000',endColor:'#0000FF'}),samples:any[]=[];
  for(const[rotation,scale]of [[0,100],[90,100],[0,200]]){l.p.rotation.v=rotation;l.p['scale.x'].v=scale;l.p['scale.y'].v=scale;PM.touch();const p=s.render(),m=PM.worldMatrix(l,0),b=PM.GL.bounds(l,0),lx=b.x0+8,ly=b.y0+8,x=Math.floor(m[0]*lx+m[2]*ly+m[4]),y=Math.floor(m[1]*lx+m[3]*ly+m[5]);const det=m[0]*m[3]-m[1]*m[2];const localX=(m[3]*(x+.5-m[4])-m[2]*(y+.5-m[5]))/det-b.x0;const t=Math.max(0,Math.min(1,localX/32));samples.push({actual:s.pixel(p,x,y),expected:[255*(1-t),0,255*t,255]});}
  fx.p.endY.v=16;PM.touch();const before=s.render();delete fx.p.endY;PM.replaceProject(PM.hydrateProject(JSON.parse(JSON.stringify(PM.proj))));const after=s.render(),loaded=PM.proj.layers[0];
  loaded.d.w=48;loaded.d.h=24;PM.touch();
  return{samples,same:JSON.stringify(before)===JSON.stringify(after),repaired:loaded.fx[0].p.endY.v,saved:loaded.fx[0].p.endX.v,defaultFrom:PM.Kernel.effects.get('gradient').params[0].defaultFrom};
 });for(const r of result.samples)for(let c=0;c<4;c++)expect(Math.abs(r.actual[c]-r.expected[c])).toBeLessThan(2);expect(result.same).toBe(true);expect(result.repaired).toBe(16);expect(result.saved).toBe(32);expect(result.defaultFrom).toEqual({dimension:'width',factor:.5});expect(session.diagnostics.pageErrors).toEqual([]);
});

test('old/new Shadow and Ramp coexist with native controls and source-sized reset defaults',async({session},testInfo)=>{
 const result=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__refinement,l=s.scene(16,32,16);const results:any[]=[];
  for(const type of ['shadow','gradient']){l.fx=[PM.mkEffect(type,1,l),PM.mkEffect(type,2,l)];PM.touch();const before=s.render();PM.replaceProject(PM.hydrateProject(JSON.parse(JSON.stringify(PM.proj))));const after=s.render(),loaded=PM.proj.layers[0];results.push({type,same:JSON.stringify(before)===JSON.stringify(after),versions:loaded.fx.map((f:any)=>f.version)});Object.assign(l,loaded);PM.proj.layers=[l];}
  PM.agentFrameCapture=false;return results;
 });for(const r of result){expect(r.same).toBe(true);expect(r.versions).toEqual([1,2]);}
 for(const type of ['shadow','gradient']){
  const id=await session.page.evaluate(type=>{const PM=(window as any).PM,p=PM.mkProject({name:'Shadow and Ramp controls',w:640,h:360,dur:2}),l=PM.mkLayer('shape',{d:{w:180,h:100,color:'#E95A2B'},p:{'position.x':320,'position.y':180}},p);p.layers=[l];PM.replaceProject(p);const fx=PM.mkEffect(type,undefined,l);l.fx=[fx];PM.selectLayers(l.id);PM.UIState.setFxOpen(fx,true);PM.touch();PM.bus.emit('layers');PM.invalidate();return fx.id;},type);
  await expect(session.page.locator(`[data-effect-id="${id}"]`)).toBeVisible();
  if(type==='gradient')await expect(session.page.getByRole('button',{name:'Swap Colors',exact:true})).toBeVisible();
  for(const theme of ['light','dark']){await session.page.evaluate(theme=>{const PM=(window as any).PM;PM.theme.apply(theme);PM.invalidate('render');},theme);await session.page.waitForFunction(()=>{const PM=(window as any).PM;return !PM.GL.compiling&&PM.GL.errors.size===0;});await session.page.screenshot({path:testInfo.outputPath(`refinement-${type}-${theme}.png`)});}
 }expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Drop Shadow projects wide support and skips mip preparation on the offset pass',async({session})=>{
 const results=await session.page.evaluate(()=>{const PM=(window as any).PM,s=(window as any).__refinement,gl=PM.GL.gl,l=s.scene(),out:any[]=[];
  s.fx(l,'shadow',{distance:0,softness:32,opacity:100,shadowOnly:true});
  const generate=gl.generateMipmap.bind(gl);let calls=0;gl.generateMipmap=(...args:any[])=>{calls++;return generate(...args);};
  try{for(const scale of [100,200]){l.p['scale.x'].v=scale;l.p['scale.y'].v=scale;PM.touch();calls=0;s.render();const pr=PM.GL.progs.get('fx:shadow').pr;out.push({scale,calls,lod:gl.getUniform(pr,gl.getUniformLocation(pr,'u_sampleLod'))});}}
  finally{gl.generateMipmap=generate;}return out;
 });expect(results).toEqual([{scale:100,calls:2,lod:1},{scale:200,calls:2,lod:2}]);expect(session.diagnostics.pageErrors).toEqual([]);
});
