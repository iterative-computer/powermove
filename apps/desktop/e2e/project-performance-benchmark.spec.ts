import { readFileSync, writeFileSync } from 'node:fs';
import { test, expect } from './helpers/app';
test.beforeEach(async ({ session }) => { await session.openEditor(); });
test.skip(!process.env.PM_PERF_PROJECT, 'Set PM_PERF_PROJECT to a local project JSON copy');
test('profile copied large project', async ({ session }, testInfo) => {
  test.setTimeout(120000);
  const page = session.page;
  await page.setViewportSize({width:1440,height:1000});
  const project = JSON.parse(readFileSync(process.env.PM_PERF_PROJECT!,'utf8'));
  await page.evaluate(project => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project',{detail:project.proj || project}));
    PM.ProjectsScreen.hide();
  }, project);
  await page.waitForTimeout(2000);
  const cdp=await page.context().newCDPSession(page);
  await cdp.send('Profiler.enable');
  await cdp.send('Profiler.start');
  await page.waitForTimeout(3000);
  const idle=await cdp.send('Profiler.stop');
  writeFileSync(testInfo.outputPath('idle.cpuprofile'),JSON.stringify(idle.profile));
  await cdp.send('Profiler.start');
  const result=await page.evaluate(async (width) => {
    const PM=(window as any).PM;
    PM.agentFrameCapture=true;
    const samples:number[]=[];
    const create=document.createElement.bind(document); let canvases=0;
    document.createElement=((tag:string,...rest:any[])=>{if(tag==='canvas')canvases++;return (create as any)(tag,...rest);}) as any;
    const uploads=PM.GL.gl.texImage2D.bind(PM.GL.gl);let textureUploads=0;
    PM.GL.gl.texImage2D=(...args:any[])=>{textureUploads++;return uploads(...args);};
    PM.GL.resize(width,width * 9 / 16);
    PM.GL.render(0);
    await new Promise(resolve=>setTimeout(resolve,100));
    canvases=0; textureUploads=0;
    for(let i=0;i<25;i++) {
      const start=performance.now();
      PM.GL.render(i/30,{mblur:true,mbSamples:6});
      samples.push(performance.now()-start);
      await new Promise(resolve=>setTimeout(resolve,50));
    }
    PM.GL.render(0);
    const canvas=PM.GL.canvas;
    return {samples, stats:PM.GL.stats, width:canvas.width,height:canvas.height,layers:PM.proj.layers.length,
      screenshot:canvas.toDataURL(), canvases,textureUploads,memory:PM.Memory.stats(),domNodes:document.querySelectorAll('*').length};
  }, Number(process.env.PM_PERF_WIDTH) || 1280);
  const profile=await cdp.send('Profiler.stop');
  writeFileSync(testInfo.outputPath('render.cpuprofile'),JSON.stringify(profile.profile));
  writeFileSync(testInfo.outputPath('canvas.png'),Buffer.from(result.screenshot.split(',')[1]!, 'base64'));
  const {screenshot,...metrics}=result;
  writeFileSync(testInfo.outputPath('metrics.json'),JSON.stringify(metrics));
  console.log('PERF',JSON.stringify(metrics));
  await page.screenshot({path:testInfo.outputPath('editor.png')});
  const interactions = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const samples = [];
    for (let i = 0; i < 12; i++) {
      const start = performance.now();
      PM.selectLayers([PM.proj.layers[i * 20].id]);
      PM.setTime(i / 30);
      PM.bus.emit('draw:timeline');
      PM.bus.emit('overlay');
      samples.push(performance.now() - start);
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    return samples;
  });
  writeFileSync(testInfo.outputPath('interactions.json'),JSON.stringify(interactions));
  console.log('INTERACTIONS',JSON.stringify(interactions));
  expect(session.diagnostics.pageErrors).toEqual([]);
  await page.evaluate(async()=>await (window as any).PM.flushProject());
  await session.app.evaluate(({app})=>app.exit(0)).catch(()=>undefined);
});
