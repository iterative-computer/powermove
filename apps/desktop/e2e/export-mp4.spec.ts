import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';
import path from 'node:path';
import {readFile} from 'node:fs/promises';

test.describe('@export-mp4 H.264 delivery', () => {
  test('exports a playable MP4 with H.264 video and AAC audio', async ({ session }) => {
    test.setTimeout(45_000);
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.pause();
      PM.assets.clear();
      PM.proj = PM.mkProject({ name: 'E2E MP4', w: 64, h: 64, fps: 30, dur: 1, bg: '#000000' });
      PM.proj.work = [0, 1];
      PM.time = 0;
      PM.sel.layers = [];
      PM.Edit.apply([{
        type: 'add_layer',
        layerType: 'solid',
        name: 'Red',
        from: 0,
        duration: 1,
        content: { color: '#FF0000', w: 64, h: 64 },
      }], { label: 'E2E red solid', origin: 'e2e' });
      PM.bus.emit('project');
      PM.bus.emit('layers');
    });
    await importFixture(page, 'tone.wav');
    await page.waitForFunction(() => (window as any).PM.proj.layers.some(
      (layer: any) => layer.type === 'audio' && layer.name === 'tone.wav',
    ));

    const output=path.join(session.userData,'E2E MP4.mp4');
    await session.app.evaluate(({dialog},file)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath:file});},output);
    const result=await page.evaluate(async()=>await (window as any).PM.Export.run({format:'mp4',scale:1,fps:30,range:'all',quality:'draft',mblur:false,alpha:false,audio:true}));
    expect(result).toEqual({cancelled:false});
    const data=[...await readFile(output)];
    const exported=await page.evaluate(async(bytes)=>{
      const video=document.createElement('video'),url=URL.createObjectURL(new Blob([Uint8Array.from(bytes)],{type:'video/mp4'}));video.muted=true;video.src=url;
      try{await new Promise<void>((resolve,reject)=>{video.onloadeddata=()=>resolve();video.onerror=()=>reject(Error('Exported video did not decode'));});
        let presentedFrames=0;video.requestVideoFrameCallback?.(()=>presentedFrames++);await video.play();await new Promise<void>(resolve=>{video.onended=()=>resolve();});
        return {bytes,name:'E2E MP4.mp4',type:'video/mp4',width:video.videoWidth,height:video.videoHeight,duration:video.duration,presentedFrames};
      }finally{URL.revokeObjectURL(url);}
    },data);

    const bytes = Uint8Array.from(exported.bytes);
    const signatures = new TextDecoder('latin1').decode(bytes);
    expect(exported.name).toBe('E2E MP4.mp4');
    expect(exported.type).toBe('video/mp4');
    expect(String.fromCharCode(...bytes.subarray(4, 8))).toBe('ftyp');
    expect(signatures).toContain('avc1');
    expect(signatures).toContain('mp4a');
    expect(exported).toMatchObject({ width: 64, height: 64 });
    expect(exported.duration).toBeGreaterThan(0.7);
    expect(exported.duration).toBeLessThan(1.5);
    expect(exported.presentedFrames).toBeGreaterThan(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
