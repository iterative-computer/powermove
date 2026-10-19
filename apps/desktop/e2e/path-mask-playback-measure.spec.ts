/* Opt-in, paired actual viewer playback. No loop: every mask raster is new. */
import { appendFile, mkdir } from 'node:fs/promises';
import { expect, launchApp, test } from './helpers/app';

test('measure animated path masks at 1080p and UHD against today', async () => {
  test.skip(!process.env.PM_FIXUPS2_MEASURE, 'opt-in paired viewer measurement');
  test.setTimeout(240_000);
  const evidence = process.env.PM_FIXUPS2_EVIDENCE ?? '/tmp/ne-fixups2-evidence';
  await mkdir(evidence, { recursive: true });
  for (const width of [1920, 3840]) for (const enabled of [false, true]) {
    const app = await launchApp({ env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
    try {
      await app.openEditor();
      await app.page.evaluate(({ width }) => {
        const PM = (window as any).PM;
        const h = width * 9/16, s = width/1920;
        const project = PM.mkProject({ name: 'Animated path mask measurement', w: width, h, fps: 30, dur: 120 });
        PM.replaceProject(project, {});
        const layer = PM.mkLayer('solid', { name: 'Masked solid' }, PM.proj);
        layer.d.w = width; layer.d.h = h; layer.d.color = '#4C8DFF';
        for (const key of ['position.x','position.y','anchor.x','anchor.y']) layer.p[key].v = 0;
        const mask = PM.mkMask('rect', PM.proj);
        mask.p.x.v = width/2; mask.p.y.v = h/2; mask.p.feather.v = 24*s;
        const values = { x:0,y:0,rotation:0,scaleX:100,scaleY:100,closed:true,trimStart:0,trimEnd:100 };
        const path = { id: PM.uid('path'), name: 'Animated blob', parent:null, p:Object.fromEntries(Object.entries(values).map(([k,v])=>[k,PM.P(v)])), vertices: [] as any[] };
        const coords = [[-250,-120,0,0,120,-160],[120,-200,-80,0,160,60],[280,60,0,-120,-40,180],[-60,220,160,0,-200,0]];
        for (const v of coords) path.vertices.push({ id:PM.uid('v'), p:Object.fromEntries(['x','y','inX','inY','outX','outY'].map((k,i)=>[k,PM.P(v[i]*s)])) });
        path.p.rotation = PM.P(0, {kf:[PM.KF(0,0),PM.KF(120,240)]});
        path.vertices[1].p.y = PM.P(-200*s,{kf:[PM.KF(0,-200*s),PM.KF(120,0)]});
        mask.path = path;
        layer.masks = [mask];
        PM.proj.layers = [layer];
        PM.proj.work = [0,120];
        PM.touch(); PM.bus.emit('project'); PM.bus.emit('layers');
        PM.previewResolution = '1'; PM.quality = 1; PM.perf.auto = false;
        PM.loop = false;
        const viewer = PM.Kernel.services.get('viewer');
        // Size the hidden viewer itself to the requested device canvas. Both
        // paths draw the same full composition, rather than fitting UHD down
        // to the normal test window's much smaller stage.
        const dpr = window.devicePixelRatio || 1;
        for (const [k,v] of Object.entries({width:width/dpr+'px',height:h/dpr+'px','min-width':width/dpr+'px','min-height':h/dpr+'px','max-width':'none','max-height':'none'})) viewer.stage.style.setProperty(k,v,'important');
        viewer.fit = false; viewer.zoom = 1/dpr; viewer.pan = [0,0];
        viewer.layout();
      }, { width });
      if (enabled) await app.page.waitForFunction(() => (window as any).PM?.EngineConnection?.status().route === 'engine' && (window as any).PM.Kernel.services.get('viewer').enginePresentation?.presenting, undefined, { timeout: 60_000 });
      const result = await app.page.evaluate(async ({ enabled, width }) => {
        const PM = (window as any).PM;
        const viewer = PM.Kernel.services.get('viewer');
        const presentation = viewer.enginePresentation;
        // Full preview resolution draws the entire composition at its own
        // size; retain the actual view/descriptor to verify that assumption.
        const frames: { t:number; at:number }[] = [];
        const off = enabled ? presentation.onPresented((f:any) => {
          if (f.timed) frames.push({t:f.pts/705600000,at:f.displayUs/1000});
        }) : PM.Kernel.events.on('frame:rendered', (f:any) => { if (PM.playing) frames.push({t:f.time,at:performance.now()}); });
        PM.setTime(0); PM.play();
        await new Promise(r=>setTimeout(r,2000));
        frames.length = 0;
        const started = performance.now();
        const before = enabled ? presentation.stats() : null;
        await new Promise(r=>setTimeout(r,12000));
        const elapsed = performance.now()-started;
        off();
        const after = enabled ? presentation.stats() : null;
        PM.pause();
        const intervals = frames.slice(1).map((f,i)=>f.at-frames[i].at).sort((a,b)=>a-b);
        const q = (p:number) => intervals[Math.min(intervals.length-1,Math.floor(intervals.length*p))] ?? null;
        let skips=0,repeats=0;
        for(let i=1;i<frames.length;i++){const delta=Math.round((frames[i].t-frames[i-1].t)*30);if(delta>1)skips+=delta-1;else if(delta===0)repeats++;}
        return { enabled,width,height:width*9/16,elapsed,shown:frames.length,skips,repeats,intervalMs:{p50:q(.5),p95:q(.95),p99:q(.99),max:q(1)},before,after,view:presentation?.state??null,descriptor:presentation?.latest?{size:presentation.latest.size}:null,glSize:{width:PM.GL.gl.canvas.width,height:PM.GL.gl.canvas.height},rasterSize:enabled&&presentation.state?{width:PM.proj.w*presentation.state.zoom*presentation.state.devicePixelRatio,height:PM.proj.h*presentation.state.zoom*presentation.state.devicePixelRatio}:null,failures:PM.EngineDoc?.failures()??[] };
      }, { enabled, width });
      const report = enabled ? await app.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive')) : null;
      await appendFile(evidence+'/path-mask-playback.jsonl', JSON.stringify({...result,report})+'\n');
      console.log(JSON.stringify({...result,report}));
      expect(result.shown).toBeGreaterThan(0);
      expect(result.failures).toEqual([]);
      if (enabled) {
        expect((report as any)?.presenter?.renderErrors ?? 0).toBe(0);
        expect((report as any)?.presenter?.layerFailures ?? 0).toBe(0);
      }
      if (enabled) expect(result.rasterSize).toEqual({width,height:width*9/16});
      else expect(result.glSize).toEqual({width,height:width*9/16});
    } finally { await app.close(); }
  }
});
