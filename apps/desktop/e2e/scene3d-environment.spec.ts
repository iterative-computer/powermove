import { expect, test } from './helpers/app';
import { hdrPanorama, exrPanorama, hdrAreaLightPanorama } from '../src/renderer/src/core/scene3d/__fixtures__/environment';

test('Environment Light is a timeline light with its own source, intensity and XYZ rotation', async ({session},info) => {
  test.setTimeout(120_000);await session.openEditor();const page=session.page;
  const setup=await page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();PM.replaceProject(PM.mkProject({w:640,h:360,dur:3,fps:30}));
    const api=PM.Kernel.api('environment-e2e').scene3d;
    api.edit({operation:'add_camera',id:'camera',camera:{p:{x:0,y:0,z:3}}});
    api.edit({operation:'add_object',object:{id:'sphere',source:{primitive:'sphere'},material:{p:{color:'#FFFFFF',metalness:1,roughness:.12}}}});
    api.edit({operation:'add_light',light:{id:'dark',type:'sun',p:{intensity:0}}});
    api.edit({operation:'set_environment',target:'camera',patch:{p:{ambient:0}}});
    const camera=JSON.stringify(PM.L('camera').d),id=await PM.cmd('3d.environment');
    if(PM.L(id)?.d.data.light.type!=='environment')throw new Error('Did not create an Environment Light layer');
    if(PM.proj.layers.filter((l:any)=>l.d?.data?.camera).length!==1)throw new Error('Environment Light added a camera');
    PM.selectLayers([id]);return {id,camera};
  });
  const source=page.getByRole('combobox',{name:'Source',exact:true}),type=page.getByRole('combobox',{name:'Type',exact:true});
  await expect(type).toContainText('Environment');await expect(source).toContainText('Default');
  const importMap=async(name:string,bytes:Uint8Array)=>{
    await expect(page.locator('.pm-menu')).toHaveCount(0);await source.scrollIntoViewIfNeeded();
    const chooser=page.waitForEvent('filechooser');await source.click();await page.getByRole('option',{name:'Import HDR / EXR…',exact:true}).click();
    await (await chooser).setFiles({name,mimeType:'application/octet-stream',buffer:Buffer.from(bytes)});await expect(source).toContainText(name);
  };
  await importMap('Studio.hdr',hdrPanorama(1024,512));
  const render=()=>page.evaluate(()=>{const PM=(window as any).PM;const pixels=PM.GL.renderToPixels(0,160,90,{transparent:true,mblur:false});return {rgb:[...pixels].reduce((sum:number,v:number,i:number)=>sum+(i%4===3?0:v),0),pixels:[...pixels]};});
  const lit=await render();expect(lit.rgb).toBeGreaterThan(10000);
  const intensity=page.locator('[data-scene-channel="light.intensity"] input');
  await intensity.click();await intensity.fill('0');await intensity.press('Enter');expect((await render()).rgb).toBeLessThan(lit.rgb*.1);
  await page.keyboard.press('Meta+z');await expect(intensity).toHaveValue('100%');
  const rotation=page.getByRole('spinbutton',{name:'Rotation Y',exact:true});
  await rotation.click();await rotation.fill('90');await rotation.press('Enter');expect((await render()).pixels).not.toEqual(lit.pixels);
  await importMap('Courtyard.exr',exrPanorama());expect((await render()).rgb).toBeGreaterThan(10000);
  for(const axis of ['X','Y','Z'])await expect(page.getByRole('spinbutton',{name:`Rotation ${axis}`,exact:true})).toBeVisible();
  await type.click();await page.getByRole('option',{name:'Point',exact:true}).click();await expect(source).toHaveCount(0);await expect(page.locator('.pm-menu')).toHaveCount(0);
  await type.click();await page.getByRole('option',{name:'Environment',exact:true}).click();await expect(source).toContainText('Courtyard.exr');await expect(page.locator('.pm-menu')).toHaveCount(0);
  for(const theme of ['light','dark']){
    await page.evaluate(theme=>(window as any).PM.theme.apply(theme),theme);await expect(page.locator('.pm-menu')).toHaveCount(0);await source.scrollIntoViewIfNeeded();
    await page.locator('#panel-inspector').screenshot({path:info.outputPath(`environment-${theme}.png`)});
  }
  expect(await page.evaluate(()=>JSON.stringify((window as any).PM.L('camera').d))).toBe(setup.camera);
  const filePath=session.userData+'/environment-light.pmv';
  await session.app.evaluate(({dialog},filePath)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath});dialog.showOpenDialog=async()=>({canceled:false,filePaths:[filePath]});},filePath);
  const beforeSave=await render();
  await page.evaluate(async()=>{const PM=(window as any).PM;if(!await PM.saveProject())throw new Error('Save failed');await PM.openProject();});
  await page.waitForFunction(id=>{const PM=(window as any).PM;return !!PM.assets.get(PM.L(id)?.d?.data?.light?.map)?.environmentTexture;},setup.id);
  await page.evaluate(id=>(window as any).PM.selectLayers([id]),setup.id);
  await expect(source).toContainText('Courtyard.exr');await expect(rotation).toHaveValue('90°');expect((await render()).pixels).toEqual(beforeSave.pixels);
  await page.evaluate(id=>{const PM=(window as any).PM;PM.Edit.apply({type:'set_layer',target:id,patch:{visible:false}});},setup.id);
  expect((await render()).rgb).toBeLessThan(beforeSave.rgb*.1);
  await page.evaluate(()=>(window as any).PM.hist.undo());expect((await render()).pixels).toEqual(beforeSave.pixels);
  expect(session.diagnostics.console.filter(record=>/Shader Error|VALIDATE_STATUS|ERROR: 0:/.test(record.text))).toEqual([]);
});

