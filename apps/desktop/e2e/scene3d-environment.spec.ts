import { expect, test } from './helpers/app';
import { hdrPanorama, exrPanorama } from '../src/renderer/src/core/scene3d/__fixtures__/environment';

test('HDR and EXR environment lighting imports, renders, rotates, undoes and reopens', async ({session},info) => {
  test.setTimeout(120_000); await session.openEditor(); const page=session.page;
  const camera=await page.evaluate(async bytes=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();PM.replaceProject(PM.mkProject({w:640,h:360,dur:3,fps:30}));
    const api=PM.Kernel.api('environment-e2e').scene3d;
    api.edit({operation:'add_camera',id:'camera',camera:{p:{x:0,y:0,z:3}}});
    api.edit({operation:'add_object',object:{id:'sphere',source:{primitive:'sphere'},material:{p:{color:'#FFFFFF',metalness:1,roughness:.1}}}});
    api.edit({operation:'add_light',light:{id:'dark',type:'sun',p:{intensity:0}}});
    api.edit({operation:'set_environment',target:'camera',patch:{p:{ambient:0}}});
    // Camera data written by a version before environment maps existed.
    delete PM.L('camera').d.data.environment.map;delete PM.L('camera').d.data.environment.p.intensity;delete PM.L('camera').d.data.environment.p.rotation;
    await PM.cmd('3d.environment',new File([new Uint8Array(bytes)],'Studio.hdr'));
    PM.selectLayers(['camera']);return 'camera';
  },[...hdrPanorama()]);
  const map=page.getByRole('combobox',{name:'Map',exact:true});
  await expect(map).toBeVisible();await expect(map).toContainText('Studio.hdr');
  const render=()=>page.evaluate(()=>{const PM=(window as any).PM;const pixels=PM.GL.renderToPixels(0,160,90,{transparent:true,mblur:false});return {rgb:[...pixels].reduce((sum:number,v:number,i:number)=>sum+(i%4===3?0:v),0),pixels:[...pixels]};});
  const lit=await render();expect(lit.rgb).toBeGreaterThan(10000);
  const strength=page.locator('[data-scene-channel="environment.intensity"] input');
  await strength.click();await strength.fill('0');await strength.press('Enter');const dark=await render();expect(dark.rgb).toBeLessThan(lit.rgb*.1);
  await page.keyboard.press('Meta+z');await expect(strength).toHaveValue('1');
  const rotation=page.locator('[data-scene-channel="environment.rotation"] input');
  await rotation.click();await rotation.fill('90');await rotation.press('Enter');expect((await render()).pixels).not.toEqual(lit.pixels);
  // SelectField's Import action opens the system file input and keeps the old
  // map until the new file has decoded successfully.
  const chooserPromise=page.waitForEvent('filechooser');
  await map.click();await page.getByRole('option',{name:'Import HDR / EXR…',exact:true}).click();
  await (await chooserPromise).setFiles({name:'Courtyard.exr',mimeType:'image/x-exr',buffer:Buffer.from(exrPanorama())});
  await expect(map).toContainText('Courtyard.exr');
  expect((await render()).rgb).toBeGreaterThan(10000);
  for(const theme of ['light','dark']) {
    await page.evaluate(theme=>(window as any).PM.theme.apply(theme),theme);
    await map.scrollIntoViewIfNeeded();
    await page.locator('#panel-inspector').screenshot({path:info.outputPath(`environment-${theme}.png`)});
  }
  const filePath=session.userData+'/environment.pmv';
  await session.app.evaluate(({dialog},filePath)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath});dialog.showOpenDialog=async()=>({canceled:false,filePaths:[filePath]});},filePath);
  const beforeSave=await render();
  await page.evaluate(async()=>{const PM=(window as any).PM;if(!await PM.saveProject())throw new Error('Save failed');await PM.openProject();});
  await page.waitForFunction(()=>{const PM=(window as any).PM;return !!PM.assets.get(PM.L('camera')?.d?.data?.environment?.map)?.environmentTexture;});
  await page.evaluate(id=>(window as any).PM.selectLayers([id]),camera);
  await expect(map).toContainText('Courtyard.exr');await expect(rotation).toHaveValue('90°');
  expect((await render()).pixels).toEqual(beforeSave.pixels);
  await map.click();await page.getByRole('option',{name:'None',exact:true}).click();expect((await render()).rgb).toBeLessThan(beforeSave.rgb*.1);
  await page.keyboard.press('Meta+z');await expect(map).toContainText('Courtyard.exr');
});
