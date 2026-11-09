import {mkdir,writeFile} from 'node:fs/promises';
import {expect,test} from './helpers/app';
const folder=process.env.PM_ENVIRONMENT_SHOTS || '/tmp/pm-shots/studio3d/laneF';
test.beforeEach(async()=>{await mkdir(folder,{recursive:true});});
test('agent authors a fog-lit room and a scattered golden-hour landscape',async({session})=>{
  test.setTimeout(240000);await session.openEditor();
  const page=session.page;
  const build=async(kind:'room'|'landscape')=>page.evaluate(async(kind)=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();PM.replaceProject(PM.mkProject({name:`Environment ${kind}`,w:640,h:360,fps:24,dur:1}));
    const baseRevision=PM.proj.revision,runId=`laneF-${kind}`;
    const call=async(tool:string,args:any={})=>{const result=await PM.AgentHarness.test.handleLiveAgentTool({runId,callId:crypto.randomUUID(),tool,arguments:args,baseRevision});if(!result.ok)throw new Error(JSON.stringify(result));return result;};
    await call('get_project_state');
    const edit=(args:any)=>call('edit_3d',args);
    await edit({operation:'set_world',id:'world',world:{preset:'golden-hour',p:{elevation:kind==='room'?18:7,azimuth:kind==='room'?-75:-65,intensity:kind==='room'?.35:.7,sunIntensity:1,haze:2.7,fogDensity:kind==='room'?.002:.003,fogDistance:kind==='room'?30:150,fogHeightFalloff:.14,fogColor:'#E7D2B8',fogAnisotropy:.35}}});
    await edit({operation:'add_camera',id:'camera',camera:{p:kind==='room'?{x:2.7,y:1.7,z:2.1,targetX:-2,targetY:1.3,targetZ:-.7,fov:67}:{x:18,y:12,z:23,targetX:0,targetY:0,targetZ:0,fov:45}}});
    await edit({operation:'set_environment',patch:{samples:32,p:{ambient:0},background:null}});
    const box=async(id:string,w:number,h:number,d:number,x:number,y:number,z:number,color:string)=>edit({operation:'add_object',object:{id,name:id,source:{primitive:'box',parameters:{width:w,height:h,depth:d}},p:{x,y,z},material:{p:{color,roughness:.75,metalness:0}}}});
    if(kind==='room'){
      await box('floor',6,.16,5,0,-.08,0,'#A18461');
      await box('ceiling',6,.12,5,0,3.06,0,'#D2C6B1');
      await box('back_wall',6,3,.15,0,1.5,-2.5,'#D2C6B1');
      await box('window_front_pier',.15,3,1.1,-3,1.5,1.95,'#D2C6B1');
      await box('window_back_pier',.15,3,1.4,-3,1.5,-1.8,'#D2C6B1');
      await box('window_sill',.15,.75,2.5,-3,.375,.25,'#D2C6B1');
      await box('window_lintel',.15,.45,2.5,-3,2.775,.25,'#D2C6B1');
      await box('window_mullion',.2,1.8,.055,-3,1.65,.25,'#63503D');
      await box('sofa',2.4,.65,.8,1,.4,-1.55,'#774E37');
      await box('sofa_back',2.4,.7,.2,1,.9,-1.85,'#774E37');
      await box('table',1.5,.15,.9,-.3,.65,.2,'#50402D');
      for(const x of [-.85,.25])for(const z of [-.1,.5])await box(`leg_${x<0?'a':'b'}_${z<0?'a':'b'}`,.08,.6,.08,x,.3,z,'#50402D');
      await edit({operation:'add_object',object:{id:'vase',source:{primitive:'sphere',parameters:{radius:.23}},p:{x:-.4,y:.95,z:.15,sy:1.5},material:{p:{color:'#52654C',roughness:.35}}}});
      await edit({operation:'add_fog',id:'fog',fog:{p:{density:.08,width:6,height:3,depth:5,anisotropy:.55,color:'#E3D5BE'}}});
      await edit({operation:'update_object',target:'fog',patch:{p:{y:1.5}}});
    }else{
      await edit({operation:'add_terrain',id:'terrain',terrain:{p:{width:34,depth:34,resolution:64,amplitude:3.5,frequency:.12,seed:41,octaves:5},regions:[{p:{minSlope:32,color:'#8C7D64'}},{p:{minHeight:1.4,color:'#A3A078'}},{p:{maxHeight:1.4,color:'#687348'}}]}});
      await edit({operation:'add_object',object:{id:'tree',source:{primitive:'cone',parameters:{radius:.48,height:2,segments:8}},p:{x:100},material:{p:{color:'#334A2E',roughness:.85}}}});
      await call('apply_commands',{commands:[{type:'set_layer',target:'tree',patch:{visible:false}}]});
      await edit({operation:'add_scatter',id:'trees',scatter:{model:'tree',surface:'terrain',p:{count:450,seed:71,scaleMin:.6,scaleMax:1.4,minSlope:0,maxSlope:32,minHeight:-2.5,maxHeight:2.6}}});
    }
    const state=JSON.parse((await call('get_3d_scene')).content[0].text);
    await call('render_frames',{times:[0],width:640});
    PM.agentFrameCapture=true;await PM.prepareFrame(0,{mblur:false,width:640,height:360});const canvas=PM.renderFrameTo(0,640,360,{mblur:false});const png=canvas.toDataURL('image/png');PM.agentFrameCapture=false;
    const status=PM.Studio.status(),passFiles:any[]=[];
    if(kind==='landscape')for await(const output of PM.Export.renderPasses(0,160,90,['beauty','depth','normal','albedo','object_id','material_id','shadow'])){passFiles.push({pass:output.pass,bytes:Array.from(new Uint8Array(await output.blob.arrayBuffer())),metadata:output.metadata});}
    await call('__finish_run',{commit:true});
    return {state,png,status,passFiles};
  },kind);
  for(const kind of ['room','landscape'] as const){
    const result=await build(kind);await writeFile(`${folder}/${kind}.png`,Buffer.from(result.png.split(',')[1],'base64'));
    for(const pass of result.passFiles){expect(pass.bytes[24]).toBe(16);await writeFile(`${folder}/landscape-${pass.pass}-16bit.png`,new Uint8Array(pass.bytes));}
    expect(result.state.world.p.elevation.v).toBe(kind==='room'?18:7);expect(result.status.ready).toBe(true);expect(result.status.failure).toBeNull();if(process.platform==='darwin')expect(result.status.backend,JSON.stringify({status:result.status,nativeWarnings:session.diagnostics.console.filter(m=>m.text.includes('Native Studio'))})).toBe(process.env.PM_STUDIO_TEST_WEBGPU?'webgpu':'native-studio');
    expect(result.state.channels.some((p:any)=>p.path==='world.fogDensity')).toBe(true);
    if(kind==='room')expect(result.state.fogVolumes).toHaveLength(1);else expect(result.state.scatters).toHaveLength(1);
  }
  await writeFile(`${folder}/captions.md`,'# Agent-built environments\n\nroom.png — A room assembled with edit_3d only: a window opening, golden sun, height fog and a local volume, furniture and a vase. Final frame at 640×360.\n\nlandscape.png — Seeded procedural terrain with slope/height material regions and a seeded scatter of 450 simple conifer prototypes, under a golden-hour sky with height fog. Final frame at 640×360. These are simple test assets, not production foliage.\n');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('HDR and hidden layer sources survive environment edits and height sampling',async({session})=>{
  test.setTimeout(120000);await session.openEditor();
  const result=await session.page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.replaceProject(PM.mkProject({w:320,h:180,fps:24,dur:1}));
    const baseRevision=PM.proj.revision,runId='laneF-sources';
    const call=async(tool:string,args:any={})=>{const r=await PM.AgentHarness.test.handleLiveAgentTool({runId,callId:crypto.randomUUID(),tool,arguments:args,baseRevision});if(!r.ok)throw new Error(JSON.stringify(r));return r;};
    await call('get_project_state');
    const hdr=new Uint8Array([...new TextEncoder().encode('#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\n'),128,64,32,132,32,64,128,130]);
    const asset=await PM.assets.add(new File([hdr],'test-environment.hdr'),{silent:true});
    await call('edit_3d',{operation:'set_world',id:'world',world:{mode:'hdri',source:asset.id,p:{visible:true}}});
    await call('edit_3d',{operation:'add_object',object:{id:'ball',source:{primitive:'sphere'},material:{p:{roughness:.2,metalness:.8}}}});
    await call('edit_3d',{operation:'add_camera',id:'camera',camera:{p:{x:0,y:1,z:5,targetY:0}}});
    await call('edit_3d',{operation:'set_environment',patch:{samples:16,p:{ambient:0}}});
    const capture=async()=>{PM.agentFrameCapture=true;await PM.prepareFrame(0,{width:320,height:180,mblur:false});const c=PM.renderFrameTo(0,320,180,{mblur:false});PM.agentFrameCapture=false;return c.toDataURL('image/png');};
    const hdri=await capture();
    await call('apply_commands',{commands:[{type:'add_layer',id:'sky',layerType:'solid',visible:false,content:{w:32,h:16,color:'#FF3300'}}]});
    await call('edit_3d',{operation:'set_world',patch:{mode:'hdri',source:{layerId:'sky'}}});const red=await capture();
    await call('apply_commands',{commands:[{type:'set_content',target:'sky',patch:{color:'#0033FF'}}]});const blue=await capture();
    await call('edit_3d',{operation:'add_terrain',id:'height',terrain:{heightSource:{layerId:'sky'},p:{resolution:4,width:4,depth:4,amplitude:2}}});
    const state=JSON.parse((await call('get_3d_scene')).content[0].text);await capture();
    const status=PM.Studio.status(),saved=PM.deserialize(PM.serialize());
    await call('__finish_run',{commit:true});
    return {hdri,red,blue,hdrPeak:Math.max(...asset.environmentMap.pixels.map((p:any)=>p.x)),source:state.world.source,terrain:state.terrains[0].terrain.heightSource,saved:saved.layers.find((l:any)=>l.id==='world').d.data.object.world.source,status};
  });
  expect(result.hdrPeak).toBeGreaterThan(1);expect(result.red).not.toBe(result.blue);expect(result.hdri).not.toBe(result.red);
  expect(result.source).toEqual({layerId:'sky'});expect(result.saved).toEqual(result.source);expect(result.terrain).toEqual({layerId:'sky'});expect(result.status.failure).toBeNull();if(process.platform==='darwin')expect(result.status.backend,JSON.stringify({status:result.status,nativeWarnings:session.diagnostics.console.filter(m=>m.text.includes('Native Studio'))})).toBe(process.env.PM_STUDIO_TEST_WEBGPU?'webgpu':'native-studio');
  for(const [name,png] of [['hdri',result.hdri],['layer-red',result.red],['layer-blue',result.blue]])await writeFile(`${folder}/${name}.png`,Buffer.from(png.split(',')[1],'base64'));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('3D artwork casts alpha shadows and exports actual 16-bit sequences',async({session})=>{
  test.setTimeout(120000);await session.openEditor();const output=`${session.userData}/passes`;await mkdir(output,{recursive:true});
  await session.app.evaluate(({dialog},output)=>{dialog.showOpenDialog=async()=>({canceled:false,filePaths:[output]});},output);
  const proof=await session.page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.replaceProject(PM.mkProject({name:'Shadow',w:320,h:180,fps:1,dur:1}));
    const api=PM.Kernel.api('laneF-shadow').scene3d,edit=(a:any)=>{const r=api.edit(a);if(!r.ok)throw new Error(r.message);};
    edit({operation:'add_camera',id:'camera',camera:{p:{x:0,y:0,z:6}}});
    edit({operation:'add_object',object:{id:'wall',source:{primitive:'plane',parameters:{width:7,height:4}},p:{z:-1},material:{p:{color:'#FFFFFF',roughness:1,metalness:0}}}});
    edit({operation:'add_light',light:{type:'parallel',p:{x:-2,y:3,z:5,intensity:2}}});
    edit({operation:'set_environment',patch:{samples:32,p:{ambient:.05}}});
    const r=PM.Edit.apply({type:'add_layer',id:'card',layerType:'shape',content:{shape:'ellipse',w:100,h:100,fill:'#FFFFFF',color:'#FFFFFF'},properties:{'position.x':160,'position.y':90}});if(!r.ok)throw new Error(r.message);
    PM.Edit.apply({type:'set_layer',target:'card',patch:{threeD:true}});
    edit({operation:'set_material_options',target:'card',patch:{castsShadows:'only',acceptsLights:true}});
    const capture=async()=>{PM.agentFrameCapture=true;await PM.prepareFrame(0,{mblur:false,width:320,height:180});const c=PM.renderFrameTo(0,320,180,{mblur:false});PM.agentFrameCapture=false;return c.toDataURL('image/png');};
    const casts=await capture();edit({operation:'set_material_options',target:'card',patch:{castsShadows:'off'}});const off=await capture();
    edit({operation:'set_camera',patch:{p:{near:2,far:12}}});
    edit({operation:'set_material_options',target:'card',patch:{castsShadows:'only'}});
    const result=await PM.Export.run({format:'png',passes:['beauty','depth','normal','albedo','object_id','material_id','shadow'],scale:1,fps:1,range:'all',mblur:false,audio:false,reportPath:true});
    return {casts,off,result,status:PM.Studio.status()};
  });
  expect(proof.casts).not.toBe(proof.off);expect(proof.result.error).toBeUndefined();expect(proof.result.cancelled).toBe(false);
  const {readdir,readFile}=await import('node:fs/promises');const files=await readdir(output);expect(files.filter(f=>f.endsWith('.png'))).toHaveLength(7);
  for(const file of files.filter(f=>f.endsWith('.png'))){const png=await readFile(`${output}/${file}`);expect(png[24]).toBe(16);}
  expect(files).toContain('passes_00000.json');
  const {inflateSync}=await import('node:zlib'),depth=await readFile(`${output}/${files.find(f=>f.includes('_depth_'))}`),chunks=[];
  for(let at=8;at<depth.length;){const length=depth.readUInt32BE(at);if(depth.toString('ascii',at+4,at+8)==='IDAT')chunks.push(depth.subarray(at+8,at+8+length));at+=length+12;}
  const pixels=inflateSync(Buffer.concat(chunks)),center=90*(1+320*8)+1+160*8;
  expect(pixels.readUInt16BE(center)/65535).toBeCloseTo(.5,3);
  for(const [name,png] of [['artwork-casts-shadow',proof.casts],['artwork-no-shadow',proof.off]])await writeFile(`${folder}/${name}.png`,Buffer.from(png.split(',')[1],'base64'));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('camera iris shape, rotation and aspect change deterministic Final bokeh',async({session})=>{
  test.setTimeout(120000);await session.openEditor();
  const proof=await session.page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.replaceProject(PM.mkProject({w:320,h:180,fps:24,dur:1}));
    const api=PM.Kernel.api('laneF-iris').scene3d,edit=(a:any)=>{const r=api.edit(a);if(!r.ok)throw new Error(r.message);};
    edit({operation:'add_camera',id:'camera',camera:{p:{x:0,y:0,z:6,fStop:.5,focusDistance:2,irisShape:3,irisRoundness:0,irisAspect:1}}});
    edit({operation:'set_environment',patch:{samples:128,background:'#000000',p:{ambient:0}}});
    for(const [id,x,y,color] of [['red',-.7,.1,'#FF2200'],['green',0,-.35,'#22FF66'],['blue',.7,.25,'#2266FF']])edit({operation:'add_object',object:{id,source:{primitive:'sphere',parameters:{radius:.15,segments:16}},p:{x,y},material:{p:{color:'#000000',emissive:color,emissiveIntensity:100,metalness:0}}}});
    const capture=async()=>{PM.agentFrameCapture=true;await PM.prepareFrame(0,{width:320,height:180,mblur:false});const canvas=PM.renderFrameTo(0,320,180,{mblur:false});PM.agentFrameCapture=false;return canvas.toDataURL('image/png');};
    const triangle=await capture(),repeat=await capture();edit({operation:'set_camera',patch:{p:{irisRotation:45}}});const rotated=await capture();edit({operation:'set_camera',patch:{p:{irisShape:0,irisAspect:1.8}}});const anamorphic=await capture();
    return {triangle,repeat,rotated,anamorphic,status:PM.Studio.status()};
  });
  expect(proof.repeat).toBe(proof.triangle);expect(proof.rotated).not.toBe(proof.triangle);expect(proof.anamorphic).not.toBe(proof.rotated);expect(proof.status.failure).toBeNull();if(process.platform==='darwin')expect(proof.status.backend).toBe(process.env.PM_STUDIO_TEST_WEBGPU?'webgpu':'native-studio');
  for(const [name,png] of [['iris-triangle',proof.triangle],['iris-rotated',proof.rotated],['iris-anamorphic',proof.anamorphic]])await writeFile(`${folder}/${name}.png`,Buffer.from(png.split(',')[1],'base64'));
  expect(session.diagnostics.pageErrors).toEqual([]);
});
