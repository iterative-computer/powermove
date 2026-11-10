import {test,expect} from './helpers/app';

function modelGlb():number[]{
  const positions=new Float32Array([-2,-1,0,2,-1,0,0,1,1]);
  const json=JSON.stringify({asset:{version:'2.0'},scene:0,scenes:[{nodes:[0]}],nodes:[{name:'Body',mesh:0,translation:[8,4,-3]}],
    meshes:[{primitives:[{attributes:{POSITION:0}}]}],buffers:[{byteLength:positions.byteLength}],
    bufferViews:[{buffer:0,byteOffset:0,byteLength:positions.byteLength}],accessors:[{bufferView:0,componentType:5126,count:3,type:'VEC3',min:[-2,-1,0],max:[2,1,1]}]});
  const text=new TextEncoder().encode(json.padEnd(Math.ceil(json.length/4)*4,' '));
  const bytes=new Uint8Array(28+text.length+positions.byteLength),view=new DataView(bytes.buffer);
  view.setUint32(0,0x46546c67,true);view.setUint32(4,2,true);view.setUint32(8,bytes.length,true);
  view.setUint32(12,text.length,true);view.setUint32(16,0x4e4f534a,true);bytes.set(text,20);
  view.setUint32(20+text.length,positions.byteLength,true);view.setUint32(24+text.length,0x004e4942,true);
  bytes.set(new Uint8Array(positions.buffer),28+text.length);return [...bytes];
}

test('imported models under an animated group gain real bounds nulls without changing rendered frames',async({session})=>{
  test.setTimeout(90_000);await session.openEditor();
  const result=await session.page.evaluate(async bytes=>{
    const PM=(window as any).PM,api=PM.Kernel.api('bounds-proof');PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({name:'Imported bounds',w:640,h:360,fps:30,dur:3}));
    const check=(r:any)=>{if(!r.ok)throw new Error(r.message);return r.data;};
    const imported=check(await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'model.glb',{type:'model/gltf-binary'}))).result.id;
    check(api.scene3d.edit({operation:'add_object',object:{id:'other',source:{primitive:'box'},p:{x:1.5,sx:.4,sy:.4,sz:.4}}}));
    const group=PM.groupLayers([imported,'other']);
    check(PM.Edit.apply({type:'replace_keyframes',target:group.id,path:'rotation.y',keyframes:[{time:0,value:-20},{time:3,value:100}]}));
    const parts=api.scene3d.listParts(imported);
    if(parts.length)check(api.scene3d.setPart(imported,parts[0].key,{x:.35,scale:1.3}));
    PM.L('other').p['position.y'].expr='value + sin(time * 2) * 0.2';PM.touch();
    const times=[0,.17,.49,1.13,2.81];
    const frames=async()=>{const out:string[]=[];for(const t of times){await PM.prepareFrame(t,{mblur:false});out.push(PM.renderFrameTo(t,320,180,{mblur:false}).toDataURL());}return out;};
    const before=await frames(),channels=JSON.stringify(PM.proj.layers.map((l:any)=>[l.id,l.p,l.d]));PM.time=.49;PM.touch();
    const bounds=api.scene3d.geometryBounds([imported,'other'],PM.time),count=PM.hist.list().length;
    const created=check(await api.groups.createBoundsNull([imported,'other'],0)).result.nullIds;
    const keptChannels=JSON.stringify(PM.proj.layers.filter((l:any)=>!created.includes(l.id)).map((l:any)=>[l.id,l.p,l.d]))===channels;
    const after=await frames(),oneUndo=PM.hist.list().length===count+1;
    PM.hist.undo();const undo=await frames(),removed=created.every((id:string)=>!PM.L(id));
    PM.hist.redo();const redo=await frames();
    const saved=PM.serialize();PM.replaceProject(PM.hydrateProject(PM.deserialize(saved)));await PM.prepareFrame(.49,{mblur:false});
    const reopened=await frames();
    const afterBounds=api.scene3d.geometryBounds([imported,'other'],.49);
    const node=PM.L(created[0]);
    return {bounds,afterBounds,parts:parts.length,created,keptChannels,oneUndo,removed,
      identical:JSON.stringify(before)===JSON.stringify(after),undo:JSON.stringify(before)===JSON.stringify(undo),
      redo:JSON.stringify(before)===JSON.stringify(redo),reopened:JSON.stringify(before)===JSON.stringify(reopened),
      groupStill3D:api.scene3d.isGroup(group.id),null3D:node.threeD,errors:[...PM.GL.errors.entries()],
      pivot:['x','y','z'].map(axis=>node.p[`anchor.${axis}`].v)};
  },modelGlb());
  expect(result.created).toHaveLength(1);expect(result.parts).toBeGreaterThan(0);
  for(const flag of ['keptChannels','oneUndo','removed','identical','undo','redo','reopened','groupStill3D','null3D'] as const)expect(result[flag],flag).toBe(true);
  expect(result.afterBounds).toEqual(result.bounds);expect(result.errors).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('extruded and native 3D artwork keeps its animated frames in mixed parent spaces',async({session})=>{
  test.setTimeout(90_000);await session.openEditor();
  const result=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('bounds-native-proof');PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({name:'Native bounds',w:640,h:360,fps:30,dur:3}));
    const check=(r:any)=>{if(!r.ok)throw new Error(r.message);return r.data;};
    for(const [id,x] of [['a',240],['b',400]] as const){
      check(PM.Edit.apply({type:'add_layer',id,layerType:'shape',content:{shape:'star',w:100,h:80,color:'#48D4CC'},properties:{'position.x':x,'position.y':180}}));
      check(PM.Edit.apply({type:'add_layer',id:`parent-${id}`,layerType:'null'}));
      check(PM.Edit.apply({type:'set_layer',target:id,patch:{parent:`parent-${id}`,threeD:true}}));
      PM.L(`parent-${id}`).p.rotation.expr='sin(time) * 20';
    }
    check(api.scene3d.setExtrusion('a',{enabled:true,depth:40}));
    PM.touch();const times=[0,.37,1.19,2.83];
    const frames=async()=>{const out:string[]=[];for(const time of times){await PM.prepareFrame(time,{mblur:false});out.push(PM.renderFrameTo(time,320,180,{mblur:false}).toDataURL());}return out;};
    const before=await frames();PM.time=.37;PM.touch();const bounds=api.scene3d.geometryBounds(['a','b']);
    const created=check(api.groups.createBoundsNull(['a','b'],8)).result.nullIds,after=await frames();
    const saved=PM.serialize();PM.replaceProject(PM.hydrateProject(PM.deserialize(saved)));const reopened=await frames();
    PM.L('a').lock=true;PM.touch();const lockedBefore=PM.serialize(),failure=api.groups.createBoundsNull(['a','b'],4);
    return {bounds,created,identical:JSON.stringify(before)===JSON.stringify(after),reopened:JSON.stringify(before)===JSON.stringify(reopened),
      failure,atomic:lockedBefore===PM.serialize(),errors:[...PM.GL.errors.entries()]};
  });
  expect(result.created).toHaveLength(1);expect(result.bounds.z1-result.bounds.z0).toBeGreaterThan(.1);
  expect(result.identical).toBe(true);expect(result.reopened).toBe(true);expect(result.failure.ok).toBe(false);expect(result.atomic).toBe(true);
  expect(result.errors).toEqual([]);expect(session.diagnostics.pageErrors).toEqual([]);
});

