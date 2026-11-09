import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

const shots='/tmp/pm-shots/studio3d/laneK';
for(const fixture of ['tiny-static','tiny-animated'])test(`FBX ${fixture} imports through the existing model flow and renders`,async({session})=>{
  test.setTimeout(120_000);await mkdir(shots,{recursive:true});await session.openEditor();
  const bytes=[...await readFile(path.resolve(repoRoot,`../../engine/hosts/studio-electron/fixtures/${fixture}.fbx`))];
  const result=await session.page.evaluate(async({bytes,fixture})=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({name:'FBX regression',w:640,h:360,fps:30,dur:2}));
    const api=PM.Kernel.api('fbx-e2e').scene3d;
    api.edit({operation:'add_camera',camera:{p:{x:0,y:.2,z:4,targetY:.2}}});
    api.edit({operation:'add_light',light:{type:'area',p:{x:0,y:2,z:4,intensity:4,width:4,height:4}}});
    const begin=performance.now(),imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],`${fixture}.fbx`));
    if(!imported.ok)throw new Error(imported.message);
    const importMs=performance.now()-begin,layer=PM.proj.layers.find((l:any)=>l.d?.data?.object);
    const asset=PM.assets.get(layer.d.data.object.source.assetId),parts=api.listParts(layer.id);
    const captures=[];
    for(const time of [0,.5]){
      await PM.prepareFrame(time,{mblur:false});const canvas=PM.renderFrameTo(time,640,360,{mblur:false}),pixels=canvas.getContext('2d').getImageData(0,0,640,360).data;
      let red=0,xsum=0;for(let i=0;i<pixels.length;i+=4)if(pixels[i]>pixels[i+1]+20&&pixels[i]>pixels[i+2]+20){red++;xsum+=(i/4)%640;}
      captures.push({time,png:canvas.toDataURL('image/png'),red,center:red?xsum/red:0});
    }
    return {id:layer.id,format:asset.format,clips:asset.animations.length,parts:parts.map((p:any)=>p.name),importMs,captures,status:PM.Studio.status()};
  },{bytes,fixture});
  expect(result.format).toBe('glb');expect(result.clips).toBe(fixture==='tiny-animated'?1:0);
  if(process.platform==='darwin')expect(result.status.backend).toBe('native-studio');
  expect(result.parts).toEqual(expect.arrayContaining(['Triangle','Joint','Camera','KeyLight']));
  for(const capture of result.captures){expect(capture.red).toBeGreaterThan(500);await writeFile(`${shots}/fbx-${fixture}-${capture.time}.png`,Buffer.from(capture.png.split(',')[1],'base64'));}
  if(fixture==='tiny-animated')expect(result.captures[1]!.center-result.captures[0]!.center).toBeGreaterThan(30);
  const file=path.join(session.userData,`${fixture}.pmv`);
  await session.app.evaluate(({dialog},filePath)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath});dialog.showOpenDialog=async()=>({canceled:false,filePaths:[filePath]});},file);
  const reopened=await session.page.evaluate(async id=>{
    const PM=(window as any).PM;await PM.saveProject();await PM.openProject();await PM.prepareFrame(.5,{mblur:false});
    const layer=PM.proj.layers.find((layer:any)=>layer.id===id),asset=PM.assets.get(layer.d.data.object.source.assetId);
    const canvas=PM.renderFrameTo(.5,640,360,{mblur:false}),pixels=canvas.getContext('2d').getImageData(0,0,640,360).data;
    let red=0;for(let i=0;i<pixels.length;i+=4)if(pixels[i]>pixels[i+1]+20&&pixels[i]>pixels[i+2]+20)red++;
    return {format:asset.format,clips:asset.animations.length,red,status:PM.Studio.status()};
  },result.id);
  expect(reopened.format).toBe('glb');expect(reopened.clips).toBe(result.clips);expect(reopened.red).toBeGreaterThan(500);
  if(process.platform==='darwin')expect(reopened.status.backend).toBe('native-studio');
  expect(session.diagnostics.pageErrors).toEqual([]);
  await writeFile(`${shots}/fbx-${fixture}-metrics.json`,JSON.stringify({importMs:result.importMs,parts:result.parts,clips:result.clips,captures:result.captures.map(({time,red,center})=>({time,red,center})),reopened},null,2));
});
