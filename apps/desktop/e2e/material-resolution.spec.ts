import {test,expect} from './helpers/app';

for(const kind of ['group layer','image layer','vector composition','continuous precomp','nested 3D composition'] as const){
  test(`${kind} keeps fine source detail on a 2048-pixel screen`,async({session},testInfo)=>{
    test.setTimeout(120_000);await session.openEditor();
    const result=await session.page.evaluate(async kind=>{
      const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();
      PM.replaceProject(PM.mkProject({name:'Material resolution',w:2048,h:512,fps:30,dur:3}));
      const main=PM.proj.compId,api=PM.Kernel.api('material-resolution');let map:any;
      const check=(result:any)=>{if(!result.ok)throw new Error(result.message);return result;};
      if(kind==='image layer'){
        const canvas=document.createElement('canvas');canvas.width=2048;canvas.height=512;
        const context=canvas.getContext('2d')!;context.fillStyle='#FFFFFF';context.fillRect(0,0,2048,512);
        context.fillStyle='#000000';for(let x=0;x<2048;x+=2)context.fillRect(x,0,1,512);
        const blob=await new Promise<Blob>(resolve=>canvas.toBlob(blob=>resolve(blob!)));
        const asset=await PM.assets.add(new File([blob],'fine-detail.png',{type:'image/png'}),{silent:true});
        check(PM.Edit.apply({type:'add_layer',id:'artwork',layerType:'image',visible:false,content:{asset:asset.id,w:256,h:64,fit:'stretch'}}));
        map={layerId:'artwork'};
      }else if(kind==='nested 3D composition'){
        const comp=PM.Comps.create({name:'3D screen content',w:128,h:128,fps:30,dur:3},{open:true});
        check(api.scene3d.edit({operation:'add_camera',camera:{projection:'orthographic',p:{x:0,y:0,z:3,zoom:3}}}));
        check(api.scene3d.edit({operation:'add_object',object:{id:'inner',source:{primitive:'box'},material:{p:{color:'#FFFFFF',emissive:'#FFFFFF',emissiveIntensity:1}}}}));
        PM.Comps.open(main);map={compositionId:comp};
      }else{
        const comp=kind==='group layer'?main:PM.Comps.create({name:'Grouped vector artwork',w:64,h:16,fps:30,dur:3},{open:true});
        const commands:any[]=[];
        const add=(id:string,w:number,x:number,color:string)=>commands.push(
          {type:'add_layer',id,layerType:id==='paper'?'solid':'shape',content:{w,h:16,color,...(id==='paper'?{}:{shape:'rect',radius:0,stroke:0})},properties:{'anchor.x':0,'anchor.y':0}},
          {type:'set_property',target:id,path:'position.x',value:x},{type:'set_property',target:id,path:'position.y',value:0});
        add('paper',64,0,'#FFFFFF');
        for(let index=0;index<128;index++)add(`stripe-${index}`,.25,index*.5,'#000000');
        check(PM.Edit.apply(commands));
        check(PM.Edit.apply({type:'group_layers',targets:PM.proj.layers.map((layer:any)=>layer.id),name:'Vector stripes'}));
        const group=PM.proj.layers.find((layer:any)=>layer.type==='group');
        if(kind==='group layer'){group.on=false;PM.touch();map={layerId:group.id};}
        PM.Comps.open(main);
        if(kind==='continuous precomp'){
          check(PM.Edit.apply({type:'add_layer',id:'artwork',layerType:'precomp',visible:false,content:{comp,w:64,h:16}}));
          check(PM.Edit.apply({type:'set_layer',target:'artwork',patch:{continuousRaster:true}}));
          map={layerId:'artwork'};
        }else if(kind!=='group layer')map={compositionId:comp};
      }
      check(api.scene3d.edit({operation:'add_camera',camera:{projection:'orthographic',p:{x:0,y:0,z:3,zoom:3}}}));
      check(api.scene3d.edit({operation:'add_object',object:{id:'screen',source:{mesh:{
        positions:[-4,-1,0,4,-1,0,-4,1,0,4,-1,0,4,1,0,-4,1,0],uvs:[0,1,1,1,0,0,1,1,1,0,0,0]}},
        useSourceMaterials:false,material:{p:{color:'#FFFFFF',roughness:1,metalness:0,emissive:'#FFFFFF',emissiveIntensity:1}}}}));
      for(const slot of ['color','emissive'])check(api.scene3d.setMaterialMap('screen',null,slot,map));
      const original=PM.GL.renderMaterialComposition,captures:{width:number;height:number}[]=[];
      PM.GL.renderMaterialComposition=(...args:any[])=>{captures.push({width:args[2],height:args[3]});return original(...args);};
      try{
        const canvas=PM.renderFrameTo(0,2048,512,{mblur:false}),ctx=canvas.getContext('2d')!;
        const pixels=ctx.getImageData(256,256,1536,1).data;
        let low=255,high=0,mid=0,adjacent=0;
        for(let i=0;i<pixels.length;i+=4){
          const value=pixels[i];low=Math.min(low,value);high=Math.max(high,value);
          if(value>40 && value<180)mid++;
          if(i)adjacent+=Math.abs(value-pixels[i-4]);
        }
        let zoomedContrast:number|undefined;
        if(kind==='group layer' || kind==='vector composition' || kind==='continuous precomp'){
          check(api.scene3d.edit({operation:'set_camera',patch:{p:{zoom:6}}}));
          const zoomed=PM.renderFrameTo(0,2048,512,{mblur:false}).getContext('2d').getImageData(256,256,1536,1).data;
          let min=255,max=0;for(let i=0;i<zoomed.length;i+=4){min=Math.min(min,zoomed[i]);max=Math.max(max,zoomed[i]);}zoomedContrast=max-min;
        }
        return {captures,low,high,midFraction:mid/(pixels.length/4),adjacentContrast:adjacent/(pixels.length/4-1),zoomedContrast,screenBytes:PM.GL.texes.get('layer3d:screen')?.bytes,errors:[...PM.GL.errors.entries()]};
      }finally{PM.GL.renderMaterialComposition=original;}
    },kind);
    await testInfo.attach('material-resolution',{body:Buffer.from(JSON.stringify(result,null,2)),contentType:'application/json'});
    expect(result.errors).toEqual([]);expect(result.captures.some(size=>size.width>=2048)).toBe(true);
    expect(result.screenBytes).toBe(2048*512*4);
    expect(result.high-result.low).toBeGreaterThan(150);expect(result.midFraction).toBeLessThan(.25);
    if(kind==='image layer')expect(result.adjacentContrast).toBeGreaterThan(120);
    if(kind==='group layer' || kind==='vector composition' || kind==='continuous precomp'){
      expect(result.captures.some(size=>size.width>=4096)).toBe(true);expect(result.zoomedContrast).toBeGreaterThan(150);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
