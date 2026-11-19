import {test,expect} from './helpers/app';
import {writeFile} from 'node:fs/promises';

test('a reflective floor mirrors scene objects, updates their motion and preserves the document',async({session},info)=>{
  test.setTimeout(180_000);await session.openEditor();const {page}=session;
  await page.evaluate(()=>{
    const PM=(window as any).PM;PM.pause();PM.replaceProject(PM.mkProject({w:640,h:480,dur:3}));
    const api=PM.Kernel.api('reflection-floor-test').scene3d;
    api.edit({operation:'add_camera',id:'camera',camera:{p:{x:3,y:3,z:6,targetY:.4}}});
    api.edit({operation:'add_light',light:{id:'sun',type:'sun',p:{x:2,y:5,z:4,intensity:3}}});
    api.edit({operation:'add_light',light:{id:'env',type:'environment',p:{intensity:50}}});
    api.edit({operation:'set_environment',patch:{shadows:false,p:{ambient:.2}}});
    api.edit({operation:'add_object',object:{id:'floor',source:{primitive:'plane',parameters:{width:8,height:8}},p:{rx:-90},material:{p:{color:'#FFFFFF',metalness:1,roughness:.03}}}});
    api.edit({operation:'add_object',object:{id:'red',source:{primitive:'box'},p:{y:1},material:{p:{color:'#FF0000',roughness:1,emissive:'#FF0000',emissiveIntensity:1}}}});
    PM.selectLayers(['floor']);PM.invalidate();
  });
  const capture=async()=>Buffer.from(await page.evaluate(()=>{
    const pixels=(window as any).PM.GL.renderToPixels(0,640,480,{transparent:true,mblur:false});let binary='';
    for(let i=0;i<pixels.length;i+=16384)binary+=String.fromCharCode(...pixels.subarray(i,i+16384));
    return btoa(binary);
  }),'base64');
  const roughness=async(value:number)=>page.evaluate(value=>{
    const result=(window as any).PM.Kernel.api('reflection-floor-test').scene3d.setMaterialProperties('floor',null,{roughness:value});if(!result.ok)throw new Error(result.message);
  },value);
  const before=await page.evaluate(()=>{const PM=(window as any).PM;return {document:PM.serialize(),history:PM.hist.list().length};});
  const reflected=await capture();expect((await capture()).every((v,i)=>v===reflected[i])).toBe(true);
  expect(await page.evaluate(()=>{const PM=(window as any).PM;return {document:PM.serialize(),history:PM.hist.list().length};})).toEqual(before);
  await roughness(1);const matte=await capture();
  const reflection=(pixels:Uint8Array,baseline:Uint8Array)=>{
    let count=0,x=0;
    for(let i=0;i<pixels.length;i+=4){
      const red=(data:Uint8Array)=>data[i]!>50 && data[i]!>data[i+1]!*1.8 && data[i]!>data[i+2]!*1.8;
      if(red(pixels)&&!red(baseline)){count++;x+=(i/4)%640;}
    }
    return {count,x:x/Math.max(1,count)};
  };
  const first=reflection(reflected,matte);expect(first.count).toBeGreaterThan(150);
  await roughness(.4);const blurred=await capture();let sharpContrast=0,blurredContrast=0;
  for(let i=0;i<reflected.length;i+=4){
    if(reflected[i]!>50 && reflected[i]!>reflected[i+1]!*1.8 && reflected[i]!>reflected[i+2]!*1.8 && !(matte[i]!>matte[i+1]!*1.8 && matte[i]!>matte[i+2]!*1.8)){
      sharpContrast+=reflected[i]!-reflected[i+1]!;blurredContrast+=blurred[i]!-blurred[i+1]!;
    }
  }
  expect(blurredContrast).toBeLessThan(sharpContrast*.9);await roughness(1);
  await page.evaluate(()=>{const PM=(window as any).PM;PM.Edit.apply({type:'set_property',target:'red',path:'position.x',value:-1.5});});
  const movedMatte=await capture();await roughness(.03);const moved=await capture(),second=reflection(moved,movedMatte);
  expect(second.count).toBeGreaterThan(150);expect(Math.abs(second.x-first.x)).toBeGreaterThan(25);
  await page.evaluate(()=>{const result=(window as any).PM.Kernel.api('reflection-floor-test').scene3d.edit({operation:'set_environment',patch:{shadows:true}});if(!result.ok)throw new Error(result.message);});
  expect(reflection(await capture(),movedMatte).count).toBeGreaterThan(100);
  const png=await page.evaluate(()=>{
    const pixels=(window as any).PM.GL.renderToPixels(0,640,480,{transparent:true,mblur:false});
    const canvas=document.createElement('canvas');canvas.width=640;canvas.height=480;
    const flipped=new Uint8ClampedArray(pixels.length);for(let y=0;y<480;y++)flipped.set(pixels.subarray(y*640*4,(y+1)*640*4),(479-y)*640*4);
    canvas.getContext('2d')!.putImageData(new ImageData(flipped,640,480),0,0);return canvas.toDataURL().split(',')[1];
  });await writeFile(info.outputPath('floor-reflection.png'),Buffer.from(png!,'base64'));
  expect(session.diagnostics.console.filter(record=>/Shader Error|VALIDATE_STATUS|ERROR: 0:/.test(record.text))).toEqual([]);
});

