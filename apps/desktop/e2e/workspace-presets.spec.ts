import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

async function saveRender(name: string, dataUrl: string) {
  const dir = process.env.POWERMOVE_WORKSPACE_PRESET_SHOTS;
  if (!dir) return;
  await mkdir(dir, {recursive: true});
  await writeFile(path.join(dir, name + '.png'), Buffer.from(dataUrl.split(',')[1]!, 'base64'));
}

test('last-used layouts and an explicit default survive new projects and relaunch', async ({session}) => {
  await session.openEditor();
  const expected = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.WS.mutate((w: any) => { w.layout.docks.find((d: any) => d.id === 'left').size = 377; });
    return {id: PM.WS.current.id, layout: PM.WS.snapshot().layout};
  });
  await session.relaunch(); await session.openEditor();
  const restarted = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', {detail: PM.mkProject({name:'Fresh project'})}));
    return {id: PM.WS.current.id, layout: PM.WS.snapshot().layout};
  });
  expect(restarted).toEqual(expected);
  const defaultId = await session.page.evaluate(() => {
    const PM = (window as any).PM, w = PM.WS.saveCurrent('My editing layout');
    PM.WS.setDefault(w.id); PM.WS.activate('animate', true);
    window.dispatchEvent(new CustomEvent('pm-open-project', {detail: PM.mkProject({name:'Default project'})}));
    if (PM.WS.current.id !== w.id) throw new Error('New project lost the chosen default');
    return w.id;
  });
  await session.relaunch(); await session.openEditor();
  expect(await session.page.evaluate(() => (window as any).PM.WS.defaultId())).toBe(defaultId);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('saved animated effect stacks survive relaunch, reproduce the render, and undo as one step', async ({session}) => {
  await session.openEditor();
  const saved = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.proj.w = 640; PM.proj.h = 360; PM.proj.bg = '#101010';
    const layer = PM.mkLayer('shape', {name:'Preset reference', p:{'position.x':320,'position.y':180},
      d:{shape:'rect',w:240,h:140,color:'#f4dfb0',stroke:0}});
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.time = 0;
    const fx = PM.mkEffect('blur', undefined, layer);
    fx.p.amount.kf = [PM.KF(0,8,'glide'),PM.KF(1,40,'glide')]; layer.fx.push(fx); PM.touch();
    const preset = PM.Presets.save({name:'Soft animated look',layerId:layer.id,effectIds:[fx.id],includeAnimation:true});
    const image = PM.renderFrameTo(1,640,360,{mblur:false}).toDataURL('image/png');
    return {presetId:preset.id,layerId:layer.id,image};
  });
  await saveRender('02-source-saved-look', saved.image);
  await session.relaunch(); await session.openEditor();
  const result = await session.page.evaluate(({presetId,layerId}) => {
    const PM = (window as any).PM, layer = PM.L(layerId);
    if (!layer || !PM.Presets.get(presetId)) throw new Error('Preset or project did not survive relaunch');
    PM.Edit.apply(layer.fx.map((fx: any) => ({type:'remove_effect',target:layerId,effect:fx.id})));
    const without = PM.renderFrameTo(1,640,360,{mblur:false}).toDataURL('image/png');
    const applied = PM.Presets.apply(presetId,layerId,0); if(!applied.ok) throw new Error(applied.message);
    const after = PM.renderFrameTo(1,640,360,{mblur:false}).toDataURL('image/png');
    PM.hist.undo(); const undoCount = PM.L(layerId).fx.length;
    PM.hist.redo(); const redoCount = PM.L(layerId).fx.length;
    return {without,after,undoCount,redoCount};
  }, saved);
  await saveRender('03-after-remove-effects', result.without);
  await saveRender('04-after-apply-preset', result.after);
  expect(result.after).toBe(saved.image); expect(result.without).not.toBe(saved.image);
  expect(result.undoCount).toBe(0); expect(result.redoCount).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('new windows inherit the last layout and receive library changes from other windows', async ({session}) => {
  await session.openEditor();
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.WS.mutate((w: any) => { w.layout.docks.find((d: any) => d.id === 'left').size = 377; });
    await PM.store.flush();
  });
  const opened = session.app.waitForEvent('window');
  await session.app.evaluate(({Menu}) => {
    const item = Menu.getApplicationMenu()!.getMenuItemById('newWindow')!;
    item.click(item, undefined, {} as any);
  });
  const second = await opened;
  await second.waitForFunction(() => Boolean((window as any).PM?.app));
  const inherited = await second.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.projectReady; await PM.Kernel.loader.builtinsReady;
    window.dispatchEvent(new CustomEvent('pm-open-project', {detail: PM.mkProject({name:'Second window'})}));
    return PM.WS.current.layout.docks.find((d: any) => d.id === 'left').size;
  });
  expect(inherited).toBe(377);
  const workspaceId = await session.page.evaluate(() => (window as any).PM.WS.saveCurrent('Shared layout').id);
  await expect.poll(() => second.evaluate(id => (window as any).PM.WS.get(id)?.name, workspaceId)).toBe('Shared layout');
  await session.page.evaluate(id => (window as any).PM.WS.rename(id, 'Renamed layout'), workspaceId);
  await expect.poll(() => second.evaluate(id => (window as any).PM.WS.get(id)?.name, workspaceId)).toBe('Renamed layout');
});
