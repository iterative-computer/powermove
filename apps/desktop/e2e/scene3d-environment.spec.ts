import { expect, test } from './helpers/app';
import { hdrPanorama, exrPanorama } from '../src/renderer/src/core/scene3d/__fixtures__/environment';

test('Environment Light is a timeline light with its own source, intensity and XYZ rotation', async ({session},info) => {
  test.setTimeout(120_000);await session.openEditor();const page=session.page;
  const setup=await page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();PM.replaceProject(PM.mkProject({w:640,h:360,dur:3,fps:30}));
    const api=PM.Kernel.api('environment-e2e').scene3d;
    api.edit({operation:'add_camera',id:'camera',camera:{p:{x:0,y:0,z:3}}});
    api.edit({operation:'add_object',object:{id:'sphere',source:{primitive:'sphere'},material:{p:{color:'#FFFFFF',metalness:1,roughness:.1}}}});
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
  await importMap('Studio.hdr',hdrPanorama());
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
});
