import {test,expect} from './helpers/app';
import {twoMaterialGlb} from './helpers/two-material-glb';

test('material GPU conversion preserves row orientation, straight alpha and HDR quantization',async({session})=>{
  await session.openEditor();
  const results=await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();
    const main=PM.proj.compId;
    const comp=PM.Comps.create({name:'Alpha texture',w:64,h:64,fps:30,dur:3},{open:true});
    for(const [index,color] of ['#FF4400','#0044FF'].entries()){
      PM.Edit.apply({type:'add_layer',id:`paint-${index}`,layerType:'solid',content:{w:64,h:32,color},
        properties:{'position.x':32,'position.y':index*32+16,'anchor.x':32,'anchor.y':16,'opacity':50}});
      PM.Edit.apply([{type:'set_property',target:`paint-${index}`,path:'position.x',value:32},
        {type:'set_property',target:`paint-${index}`,path:'position.y',value:index*32+16}]);
    }
    PM.Comps.open(main);
    return PM.GL.supportedColorDepths.map((depth:number)=>{
      PM.proj.colorDepth=depth;PM.touch();
      const source=PM.proj.comps[comp];
      const raw=PM.GL.renderToPixels(0,64,64,{comp:source,transparent:true,mblur:false});
      const actual=PM.GL.renderMaterialComposition(source,0,64,64,0);
      let maxError=0;
      for(let y=0;y<64;y++)for(let x=0;x<64;x++){
        const src=((63-y)*64+x)*4,dst=(y*64+x)*4,alpha=raw[src+3];
        for(let c=0;c<4;c++){
          const expected=c===3?alpha:alpha?Math.min(255,Math.round(raw[src+c]*255/alpha)):0;
          maxError=Math.max(maxError,Math.abs(expected-actual[dst+c]));
        }
      }
      const sample=(y:number)=>[...actual.slice((y*64+32)*4,(y*64+32)*4+4)];
      return {depth,maxError,top:sample(16),bottom:sample(48),errors:[...PM.GL.errors.entries()]};
    });
  });
  for(const result of results){
    expect(result.errors).toEqual([]);expect(result.maxError).toBeLessThanOrEqual(1);
    expect(result.top[0]).toBeGreaterThan(result.top[2]+100);
    expect(result.bottom[2]).toBeGreaterThan(result.bottom[0]+100);
    expect(result.top[3]).toBeGreaterThan(120);expect(result.top[3]).toBeLessThan(135);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('static artwork on multiple material slots is captured and uploaded once across playback',async({session},testInfo)=>{
  test.setTimeout(120_000);await session.openEditor();
  const result=await session.page.evaluate(async bytes=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();
    const api=PM.Kernel.api('material-performance'),main=PM.proj.compId,size=1024;
    const comp=PM.Comps.create({name:'Static texture',w:size,h:size,fps:30,dur:3},{open:true});
    PM.Edit.apply({type:'add_layer',id:'paint',layerType:'solid',content:{w:size,h:size,color:'#E5484D'},
      properties:{'position.x':size/2,'position.y':size/2,'anchor.x':size/2,'anchor.y':size/2}});
    PM.Edit.apply(['position.x','position.y'].map(path=>({type:'set_property',target:'paint',path,value:size/2})));
    PM.Comps.open(main);
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3}}});
    const imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'two-materials.glb',{type:'model/gltf-binary'}));
    if(!imported.ok)throw new Error(imported.message);
    const model=PM.proj.layers.find((layer:any)=>layer.d?.data?.object).id;
    for(const slot of ['color','emissive']){
      const applied=api.scene3d.setMaterialMap(model,null,slot,{compositionId:comp});
      if(!applied.ok)throw new Error(applied.message);
    }
    // Warm shader/geometry work, then invalidate only the material capture.
    PM.GL.renderToPixels(0,320,180,{transparent:true,mblur:false});PM.touch();PM.bus.emit('fonts');
    const originalCapture=PM.GL.renderMaterialComposition;
    const originalUpload=WebGL2RenderingContext.prototype.texSubImage2D;
    let captures=0,uploads=0,captureMs=0;const frameMs:number[]=[];
    PM.GL.renderMaterialComposition=(...args:any[])=>{
      captures++;const started=performance.now();try{return originalCapture(...args);}finally{captureMs+=performance.now()-started;}
    };
    (WebGL2RenderingContext.prototype as any).texSubImage2D=function(...args:any[]){
      const data=args.at(-1);
      if(this!==PM.GL.gl && ArrayBuffer.isView(data) && data.byteLength===size*size*4)uploads++;
      return originalUpload.apply(this,args as any);
    };
    try{
      for(let frame=0;frame<60;frame++){
        const started=performance.now();PM.GL.renderToPixels(frame/30,320,180,{transparent:true,mblur:false});
        frameMs.push(performance.now()-started);
      }
      // Dragging the receiver is an edit every frame; its unchanged source
      // must stay resident rather than re-capturing/re-uploading on each step.
      for(let step=0;step<10;step++){
        const edited=PM.Edit.apply({type:'set_property',target:model,path:'rotation.y',value:step});
        if(!edited.ok)throw new Error(edited.message);
        PM.GL.renderToPixels(step/30,320,180,{transparent:true,mblur:false});
      }
      return {captures,uploads,captureMs,frames:frameMs.length,medianMs:[...frameMs].sort((a,b)=>a-b)[30],errors:[...PM.GL.errors.entries()]};
    }finally{
      PM.GL.renderMaterialComposition=originalCapture;
      WebGL2RenderingContext.prototype.texSubImage2D=originalUpload;
    }
  },[...twoMaterialGlb()]);
  await testInfo.attach('material-performance',{body:Buffer.from(JSON.stringify(result,null,2)),contentType:'application/json'});
  expect(result.errors).toEqual([]);expect(result.captures).toBe(1);expect(result.uploads).toBe(1);
  expect(result.frames).toBe(60);expect(session.diagnostics.pageErrors).toEqual([]);
});

