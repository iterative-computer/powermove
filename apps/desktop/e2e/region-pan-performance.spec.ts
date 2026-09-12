import { readFileSync, writeFileSync } from 'node:fs';
import { test, expect } from './helpers/app';

test.skip(!process.env.PM_PERF_PROJECT, 'Set PM_PERF_PROJECT to a copied project');
for (const camera of [
  {name:'timeline', zoom:4.371588852276498, pan:[298.525348951279, -471.9744386396628]},
  {name:'media and shadow', zoom:3.1223372814366486, pan:[2142.9624206302624,1263.2672596867947]},
]) test(`pan the reported ${camera.name} region at fractional Retina zoom`, async ({ session }, testInfo) => {
  test.setTimeout(120000);
  const { page } = session;
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1448, height: 949, deviceScaleFactor: 2, mobile: false });
  await page.evaluate(data => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: data.proj || data }));
    PM.ProjectsScreen.hide(); PM.agentFrameCapture = true;
  }, JSON.parse(readFileSync(process.env.PM_PERF_PROJECT!, 'utf8')));
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl && (window as any).PM?.Viewer?.stage));
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const result = await page.evaluate(async camera => {
    const PM = (window as any).PM, V = PM.Viewer;
    // Match the actual user's panel and camera, independent of fixture docking.
    V.stage.style.width = '681px'; V.stage.style.height = '502px';
    V.fit = false; V.zoom = camera.zoom; V.pan = [...camera.pan];
    V.layout(); PM.GL.render(0, { mblur: true, mbSamples: 12, shutter: .5 });
    const resizes: any[] = []; const resize = PM.GL.resize;
    PM.GL.resize = (w: number,h: number,...args: any[]) => { const old = [V.el.width,V.el.height],start=performance.now(); const value=resize(w,h,...args); if(value)resizes.push({old,next:[w,h],ms:performance.now()-start}); return value; };
    const original = PM.GL.render; const costs: number[] = [], frames: number[] = [];
    let previous = performance.now();
    PM.GL.render = (...args: any[]) => { const start = performance.now(); const value = original(...args); costs.push(performance.now() - start); return value; };
    PM.agentFrameCapture = camera.freeze;
    try {
      for (let i = 0; i < 180; i++) {
        await new Promise(requestAnimationFrame);
        const now = performance.now(); frames.push(now - previous); previous = now;
        const rect = V.stage.getBoundingClientRect();
        const direction = Math.floor(i / 45) % 2 ? -1 : 1;
        V.stage.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaX: 7.3 * direction, deltaY: 2.7 * direction, clientX: rect.left + rect.width/2, clientY: rect.top + rect.height/2 }));

      }
      await new Promise(resolve => setTimeout(resolve, 150));
      return { frames, costs, resizes, stage: [V.stage.clientWidth,V.stage.clientHeight], pan: V.pan, zoom: V.zoom, draws: PM.GL.stats.draws };
    } finally { PM.GL.render = original; PM.GL.resize = resize; PM.agentFrameCapture = true; }
  }, {...camera, freeze:process.env.PM_PERF_FREEZE === '1'});
  const profile = await cdp.send('Profiler.stop');
  const pixels = await page.evaluate(() => {
    const PM=(window as any).PM,GL=PM.GL;
    const read=(enabled:boolean)=>{
      GL.render(0,{mblur:false,occlusionCulling:enabled});
      const data=new Uint8Array(GL.canvas.width*GL.canvas.height*4);
      GL.gl.readPixels(0,0,GL.canvas.width,GL.canvas.height,GL.gl.RGBA,GL.gl.UNSIGNED_BYTE,data);return data;
    };
    const reference=read(false),optimized=read(true);let different=0,max=0;
    for(let i=0;i<reference.length;i++){const d=Math.abs(reference[i]!-optimized[i]!);if(d)different++;max=Math.max(max,d);}
    return {different,max};
  });
  expect(pixels).toEqual({different:0,max:0});

  writeFileSync(testInfo.outputPath('region-pan.cpuprofile'), JSON.stringify(profile.profile));
  writeFileSync(testInfo.outputPath('region-pan.json'), JSON.stringify(result));
  const sorted = [...result.frames].sort((a,b)=>a-b);
  console.log('REGION_PAN', JSON.stringify({ median:sorted[90],p95:sorted[171],max:sorted[179],renders:result.costs.length,cpuMs:result.costs.reduce((a,b)=>a+b,0),stage:result.stage,draws:result.draws }));
  expect(result.resizes).toEqual([]);
  expect(result.costs.length).toBeLessThan(25);
  expect(result.zoom).toBe(camera.zoom);
  expect(result.pan[0]).toBeCloseTo(camera.pan[0]!, 5);
  expect(session.diagnostics.pageErrors).toEqual([]);
  await page.evaluate(async () => await (window as any).PM.flushProject());
  await session.app.evaluate(({app})=>app.exit(0)).catch(()=>undefined);
});