test('one null controls combined SVG and model bounds and wraps existing animated nulls',async({session})=>{
  test.setTimeout(120_000);
  await session.openEditor();
  const result=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('shared-bounds-proof'),viewer=PM.Kernel.services.get('viewer');PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({name:'Shared SVG and model controller',w:640,h:360,dur:3}));
    const check=(r:any)=>{if(!r.ok)throw Error(r.message);return r.data;};
    check(api.scene3d.edit({operation:'add_camera',camera:{projection:'orthographic',p:{x:0,y:0,z:6,zoom:1}}}));
    check(api.scene3d.edit({operation:'add_object',object:{id:'model',source:{primitive:'box'},p:{x:-1,y:.5}}}));
    const group=PM.groupLayers(['model']);group.p.rotation.expr='time * 17';
    await PM.importFiles([new File(['<svg xmlns="http://www.w3.org/2000/svg" width="100" height="80"><path fill="#e60012" d="M0 0H100V80H0Z"/></svg>'],'logo.svg',{type:'image/svg+xml'})]);
    const logo=PM.proj.layers.find((l:any)=>l.type==='shape');logo.p['position.x'].v=440;logo.p['position.y'].v=140;logo.p.rotation.expr='time * 9';PM.touch();
    const times=[.1,.8,1.7,2.8],frames=async()=>{const out=[];for(const t of times){await PM.prepareFrame(t,{mblur:false});out.push(PM.renderFrameTo(t,320,180,{mblur:false}).toDataURL());}return out;};
    const before=await frames();PM.time=.8;PM.touch();const bounds=api.scene3d.geometryBounds([logo.id,group.id]);
    const ids=check(api.groups.createBoundsNull([logo.id,group.id],7)).result.nullIds,first=PM.L(ids[0]);
    const after=await frames();PM.time=.8;PM.touch();viewer.fit=true;viewer.layout();PM.selectLayers([first.id]);PM.invalidate();await new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r())));
    const marker=viewer.nullMarkers().find((m:any)=>m.id===first.id),stage=viewer.stage.getBoundingClientRect(),hover=viewer.sceneGizmo.hover(stage.left+marker.x,stage.top+marker.y);
    const linked=logo.parent===first.id&&group.parent===first.id;
    first.p.rotation.expr='time * 11';first.p['position.x'].expr='value + time * .1';PM.touch();
    const animated=await frames();PM.time=.8;PM.touch();
    const nested=check(api.groups.createBoundsNull([logo.id,group.id],4)).result.nullIds;
    const wrapped=PM.L(first.id).parent===nested[0]&&logo.parent===first.id&&group.parent===first.id;
    const nestedFrames=await frames(),saved=PM.serialize();PM.replaceProject(PM.hydrateProject(PM.deserialize(saved)));const reopened=await frames();
    return {ids,nested,linked,wrapped,hover,marker,bounds,unchanged:JSON.stringify(before)===JSON.stringify(after),nestedUnchanged:JSON.stringify(animated)===JSON.stringify(nestedFrames),reopened:JSON.stringify(animated)===JSON.stringify(reopened),errors:[...PM.GL.errors.entries()]};
  });
  expect(result.ids).toHaveLength(1);expect(result.nested).toHaveLength(1);
  for(const key of ['linked','wrapped','hover','unchanged','nestedUnchanged','reopened'] as const)expect(result[key],key).toBe(true);
  expect(result.errors).toEqual([]);expect(session.diagnostics.pageErrors).toEqual([]);
});
