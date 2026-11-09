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
