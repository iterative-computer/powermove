import {mkdir,writeFile} from 'node:fs/promises';
import {test,expect} from './helpers/app';
const shots='/tmp/pm-figuate-feedback/shots/finish-engine-gaps';
test.beforeAll(async()=>{await mkdir(shots,{recursive:true});});

test('first edited built-in resets to its original arrangement; Save Changes establishes a new baseline',async({session})=>{
  await session.openEditor();
  await session.page.screenshot({path:`${shots}/01-before-built-in-layout.png`});
  const original=await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.WS.reset('design');
    const original=JSON.stringify(PM.WS.current.layout);
    PM.WS.mutate((w:any)=>{w.layout.docks.find((d:any)=>d.id==='left').size=377;});
    return original;
  });
  await session.page.screenshot({path:`${shots}/02-after-first-rearrangement.png`});
  const reset=await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.WS.reset();return JSON.stringify(PM.WS.current.layout);
  });
  expect(reset).toBe(original);
  await session.page.screenshot({path:`${shots}/03-after-reset-to-built-in.png`});
  await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.WS.mutate((w:any)=>{w.layout.docks.find((d:any)=>d.id==='left').size=388;});
    PM.WS.saveLayout();PM.WS.mutate((w:any)=>{w.layout.docks.find((d:any)=>d.id==='left').size=399;});
  });
  await session.relaunch();await session.openEditor();
  expect(await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.WS.reset();return PM.WS.current.layout.docks.find((d:any)=>d.id==='left').size;
  })).toBe(388);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('picker previews asynchronously render layers, compositions and images, then refresh after source edits',async({session})=>{
  await session.openEditor();
  const result=await session.page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({w:320,h:180,dur:3}));
    const api=PM.Kernel.api('finish-engine-proof').scene3d;
    const made=api.edit({operation:'add_object',object:{id:'model',source:{primitive:'box'}}});if(!made.ok)throw Error(made.message);
    const added=PM.Edit.apply({type:'add_layer',id:'paint',layerType:'solid',visible:false,content:{w:128,h:64,color:'#FF0000'}});if(!added.ok)throw Error(added.message);
    PM.time=0;
    const main=PM.proj.compId,comp=PM.Comps.create({name:'Thumbnail comp',w:128,h:64,dur:3},{open:true});
    PM.Edit.apply({type:'add_layer',id:'inside',layerType:'solid',content:{w:128,h:64,color:'#0000FF'},properties:{'position.x':64,'position.y':32,'anchor.x':64,'anchor.y':32}});
    PM.Comps.open(main);
    const canvas=document.createElement('canvas');canvas.width=canvas.height=8;
    const context=canvas.getContext('2d')!;context.fillStyle='#FFFF00';context.fillRect(0,0,8,8);
    const blob=await new Promise<Blob>(resolve=>canvas.toBlob(b=>resolve(b!)));
    const asset=await PM.assets.add(new File([blob],'thumbnail-yellow.png',{type:'image/png'}),{silent:true});
    const originalCapture=PM.GL.renderMaterialComposition;let captures=0;
    PM.GL.renderMaterialComposition=(...args:any[])=>{captures++;return originalCapture(...args);};
    try{
      const options={time:0,size:64,pixelRatio:2};
      // Loading an asset can publish a late source-change event. A null result
      // keeps the picker placeholder; retry once the new source state settles.
      const preview=async(source:any)=>{
        for(let attempt=0;attempt<20;attempt++){
          const result=await api.getMaterialThumbnail('model',source,options);if(result)return result;
          await new Promise(resolve=>setTimeout(resolve,16));
        }
        throw Error(`Thumbnail unavailable: ${JSON.stringify(source)}`);
      };
      const first=api.getMaterialThumbnail('model',{layerId:'paint'},options);
      const duplicate=api.getMaterialThumbnail('model',{layerId:'paint'},options);
      const deferred=captures===0;
      let [layer,same,composition,image]=await Promise.all([first,duplicate,
        api.getMaterialThumbnail('model',{compositionId:comp},options),api.getMaterialThumbnail('model',asset.id,options)]);
      layer ||= await preview({layerId:'paint'});same ||= layer;
      composition ||= await preview({compositionId:comp});image ||= await preview(asset.id);
      const afterFirst=captures;
      const cached=await api.getMaterialThumbnail('model',{layerId:'paint'},options);
      const reused=captures===afterFirst && cached.url===layer.url && same.url===layer.url;
      const edit=PM.Edit.apply({type:'set_property',target:'paint',path:'c.color',value:'#00FF00'});if(!edit.ok)throw Error(edit.message);
      const changed=await preview({layerId:'paint'});
      const pixel=async(url:string)=>{const img=new Image();img.src=url;await img.decode();const c=document.createElement('canvas');c.width=img.width;c.height=img.height;const ctx=c.getContext('2d')!;ctx.drawImage(img,0,0);return [...ctx.getImageData(c.width/2,c.height/2,1,1).data];};
      return {deferred,reused,layer,composition,image,changed,red:await pixel(layer.url),green:await pixel(changed.url),yellow:await pixel(image.url),
        accepted:api.canAssignMaterialMap('model',0,'color',{layerId:'paint'}),rejected:api.canAssignMaterialMap('model',0,'normal',{layerId:'paint'})};
    }finally{PM.GL.renderMaterialComposition=originalCapture;}
  });
  expect(result.deferred).toBe(true);expect(result.reused).toBe(true);
  expect(result.layer).toMatchObject({width:128,height:64});
  expect(result.red).toEqual([255,0,0,255]);expect(result.green).toEqual([0,255,0,255]);expect(result.yellow).toEqual([255,255,0,255]);
  expect(result.accepted).toEqual({ok:true});expect(result.rejected).toMatchObject({ok:false,reason:expect.stringContaining('base color')});
  for(const [name,thumb] of Object.entries({layer:result.layer,composition:result.composition,image:result.image,changed:result.changed}))
    await writeFile(`${shots}/04-thumbnail-${name}.png`,Buffer.from(thumb.url.split(',')[1],'base64'));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('preset delete and rename can be undone across windows while retaining newer presets',async({session})=>{
  await session.openEditor();
  const saved=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,layer=PM.mkLayer('solid');PM.proj.layers=[layer];PM.ProjectIndex.invalidate();PM.selectLayers(layer.id);
    const a=PM.Presets.save({name:'First',propertyPaths:['opacity']}),b=PM.Presets.save({name:'Second',propertyPaths:['opacity']});
    await PM.store.flush();return {a,b};
  });
  const opened=session.app.waitForEvent('window');
  await session.app.evaluate(({Menu})=>{const item=Menu.getApplicationMenu()!.getMenuItemById('newWindow')!;item.click(item,undefined,{} as any);});
  const second=await opened;await second.waitForFunction(()=>Boolean((window as any).PM?.app));
  await expect.poll(()=>second.evaluate(id=>(window as any).PM.Presets?.get(id)?.name,saved.b.id)).toBe('Second');
  const receipt=await session.page.evaluate(id=>(window as any).PM.Presets.removeWithUndo(id),saved.a.id);
  await expect.poll(()=>second.evaluate(id=>(window as any).PM.Presets.get(id),saved.a.id)).toBeNull();
  await second.evaluate(preset=>{const PM=(window as any).PM;PM.Presets.restore({...preset,id:'external',name:'Elsewhere'},1);},saved.a);
  await expect.poll(()=>session.page.evaluate(()=>(window as any).PM.Presets.get('external')?.name)).toBe('Elsewhere');
  expect(await session.page.evaluate(receipt=>(window as any).PM.Presets.restore(receipt.preset,receipt.index),receipt)).toBe(true);
  await expect.poll(()=>second.evaluate(()=>(window as any).PM.Presets.list().map((p:any)=>p.name))).toEqual(['First','Second','Elsewhere']);
  await session.page.evaluate(id=>(window as any).PM.Presets.rename(id,'Renamed'),saved.b.id);
  await expect.poll(()=>second.evaluate(id=>(window as any).PM.Presets.get(id).name,saved.b.id)).toBe('Renamed');
  expect(await second.evaluate(id=>(window as any).PM.Presets.rename(id,'Second','Renamed'),saved.b.id)).toBe(true);
  await expect.poll(()=>session.page.evaluate(id=>(window as any).PM.Presets.get(id).name,saved.b.id)).toBe('Second');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
