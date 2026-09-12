import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';
import {copyFile,unlink} from 'node:fs/promises';
import path from 'node:path';
import {fixturePath} from './helpers/media';

test.describe('@import-mp4 H.264 import smoke', () => {
  test.beforeEach(async ({session}) => {
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', {detail: PM.mkProject({name:'Regression fixture'})}));
      PM.ProjectsScreen.hide();
    });
  });

  test('keeps imported video playable after the source is removed and the app closes',async({session}) => {
    const source=path.join(session.userData,'temporary-import.mp4');
    await copyFile(fixturePath('h264-aac.mp4'),source);
    const chooser=session.page.waitForEvent('filechooser');
    await session.page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(source);
    await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((l:any) => l.name==='temporary-import.mp4'));
    const id=await session.page.evaluate(() => (window as any).PM.proj.id);
    // A successful import promises an app-owned copy, not a session-only URL or
    // a dependency on the original in Downloads. No test-only store flush.
    await unlink(source);
    await session.relaunch();
    await session.page.waitForFunction((id) => (window as any).PM.proj.id===id,id);
    await session.page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a:any) => a.name==='temporary-import.mp4'));
    const restored=await session.page.evaluate(async() => {
      const PM=(window as any).PM,asset=[...PM.assets.map.values()].find((a:any)=>a.name==='temporary-import.mp4') as any;
      const blob=await PM.MediaStore.get(PM.proj.assets[asset.id]);
      const layer=PM.proj.layers.find((l:any)=>l.d.asset===asset.id);
      PM.Edit.apply({type:'set_layer',target:layer.id,patch:{from:0}});
      PM.setTime(1.5,{raw:true,force:true});
      await new Promise<void>((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Restored video did not seek')),8000);
        const poll=()=>{if(Math.abs(asset.el.currentTime-1.5)<.05 && asset.el.readyState>=2){clearTimeout(timer);resolve()}else setTimeout(poll,50)};poll()});
      const frame=PM.renderFrameTo(1.5,PM.proj.w,PM.proj.h),ctx=frame.getContext('2d');
      const pixel=[...ctx.getImageData(frame.width/2,frame.height/2,1,1).data];
      return {
        bytes: blob?.size,
        live: asset.el.readyState,
        pixel,
        hasAudio: asset.hasAudio,
        audioBytes: asset.audioBlob?.size,
        embeddedAudio: layer.d.embeddedAudio,
      };
    });
    expect(restored.bytes).toBeGreaterThan(0);expect(restored.live).toBeGreaterThanOrEqual(2);
    expect(restored.pixel[1]).toBeGreaterThan(225);expect(restored.pixel[0]).toBeLessThan(30);
    expect(restored).toMatchObject({ hasAudio: true, embeddedAudio: true });
    expect(restored.audioBytes).toBeGreaterThan(0);
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.setTime(.25, { raw: true, force: true });
      PM.play();
    });
    await session.page.waitForFunction(() => Number(document.documentElement.dataset.audioVoices || 0) > 0);
    await session.page.evaluate(() => (window as any).PM.pause());
  });
  test('imports H.264/AAC media and renders its green frame', async ({ session }) => {
    const { page } = session;
    await importFixture(page, 'h264-aac.mp4');

    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      return [...PM.assets.map.values()].some((asset: any) => asset.name === 'h264-aac.mp4')
        && PM.proj.layers.some((layer: any) => layer.type === 'video' && layer.name === 'h264-aac.mp4');
    });

    // Import places the clip at the playhead; pin it to t=0 through the typed
    // edit boundary so the fixture's 1.5s frame (green) lands at T=1.5. The
    // engine owns the <video> element's currentTime — never seek it directly.
    const applied = await page.evaluate(() => {
      const PM = (window as any).PM;
      return PM.Edit.apply(
        [{ type: 'set_layer', target: 'h264-aac.mp4', patch: { from: 0 } }],
        { label: 'e2e: pin clip to 0' }
      );
    });
    expect(applied.ok).toBe(true);

    // The compositor scrubs the element to the layer-local time and uploads the
    // frame once the decoder delivers it — poll the rendered pixel.
    const pixel = await page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.setTime(1.5, { raw: true, force: true });
      const sample = (): number[] => {
        const canvas = PM.renderFrameTo(1.5, PM.proj.w, PM.proj.h) as HTMLCanvasElement;
        const context = canvas.getContext('2d')!;
        return [...context.getImageData(Math.floor(canvas.width / 2), Math.floor(canvas.height / 2), 1, 1).data];
      };
      const isGreen = (p: number[]): boolean => p[0]! <= 30 && p[1]! >= 225 && p[2]! <= 30;
      const deadline = Date.now() + 10_000;
      let last = sample();
      while (Date.now() < deadline) {
        if (isGreen(last) && isGreen((last = sample()))) return last; // stable across two renders
        await new Promise(resolve => setTimeout(resolve, 120));
        last = sample();
      }
      return last;
    });

    expect(pixel[0]).toBeGreaterThanOrEqual(0);
    expect(pixel[0]).toBeLessThanOrEqual(30);
    expect(pixel[1]).toBeGreaterThanOrEqual(225);
    expect(pixel[1]).toBeLessThanOrEqual(255);
    expect(pixel[2]).toBeGreaterThanOrEqual(0);
    expect(pixel[2]).toBeLessThanOrEqual(30);
  });

  test('plays an imported soundtrack and separates it from the timeline context menu', async ({ session }) => {
    const { page } = session;
    await importFixture(page, 'h264-aac.mp4');
    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers.find((item: any) => item.type === 'video' && item.name === 'h264-aac.mp4');
      const asset = layer && PM.assets.get(layer.d.asset);
      return layer?.d.embeddedAudio === true && asset?.hasAudio === true && asset?.audioBuffer;
    });

    await page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers.find((item: any) => item.type === 'video' && item.name === 'h264-aac.mp4');
      PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { from: 0 } }, { label: 'e2e: pin clip to 0' });
      PM.setTime(.25, { raw: true, force: true });
      PM.play();
    });
    await page.waitForFunction(() => Number(document.documentElement.dataset.audioVoices || 0) > 0);
    await page.evaluate(() => (window as any).PM.pause());

    const rowPosition = await page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers.find((item: any) => item.type === 'video' && item.name === 'h264-aac.mp4');
      const index = PM.TL.rows.findIndex((row: any) => row.kind === 'layer' && row.L.id === layer.id);
      return { x: Math.min(100, PM.TL.gut / 2), y: PM.TL.ruler + index * PM.TL.row - PM.TL.scrollY + PM.TL.row / 2 };
    });
    await page.locator('#tl-canvas').click({ button: 'right', position: rowPosition });
    const separate = page.getByRole('menuitem', { name: 'Separate audio' });
    await expect(separate).toBeVisible();
    await separate.click();

    const separated = await page.evaluate(() => {
      const PM = (window as any).PM;
      const video = PM.proj.layers.find((item: any) => item.type === 'video' && item.name === 'h264-aac.mp4');
      const audio = PM.proj.layers.find((item: any) => item.type === 'audio' && item.d.asset === video.d.asset);
      return {
        videoHasEmbeddedAudio: video.d.embeddedAudio,
        audio: audio && { name: audio.name, from: audio.from, dur: audio.dur, trim: audio.d.trim },
        adjacent: PM.proj.layers.indexOf(audio) + 1 === PM.proj.layers.indexOf(video),
      };
    });
    expect(separated).toEqual({
      videoHasEmbeddedAudio: false,
      audio: { name: 'h264-aac.mp4 Audio', from: 0, dur: 2, trim: 0 },
      adjacent: true,
    });

    await page.evaluate(() => (window as any).PM.cmd('undo'));
    await expect.poll(() => page.evaluate(() => {
      const PM = (window as any).PM;
      const video = PM.proj.layers.find((item: any) => item.type === 'video' && item.name === 'h264-aac.mp4');
      return video.d.embeddedAudio === true
        && !PM.proj.layers.some((item: any) => item.type === 'audio' && item.d.asset === video.d.asset);
    })).toBe(true);
  });
});
