import { expect, test } from './helpers/app';

test.beforeEach(async ({session}) => {
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM=(window as any).PM;
    PM.pause(); PM.agentFrameCapture=true; PM.quality=1; PM.AutomaticPreview?.cancel();
    (window as any).__spatial={
      scene(code:string, depth=16, size=64) {
        const project=PM.mkProject({name:'Spatial operators',w:size,h:size,dur:2});
        project.colorDepth=depth;project.backgroundFill={type:'none',stops:[],angle:0};
        const layer=PM.mkLayer('shader',{d:{w:size,h:size,code:'void main(){'+code+'}'},p:{'position.x':0,'position.y':0}},project);
        project.layers=[layer]; PM.replaceProject(project);return layer;
      },
      fx(layer:any,type:string,values:Record<string,unknown>={},version?:number) {
        const fx=PM.mkEffect(type,version); for(const [key,value] of Object.entries(values))fx.p[key].v=value;
        layer.fx=[fx];PM.touch();return fx;
      },
      render(size=64) {
        const pixels=PM.GL.renderToPixels(0,size,size,{transparent:true,mblur:false});
        if(PM.GL.errors.size)throw Error(JSON.stringify([...PM.GL.errors]));
        const error=PM.GL.gl.getError();if(error)throw Error('WebGL error '+error);
        return [...pixels];
      },
      pixel(p:number[],x:number,y:number,size=64){return p.slice((y*size+x)*4,(y*size+x)*4+4);},
    };
  });
});