test('a chrome sphere reflects nearby objects and the floor, including offscreen objects',async({session},info)=>{
  test.setTimeout(120_000);await session.openEditor();const {page}=session;
  await page.evaluate(()=>{
    const PM=(window as any).PM;PM.pause();PM.replaceProject(PM.mkProject({w:480,h:480,dur:3}));
    const api=PM.Kernel.api('reflection-sphere-test').scene3d;
    api.edit({operation:'add_camera',id:'camera',camera:{projection:'orthographic',p:{x:0,y:0,z:5}}});
    api.edit({operation:'add_light',light:{id:'sun',type:'sun',p:{intensity:3}}});
    api.edit({operation:'add_light',light:{id:'env',type:'environment'}});
    api.edit({operation:'set_environment',patch:{shadows:false,p:{ambient:.3}}});
    api.edit({operation:'add_object',object:{id:'sphere',source:{primitive:'sphere',parameters:{radius:.8}},material:{p:{color:'#FFFFFF',metalness:1,roughness:.03}}}});
    api.edit({operation:'add_object',object:{id:'floor',source:{primitive:'plane',parameters:{width:12,height:12}},p:{y:-1,rx:-90},material:{p:{color:'#0000FF',roughness:.2,emissive:'#0000FF',emissiveIntensity:1}}}});
    api.edit({operation:'add_object',object:{id:'red',source:{primitive:'box',parameters:{width:.7,height:.7,depth:.7}},p:{x:1.5,y:.3,z:1},material:{p:{color:'#FF0000',roughness:1,emissive:'#FF0000',emissiveIntensity:2}}}});
    PM.selectLayers(['sphere']);PM.invalidate();
  });
  const capture=()=>page.evaluate(()=>{
    const PM=(window as any).PM,pixels=PM.GL.renderToPixels(0,480,480,{transparent:true,mblur:false});let red=0,blue=0;
    for(let y=184;y<296;y++)for(let x=184;x<296;x++){
      if(Math.hypot(x-240,y-240)>54)continue;
      const i=(y*480+x)*4;
      if(pixels[i]>50 && pixels[i]>pixels[i+1]*1.8 && pixels[i]>pixels[i+2]*1.8)red++;
      if(pixels[i+2]>50 && pixels[i+2]>pixels[i]*1.8 && pixels[i+2]>pixels[i+1]*1.8)blue++;
    }
    const canvas=document.createElement('canvas');canvas.width=canvas.height=480;
    const flipped=new Uint8ClampedArray(pixels.length);for(let y=0;y<480;y++)flipped.set(pixels.subarray(y*480*4,(y+1)*480*4),(479-y)*480*4);
    canvas.getContext('2d')!.putImageData(new ImageData(flipped,480,480),0,0);return {red,blue,png:canvas.toDataURL().split(',')[1]};
  });
  const visible=await capture();expect(visible.red).toBeGreaterThan(30);expect(visible.blue).toBeGreaterThan(100);
  await writeFile(info.outputPath('sphere-reflection.png'),Buffer.from(visible.png!,'base64'));
  await page.evaluate(()=>{(window as any).PM.Edit.apply({type:'set_layer',target:'red',patch:{visible:false}});});
  expect((await capture()).red).toBe(0);
  await page.evaluate(()=>{const PM=(window as any).PM;PM.Edit.apply([{type:'set_layer',target:'red',patch:{visible:true}},{type:'set_property',target:'red',path:'opacity',value:0}]);});
  expect((await capture()).red).toBe(0);
  await page.evaluate(()=>{const PM=(window as any).PM;PM.Edit.apply([{type:'set_property',target:'red',path:'opacity',value:100},{type:'set_property',target:'red',path:'position.x',value:3.5},{type:'set_property',target:'red',path:'position.z',value:3}]);});
  expect((await capture()).red).toBeGreaterThan(10);
  expect(session.diagnostics.console.filter(record=>/Shader Error|VALIDATE_STATUS|ERROR: 0:/.test(record.text))).toEqual([]);
});