for(const sourceType of ['precomp','composition','group'])test(`retains clipped imported artwork used as a ${sourceType} material without losing edits or animation`,async({session},testInfo)=>{
  test.setTimeout(120_000);await session.openEditor();
  const result=await session.page.evaluate(async({bytes,sourceType})=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({w:320,h:180,fps:30,dur:4}));
    const api=PM.Kernel.api('clipped-material-performance'),main=PM.proj.compId,size=512;
    const compId=PM.Comps.create({name:'Imported frame',w:size,h:size,fps:30,dur:4},{open:true});
    const make=(id:string,type:string,d:any,x:number,y:number)=>{
      const layer=PM.mkLayer(type,{from:0,dur:4,d,p:{'position.x':x,'position.y':y}});layer.id=id;PM.proj.layers.push(layer);return layer;
    };
    const artwork=make('artwork','group',{},0,0);
    const mask=PM.mkMask('rect',PM.proj);
    mask.p.x.v=mask.p.y.v=size/2;mask.p.w.v=mask.p.h.v=size;mask.p.feather.v=0;
    artwork.masks=[mask];artwork.fx=[PM.mkEffect('invert',undefined,artwork)];
    const paper=make('paper','shape',{shape:'rect',w:size,h:size,color:'#FFFFFF'},size/2,size/2);paper.group=artwork.id;
    // Figma frame clipping uses track mattes, including a stack of nested clips.
    for(let i=0;i<12;i++){
      const clip=make(`clip-${i}`,'shape',{shape:'rect',w:100,h:80,radius:12,color:'#FFFFFF'},60+i%4*128,70+Math.floor(i/4)*128);
      const paint=make(`tile-${i}`,'shape',{shape:'ellipse',w:128,h:100,color:'#E5484D'},clip.p['position.x'].v,clip.p['position.y'].v);
      clip.group=paint.group=artwork.id;paint.matteSource=clip.id;paint.matteMode='alpha';
      paint.matteSources=[{source:clip.id,mode:'alpha'}];
    }
    const title=make('title','text',{text:'Material artwork',fontSize:24,color:'#202020'},200,470);title.group=artwork.id;
    const bitmap=await createImageBitmap(new ImageData(new Uint8ClampedArray([20,180,90,255]),1,1));
    PM.assets.map.set('tile-image',{id:'tile-image',kind:'image',el:bitmap,w:1,h:1});
    const image=make('image','image',{asset:'tile-image',w:64,h:64},460,460);image.group=artwork.id;
    PM.proj.layers.reverse();PM.touch();PM.Comps.open(main);
    let source:any;
    if(sourceType==='group'){
      PM.proj.layers.push(...PM.proj.comps[compId].layers);PM.proj.comps[compId].layers=[];artwork.on=false;source={layerId:artwork.id};
    }else if(sourceType==='precomp'){
      const frame=make('frame','precomp',{comp:compId,w:size,h:size},0,0);frame.on=false;source={layerId:frame.id};
    }else source={compositionId:compId};
    PM.touch();api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3}}});
    const imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'two-materials.glb',{type:'model/gltf-binary'}));
    if(!imported.ok)throw new Error(imported.message);
    const model=PM.proj.layers.find((layer:any)=>layer.d?.data?.object).id;
    for(const slot of ['color','emissive']){
      const applied=api.scene3d.setMaterialMap(model,null,slot,source);if(!applied.ok)throw new Error(applied.message);
    }
    await document.fonts.ready;
    PM.GL.renderToPixels(0,320,180,{transparent:true,mblur:false});PM.bus.emit('fonts');
    const original=PM.GL.renderMaterialComposition;let captures=0;const captureTimes:number[]=[],frames:number[]=[];
    PM.GL.renderMaterialComposition=(...args:any[])=>{
      captures++;const start=performance.now();try{return original(...args);}finally{captureTimes.push(performance.now()-start);}
    };
    const draw=(time:number)=>{const start=performance.now();const pixels=PM.GL.renderToPixels(time,320,180,{transparent:true,mblur:false});frames.push(performance.now()-start);return pixels;};
    const hash=(pixels:Uint8Array)=>{let n=0;for(const b of pixels)n=(Math.imul(n,31)+b)|0;return n;};
    try{
      const first=hash(draw(0));for(let i=1;i<30;i++)draw(i/30);
      for(let i=0;i<5;i++){
        PM.Edit.apply({type:'set_property',target:model,path:'rotation.y',value:i});draw(i/30);
      }
      const staticCaptures=captures;
      // A descendant edit must replace the retained pixels, including through a precomp.
      const sourceComp=sourceType==='group'?PM.proj:PM.proj.comps[compId];
      const tile=sourceComp.layers.find((l:any)=>l.id==='tile-0');tile.d.color='#0044FF';PM.touch();draw(.5);
      const editedCaptures=captures;
      tile.p['position.x'].kf=[PM.KF(0,tile.p['position.x'].v),PM.KF(1,tile.p['position.x'].v+100)];PM.touch();
      draw(.5);const a=hash(draw(.6)),b=hash(draw(.8));
      return {staticCaptures,editedCaptures,animatedCaptures:captures-editedCaptures,animatedChanged:a!==b,first,
        captureMs:captureTimes.reduce((a,b)=>a+b,0),medianMs:frames.slice(1,30).sort((a,b)=>a-b)[14],errors:[...PM.GL.errors.entries()]};
    }finally{PM.GL.renderMaterialComposition=original;}
  },{bytes:[...twoMaterialGlb()],sourceType});
  await testInfo.attach(`${sourceType}-performance`,{body:Buffer.from(JSON.stringify(result,null,2)),contentType:'application/json'});
  expect(result.errors).toEqual([]);expect(result.staticCaptures).toBe(1);expect(result.editedCaptures).toBe(2);
  expect(result.animatedCaptures).toBe(3);expect(result.animatedChanged).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