test('Gaussian dimensions, repeat edges and scale follow independent convolution expectations',async({session},testInfo)=>{
  const results=await session.page.evaluate(()=>{
    const PM=(window as any).PM,s=(window as any).__spatial,results:any[]=[];
    for(const depth of [8,16,32]){
      if(!PM.GL.supportedColorDepths.includes(depth))continue;
      const layer=s.scene('float v=step(.5,uv.x);fragColor=vec4(vec3(v),1.);',depth);
      const source=s.render();s.fx(layer,'blur',{amount:4,dimensions:2,repeatEdgePixels:true});const vertical=s.render();
      s.fx(layer,'blur',{amount:4,dimensions:1,repeatEdgePixels:true});const horizontal=s.render();
      let weighted=0,total=0;for(let i=-6;i<=6;i++){const w=Math.exp(-.5*i*i/4);weighted+=s.pixel(source,31+i,32)[0]*w;total+=w;}
      results.push({name:'vertical preserves horizontal step',depth,actual:s.pixel(vertical,31,32),expected:s.pixel(source,31,32)});
      results.push({name:'horizontal Gaussian convolution',depth,actual:s.pixel(horizontal,31,32)[0],expected:weighted/total});
      s.fx(layer,'blur',{amount:0});results.push({name:'zero identity',depth,same:JSON.stringify(s.render())===JSON.stringify(source)});
      s.scene('fragColor=vec4(.2,.4,.6,1.);',depth);const full=PM.proj.layers[0];
      s.fx(full,'blur',{amount:20,repeatEdgePixels:true});const repeated=s.render();
      s.fx(full,'blur',{amount:20,repeatEdgePixels:false});const transparent=s.render();
      results.push({name:'edge extension',depth,repeat:s.pixel(repeated,0,32),transparent:s.pixel(transparent,0,32)});
      const step=s.scene('float v=step(.5,uv.x);fragColor=vec4(vec3(v),1.);',depth,128);
      s.fx(step,'blur',{amount:12,dimensions:1,repeatEdgePixels:true});const big=s.render(128),small=s.render(64);
      results.push({name:'quality scale',depth,big:s.pixel(big,60,64,128)[0],small:s.pixel(small,30,32)[0]});
    }
    return results;
  });
  for(const result of results){
    if(result.name==='zero identity')expect(result.same).toBe(true);
    else if(result.name==='edge extension'){expect(result.repeat[3]).toBe(255);expect(result.transparent[3]).toBeLessThan(180);expect(result.transparent[3]).toBeGreaterThan(100);}
    else if(result.name==='quality scale')expect(Math.abs(result.big-result.small)).toBeLessThan(12);
    else if(Array.isArray(result.actual))expect(result.actual).toEqual(result.expected);
    else expect(Math.abs(result.actual-result.expected)).toBeLessThan(2);
  }
  await testInfo.attach('Gaussian-independent-results',{body:JSON.stringify(results,null,2),contentType:'application/json'});
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Mosaic computes cell averages or centers in independent layer-local grids with premultiplied alpha',async({session},testInfo)=>{
  const results=await session.page.evaluate(()=>{
    const PM=(window as any).PM,s=(window as any).__spatial,results:any[]=[];
    for(const depth of [8,16,32]){
      if(!PM.GL.supportedColorDepths.includes(depth))continue;
      const layer=s.scene('float v=step(.125,uv.x);fragColor=vec4(v*.5,uv.y*.5,0.,.5);',depth);
      const source=s.render();
      for(const [columns,rows] of [[2,4],[4,2]]){
        s.fx(layer,'pixelate',{horizontalBlocks:columns,verticalBlocks:rows});const averaged=s.render();
        s.fx(layer,'pixelate',{horizontalBlocks:columns,verticalBlocks:rows,sharpColors:true});const sharp=s.render();
        for(let cy=0;cy<rows;cy++)for(let cx=0;cx<columns;cx++){
          const w=64/columns,h=64/rows,x=cx*w+Math.floor(w/2),y=cy*h+Math.floor(h/2),expected=[0,0,0,0];
          for(let dy=0;dy<h;dy++)for(let dx=0;dx<w;dx++){const p=s.pixel(source,cx*w+dx,cy*h+dy);for(let c=0;c<4;c++)expected[c]!+=p[c]!/(w*h);}
          results.push({depth,columns,rows,cell:[cx,cy],actual:s.pixel(averaged,x,y),expected});
        }
        results.push({depth,columns,sharp:s.pixel(sharp,8,8),average:s.pixel(averaged,8,8)});
      }
      layer.p['position.x'].v=32;layer.p['position.y'].v=32;layer.p.rotation.v=30;layer.d.w=32;layer.d.h=32;PM.touch();
      const transformed=s.render();results.push({depth,corner:s.pixel(transformed,0,0),occupied:transformed.some((v:number,i:number)=>i%4===3&&v>0)});
    }
    return results;
  });
  for(const result of results){
    if(result.actual)for(let c=0;c<4;c++)expect(Math.abs(result.actual[c]-result.expected[c]),JSON.stringify(result)).toBeLessThan(2);
    if(result.sharp){if(result.columns===2)expect(result.sharp[0]).toBeGreaterThan(result.average[0]+20);expect(result.sharp[3]).toBe(128);}
    if(result.corner){expect(result.corner).toEqual([0,0,0,0]);expect(result.occupied).toBe(true);}
  }
  await testInfo.attach('Mosaic-independent-results',{body:JSON.stringify(results,null,2),contentType:'application/json'});
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Unsharp Mask and Find Edges preserve neutral settings, thresholds and alpha',async({session})=>{
  const results=await session.page.evaluate(()=>{
    const PM=(window as any).PM,s=(window as any).__spatial,results:any[]=[];
    for(const depth of [8,16,32]){
      if(!PM.GL.supportedColorDepths.includes(depth))continue;
      const layer=s.scene('float v=mix(.25,.75,step(.5,uv.x));fragColor=vec4(vec3(v*.5),.5);',depth);
      const original=s.render();s.fx(layer,'unsharpMask',{amount:0});const neutral=s.render();
      s.fx(layer,'unsharpMask',{amount:100,radius:2,threshold:255});const threshold=s.render();
      s.fx(layer,'unsharpMask',{amount:100,radius:2});const sharpened=s.render();
      s.fx(layer,'findEdges',{});const edges=s.render();
      s.fx(layer,'findEdges',{invert:true});const inverted=s.render();
      s.fx(layer,'findEdges',{blendWithOriginal:100});const blend=s.render();
      results.push({depth,neutral:JSON.stringify(neutral)===JSON.stringify(original),threshold:JSON.stringify(threshold)===JSON.stringify(original),blend:JSON.stringify(blend)===JSON.stringify(original),
        before:s.pixel(original,32,32),after:s.pixel(sharpened,32,32),flat:s.pixel(edges,16,32),edge:s.pixel(edges,31,32),inverted:s.pixel(inverted,16,32)});
    }
    return results;
  });
  for(const r of results){expect(r.neutral).toBe(true);expect(r.threshold).toBe(true);expect(r.blend).toBe(true);expect(r.after[0]).toBeGreaterThan(r.before[0]);expect(r.after[3]).toBe(r.before[3]);expect(r.flat).toEqual([128,128,128,128]);expect(r.inverted).toEqual([0,0,0,128]);expect(r.edge[0]).toBeLessThan(r.flat[0]);}
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('wide spatial kernels budget mip textures, restore pooled samplers and retain signed HDR',async({session},testInfo)=>{
  const results=await session.page.evaluate(()=>{
    const PM=(window as any).PM,s=(window as any).__spatial,gl=PM.GL.gl,results:any[]=[];
    for(const depth of [16,32]){
      if(!PM.GL.supportedColorDepths.includes(depth))continue;
      for(const type of ['blur','pixelate','unsharpMask']){
        const layer=s.scene('fragColor=vec4(-.25,2.,.5,1.);',depth,128);
        s.fx(layer,type,type==='blur'?{amount:100,repeatEdgePixels:true}:type==='pixelate'?{horizontalBlocks:2,verticalBlocks:2}:{amount:100,radius:30});
        let sample:number[]|undefined;const read=gl.readPixels.bind(gl);
        gl.readPixels=(...args:any[])=>{read(...args);if(args[5]===gl.FLOAT&&args[6]instanceof Float32Array)sample=[...args[6].slice((64*128+64)*4,(64*128+64)*4+4)];};
        let pixels;try{pixels=s.render(128);}finally{gl.readPixels=read;}
        const repeated=s.render(128),accounted=PM.GL.memoryStats().framebuffers.bytes,allocated=PM.GL.pool.reduce((sum:number,f:any)=>sum+f.bytes,0),mips=PM.GL.pool.filter((f:any)=>f.mipLevels>0);
        const previousBinding=gl.getParameter(gl.TEXTURE_BINDING_2D);
        const sampler=mips.every((f:any)=>{gl.bindTexture(gl.TEXTURE_2D,f.tex);return gl.getTexParameter(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER)===gl.LINEAR&&gl.getTexParameter(gl.TEXTURE_2D,gl.TEXTURE_MAX_LEVEL)===0;});
        // Direct inspection changes binding state: restore through the renderer before another capture.
        gl.bindTexture(gl.TEXTURE_2D,previousBinding);
        results.push({depth,type,sample,accounted,allocated,same:JSON.stringify(pixels)===JSON.stringify(repeated),mips:mips.map((f:any)=>({w:f.w,h:f.h,bytes:f.bytes,mipBytes:f.mipBytes})),sampler});
      }
    }
    return results;
  });
  for(const r of results){expect(r.sample).toBeDefined();for(const[c,v]of [-.25,2,.5,1].entries())expect(Math.abs(r.sample[c]-v),r.type).toBeLessThan(.006);expect(r.same).toBe(true);expect(r.mips.length).toBeGreaterThan(0);expect(r.mips.every((f:any)=>f.mipBytes>0&&f.bytes>f.mipBytes)).toBe(true);expect(r.sampler).toBe(true);expect(r.accounted).toBe(r.allocated);}
  await testInfo.attach('spatial-mip-budget',{body:JSON.stringify(results,null,2),contentType:'application/json'});
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('saved Blur and Mosaic revisions coexist and survive hydration without changing channels or pixels',async({session})=>{
  const result=await session.page.evaluate(()=>{
    const PM=(window as any).PM,s=(window as any).__spatial;
    s.scene('fragColor=vec4(uv.x,uv.y,0.,1.);');let layer=PM.proj.layers[0];
    const snapshots:any[]=[];
    for(const type of ['blur','pixelate']){
      const old=PM.mkEffect(type,1),current=PM.mkEffect(type);delete old.version;
      layer.fx=[old,current];PM.touch();const before=s.render();
      PM.replaceProject(PM.hydrateProject(JSON.parse(JSON.stringify(PM.proj))));const after=s.render();
      const loaded=PM.proj.layers[0];snapshots.push({type,same:JSON.stringify(before)===JSON.stringify(after),versions:loaded.fx.map((fx:any)=>fx.version),keys:loaded.fx.map((fx:any)=>Object.keys(fx.p))});
      layer=loaded;
    }
    return{snapshots,programs:[...PM.GL.progs.keys()]};
  });
  for(const r of result.snapshots){expect(r.same).toBe(true);expect(r.versions).toEqual([1,2]);expect(result.programs).toContain('fx:'+r.type+'@1');expect(result.programs).toContain('fx:'+r.type);}
  expect(result.snapshots[1].keys).toEqual([['size'],['horizontalBlocks','verticalBlocks','sharpColors']]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});


test('spatial effects keep cropped high-zoom pixels aligned with full-frame renders and native controls',async({session},testInfo)=>{
  const result=await session.page.evaluate(()=>{
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');
    const project=PM.mkProject({name:'Spatial controls',w:512,h:288,dur:2});project.colorDepth=16;
    project.backgroundFill={type:'none',stops:[],angle:0};
    const layer=PM.mkLayer('shape',{name:'Spatial effects',d:{w:180,h:100,radius:0,color:'#E95A2B'},p:{'position.x':256,'position.y':144}},project);
    project.layers=[layer];PM.replaceProject(project);PM.perf.auto=false;PM.previewResolution='auto';
    viewer.fit=false;viewer.zoom=4;viewer.pan=[0,0];
    const results:any[]=[];
    for(const type of ['blur','pixelate','unsharpMask','findEdges','shadow','gradient']){
      const fx=PM.mkEffect(type,undefined,layer);layer.fx=[fx];
      if(type==='blur')fx.p.amount.v=4;
      if(type==='unsharpMask')fx.p.radius.v=2;
      if(type==='shadow'){fx.p.softness.v=4;fx.p.distance.v=12;}
      PM.touch();viewer.layout();PM.GL.render(0,{mblur:false,exporting:true});
      const gl=PM.GL.gl,cv=PM.GL.canvas,v=PM.GL.previewViewport;
      const cropped=new Uint8Array(cv.width*cv.height*4);gl.readPixels(0,0,cv.width,cv.height,gl.RGBA,gl.UNSIGNED_BYTE,cropped);
      if(type==='pixelate'){results.push({type,cropped:!!v});continue;}
      if(!v)throw Error('Missing cropped viewport for '+type);
      const scale=cv.width/v.width,fullW=Math.round(512*scale),fullH=Math.round(288*scale);
      const full=PM.GL.renderToPixels(0,fullW,fullH,{transparent:true,mblur:false});
      const ox=Math.round(v.x*scale),oy=Math.round(v.y*scale);
      let max=0,changed=0,samples=0;
      // Compare the visible center with padding around every neighborhood kernel.
      for(let y=32;y<cv.height-32;y+=7)for(let x=32;x<cv.width-32;x+=7){
        const fxp=ox+x,fyp=fullH-oy-cv.height+y;if(fxp<0||fxp>=fullW||fyp<0||fyp>=fullH)continue;
        for(let c=0;c<3;c++){const delta=Math.abs(cropped[(y*cv.width+x)*4+c]!-full[(fyp*fullW+fxp)*4+c]!);max=Math.max(max,delta);if(delta>2)changed++;samples++;}
      }
      results.push({type,cropped:true,max,changed,samples});
    }
    layer.fx=['blur','pixelate','unsharpMask','findEdges'].map(type=>PM.mkEffect(type));
    PM.selectLayers(layer.id);for(const fx of layer.fx)PM.UIState.setFxOpen(fx,true);
    PM.agentFrameCapture=false;PM.setTime(0,{force:true});PM.touch();PM.bus.emit('layers');PM.invalidate();
    return{results};
  });
  for(const r of result.results){if(r.type==='pixelate')expect(r.cropped).toBe(false);else{expect(r.cropped).toBe(true);expect(r.samples).toBeGreaterThan(100);expect(r.max,JSON.stringify(r)).toBeLessThanOrEqual(3);}}
  await expect(session.page.getByText('Blur Dimensions',{exact:true})).toBeVisible();
  await expect(session.page.getByText('Repeat Edge Pixels',{exact:true})).toBeVisible();
  for(const type of ['blur','pixelate','unsharpMask','findEdges']){
    const id=await session.page.evaluate(type=>{
      const PM=(window as any).PM,layer=PM.proj.layers[0],fx=PM.mkEffect(type),viewer=PM.Kernel.services.get('viewer');
      if(type==='blur')fx.p.amount.v=6;
      layer.fx=[fx];PM.UIState.setFxOpen(fx,true);PM.touch();PM.bus.emit('layers');
      viewer.fit=true;viewer.layout();PM.invalidate();return fx.id;
    },type);
    await expect(session.page.locator(`[data-effect-id="${id}"]`)).toBeVisible();
    for(const theme of ['light','dark']){
      await session.page.evaluate(theme=>{const PM=(window as any).PM;PM.theme.apply(theme);PM.invalidate('render');},theme);
      await session.page.waitForFunction(()=>{const PM=(window as any).PM;return !PM.GL.compiling&&PM.GL.errors.size===0;});
      await session.page.screenshot({path:testInfo.outputPath(`spatial-${type}-${theme}.png`)});
    }
  }
  await testInfo.attach('crop-comparison',{body:JSON.stringify(result,null,2),contentType:'application/json'});
  expect(session.diagnostics.pageErrors).toEqual([]);
});