test('environment images cast shadows without adding direct light and respect every shadow switch',async({session},info)=>{
  test.setTimeout(120_000);await session.openEditor();const {page}=session;
  await page.evaluate(()=>{
    const PM=(window as any).PM;PM.pause();PM.replaceProject(PM.mkProject({w:480,h:480,dur:3}));
    const api=PM.Kernel.api('environment-shadow-e2e').scene3d;
    api.edit({operation:'add_camera',id:'camera',camera:{p:{x:4,y:5,z:6,targetY:.3}}});
    api.edit({operation:'set_environment',patch:{p:{ambient:0}}});
    api.edit({operation:'add_object',object:{id:'floor',source:{primitive:'plane',parameters:{width:8,height:8}},p:{rx:-90},material:{p:{color:'#FFFFFF',roughness:1}}}});
    api.edit({operation:'add_object',object:{id:'box',source:{primitive:'box'},p:{y:.8},material:{p:{color:'#FFFFFF',roughness:1}}}});
    api.edit({operation:'add_light',light:{id:'env',type:'environment'}});PM.selectLayers(['env']);PM.invalidate();
  });
  const source=page.getByRole('combobox',{name:'Source',exact:true}),chooser=page.waitForEvent('filechooser');
  await source.click();await page.getByRole('option',{name:'Import HDR / EXR…',exact:true}).click();
  await (await chooser).setFiles({name:'Area light.hdr',mimeType:'application/octet-stream',buffer:Buffer.from(hdrAreaLightPanorama())});
  await expect(source).toContainText('Area light.hdr');
  const render=()=>page.evaluate(()=>[...(window as any).PM.GL.renderToPixels(0,240,240,{transparent:true,mblur:false})] as number[]);
  const sameFrame=(actual:number[],expected:number[],label:string)=>{
    expect(actual.length).toBe(expected.length);
    expect(actual.reduce((max,v,i)=>Math.max(max,Math.abs(v-expected[i]!)),0),label).toBeLessThanOrEqual(1);
  };
  const edit=async(operation:string,target:string,patch:any)=>page.evaluate(({operation,target,patch})=>{
    const result=(window as any).PM.Kernel.api('environment-shadow-e2e').scene3d.edit({operation,target,patch});if(!result.ok)throw new Error(result.message);
  },{operation,target,patch});
  const shadowed=await render();
  await edit('update_light','env',{castShadow:false});const unshadowed=await render();
  let darker=0,brighter=0;
  for(let i=0;i<shadowed.length;i+=4){
    if(!shadowed[i+3]||!unshadowed[i+3])continue;
    const delta=unshadowed[i]!+unshadowed[i+1]!+unshadowed[i+2]!-shadowed[i]!-shadowed[i+1]!-shadowed[i+2]!;
    if(delta>30)darker++;if(delta< -10)brighter++;
  }
  expect(darker).toBeGreaterThan(100);expect(brighter).toBe(0);
  await edit('update_light','env',{castShadow:true});sameFrame(await render(),shadowed,'light switch restores shadows');
  await edit('set_environment','camera',{shadows:false});sameFrame(await render(),unshadowed,'global switch disables shadows');
  await edit('set_environment','camera',{shadows:true});
  await edit('update_object','box',{castShadow:false});const noCaster=await render();expect(noCaster).not.toEqual(shadowed);
  await edit('update_object','box',{castShadow:true});sameFrame(await render(),shadowed,'caster switch restores shadows');
  await edit('update_object','floor',{receiveShadow:false});expect(await render()).not.toEqual(shadowed);
  await edit('update_object','floor',{receiveShadow:true});
  await page.evaluate(()=>{const PM=(window as any).PM;PM.Edit.apply({type:'set_property',target:'env',path:'rotation.y',value:90});});
  const rotated=await render();expect(rotated).not.toEqual(shadowed);
  await page.evaluate(()=>{
    const PM=(window as any).PM;
    const result=PM.Edit.apply([
      {type:'add_layer',id:'environment-rig',layerType:'null',properties:{'rotation.y':90}},
      {type:'set_layer',target:'environment-rig',patch:{threeD:true}},
      {type:'set_layer',target:'env',patch:{parent:'environment-rig'}},
      {type:'set_property',target:'env',path:'rotation.y',value:0}
    ]);if(!result.ok)throw new Error(result.message);
    PM.selectLayers(['env']);PM.invalidate();
  });
  sameFrame(await render(),rotated,'parent rotation matches light rotation');
  for(const theme of ['light','dark']){
    await page.evaluate(theme=>(window as any).PM.theme.apply(theme),theme);
    await expect(page.getByRole('radiogroup',{name:'Cast shadows',exact:true})).toBeVisible();
    await page.locator('#panel-inspector').screenshot({path:info.outputPath(`shadow-controls-${theme}.png`)});
  }
  await page.locator('#stage').screenshot({path:info.outputPath('environment-shadow.png')});
  expect(session.diagnostics.console.filter(record=>/Shader Error|VALIDATE_STATUS|ERROR: 0:/.test(record.text))).toEqual([]);
});
