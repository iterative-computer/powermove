/* Opt-in, paired actual viewer playback. No loop: every mask raster is new. */
import { appendFile, mkdir, statfs } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, launchApp, test } from './helpers/app';
import { compositorTiming } from './helpers/compositor-timing';

test('measure animated path masks at 1080p and UHD against today', async () => {
  test.skip(!process.env.PM_FIXUPS2_MEASURE, 'opt-in paired viewer measurement');
  test.setTimeout(240_000);
  const evidence = process.env.PM_FIXUPS2_EVIDENCE ?? '/tmp/ne-fixups2-evidence';
  await mkdir(evidence, { recursive: true });
  const runTag = process.env.PM_FIXUPS2_RUN ?? 'r3';
  if (!/^[a-z0-9-]+$/i.test(runTag)) throw new Error('Invalid measurement run tag');
  const runs = process.env.PM_FIXUPS2_HD_ONLY
    ? [{width:1920,enabled:true,hidden:true}]
    : [1920,3840].flatMap(width => [false,true].map(enabled=>({width,enabled,hidden:false}))).concat(process.env.PM_FIXUPS2_HIDDEN_DIAGNOSTIC ? [{width:1920,enabled:true,hidden:true}] : []);
  for (const {width,enabled,hidden} of runs) {
    const disk = await statfs('/System/Volumes/Data');
    const freeGiB = disk.bavail * disk.bsize / 1024**3;
    if (freeGiB < 40) throw new Error(`Disk below mandatory 40 GiB floor before app launch: ${freeGiB.toFixed(2)} GiB`);
    console.log(`Disk before ${width}/${enabled?'engine':'today'}: ${freeGiB.toFixed(2)}GiB`);
    const app = await launchApp({ env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
    try {
      const displayBeforeShow = await app.page.evaluate(() => ({at:performance.now(),wallMs:Date.now(),visibility:document.visibilityState,dpr:devicePixelRatio,edr:matchMedia('(dynamic-range: high)').matches,wide:matchMedia('(color-gamut: p3)').matches}));
      // S1 trace mode: composite an unfocused window entirely off every display.
      const firstShow = !hidden ? await app.app.evaluate(({ BrowserWindow, screen }, harnessPath) => {
        const win = BrowserWindow.getAllWindows().find(w=>!w.isDestroyed() && w.webContents.getURL().startsWith('app:'))!;
        process.getBuiltinModule('module').createRequire(harnessPath)(harnessPath).allowOffscreenWindow(win.getNativeWindowHandle());
        const right = Math.max(...screen.getAllDisplays().map(d=>d.bounds.x+d.bounds.width));
        win.setBounds({x:right+4000,y:-6000,width:1200,height:800});
        win.webContents.setAudioMuted(true);
        const visible = () => { const b = win.getBounds(); return screen.getAllDisplays().some(d=>b.x<d.bounds.x+d.bounds.width && b.x+b.width>d.bounds.x && b.y<d.bounds.y+d.bounds.height && b.y+b.height>d.bounds.y); };
        if (visible()) throw new Error('Measurement window would be visible');
        const wallMs = Date.now();
        win.showInactive();
        if (visible()) { win.hide(); throw new Error('Measurement window moved onto a display'); }
        return {wallMs,bounds:win.getBounds()};
      }, process.env.POWERMOVE_PM_BRIDGE_HARNESS ?? path.join(os.homedir(),'Library/Caches/powermove-engine/pm_bridge_harness.node')) : null;
      const displayAfterShow = await app.page.evaluate(() => ({at:performance.now(),wallMs:Date.now(),visibility:document.visibilityState,dpr:devicePixelRatio,edr:matchMedia('(dynamic-range: high)').matches,wide:matchMedia('(color-gamut: p3)').matches}));
      if (enabled) await app.page.evaluate(() => (window as any).PM.EngineConnection.setTrace(true));
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
      await app.app.evaluate(({}, harnessPath) => {
        const harness = process.getBuiltinModule('module').createRequire(harnessPath)(harnessPath);
        const state = {changes:[] as any[],last:'',timer:null as any};
        const read = () => { const display = harness.displayState(); const key=JSON.stringify(display); if(key!==state.last){state.last=key;state.changes.push({wallMs:Date.now(),...display});} };
        read(); state.timer=setInterval(read,100); (globalThis as any).__maskDisplayState=state;
      }, process.env.POWERMOVE_PM_BRIDGE_HARNESS ?? path.join(os.homedir(),'Library/Caches/powermove-engine/pm_bridge_harness.node'));
      const tracePath = `${evidence}/mask-${runTag}-${width}-${enabled ? 'engine' : 'today'}${hidden?'-hidden':''}.trace.json`;
      await app.app.evaluate(async ({ contentTracing }) => {
        await contentTracing.startRecording({ included_categories: ['disabled-by-default-devtools.timeline.frame','blink.user_timing','graphics.pipeline','viz'], excluded_categories: ['*'] });
      });
      const result = await app.page.evaluate(async ({ enabled, width }) => {
        const PM = (window as any).PM;
        const viewer = PM.Kernel.services.get('viewer');
        const presentation = viewer.enginePresentation;
        // Full preview resolution draws the entire composition at its own
        // size; retain the actual view/descriptor to verify that assumption.
        const frames: { t:number; at:number; raf:number|null }[] = [];
        let inRaf: number|null = null;
        const nativeRaf = window.requestAnimationFrame.bind(window);
        if (!enabled) window.requestAnimationFrame = callback => nativeRaf(t=>{
          inRaf = t; try {callback(t);} finally {inRaf = null;}
        });
        let measuring = false;
        const generations = new Set<number>();
        const transitions: any[] = [];
        const displayChanges: any[] = [];
        let lastDisplay = '';
        const readDisplay = () => {
          const state = {visibility:document.visibilityState,dpr:devicePixelRatio,edr:matchMedia('(dynamic-range: high)').matches,wide:matchMedia('(color-gamut: p3)').matches};
          const key = JSON.stringify(state);
          if (key !== lastDisplay) { lastDisplay = key; displayChanges.push({at:performance.now(),...state}); }
        };
        readDisplay();
        const displayTimer = setInterval(readDisplay,100);
        if (presentation) {
          const update = presentation.update.bind(presentation);
          presentation.update = (...args:any[]) => {
            const old = presentation.state; update(...args);
            if (presentation.state !== old) transitions.push({at:performance.now(),old,next:presentation.state,stack:new Error().stack});
          };
        }
        const off = enabled ? presentation.onPresented((f:any) => {
          if (measuring) generations.add(presentation.stats().generation);
          if (f.timed) frames.push({t:f.pts/705600000,at:f.displayUs/1000,raf:f.rafMs});
        }) : PM.bus.on('preview:presented', (f:any) => { if (PM.playing) frames.push({t:f.time,at:performance.now(),raf:inRaf}); });
        performance.mark('pm-mask-origin');
        PM.setTime(0); PM.play();
        await new Promise(r=>setTimeout(r,2000));
        frames.length = 0;
        const started = performance.now();
        const before = enabled ? presentation.stats() : null;
        const legacyContext = PM.GL?.gl;
        const legacyBefore = !enabled ? {colorSpace:legacyContext?.drawingBufferColorSpace,attributes:legacyContext?.getContextAttributes(),width:legacyContext?.drawingBufferWidth,height:legacyContext?.drawingBufferHeight} : null;
        const beforeState = presentation?.state;
        const resizeAtStart = presentation ? {pending:presentation.pendingSize,applied:presentation.appliedSize,timer:presentation.settleTimer} : null;
        measuring = true;
        await new Promise(r=>setTimeout(r,12000));
        const elapsed = performance.now()-started;
        off();
        clearInterval(displayTimer);
        if (!enabled) window.requestAnimationFrame = nativeRaf;
        const after = enabled ? presentation.stats() : null;
        PM.pause();
        const trace = enabled ? await presentation.takeTrace() : null;
        const submits = enabled ? (trace?.draws ?? []).filter((d:any)=>d[7]===1 && d[3]/705600000 >= frames[0]?.t && d[3]/705600000 <= frames.at(-1)!.t).map((d:any)=>d[5]) : frames.map(f=>f.at);
        const intervals = frames.slice(1).map((f,i)=>f.at-frames[i].at).sort((a,b)=>a-b);
        const q = (p:number) => intervals[Math.min(intervals.length-1,Math.floor(intervals.length*p))] ?? null;
        let skips=0,repeats=0;
        for(let i=1;i<frames.length;i++){const delta=Math.round((frames[i].t-frames[i-1].t)*30);if(delta>1)skips+=delta-1;else if(delta===0)repeats++;}
        return { legacyBefore,legacyAfter:!enabled?{sameContext:legacyContext===PM.GL?.gl,colorSpace:PM.GL?.gl?.drawingBufferColorSpace,attributes:PM.GL?.gl?.getContextAttributes(),width:PM.GL?.gl?.drawingBufferWidth,height:PM.GL?.gl?.drawingBufferHeight}:null,rafs:frames.map(f=>f.raf),displayChanges,resizeAtStart,generations:[...generations],submits,transitions,beforeState,enabled,width,height:width*9/16,elapsed,shown:frames.length,skips,repeats,intervalMs:{p50:q(.5),p95:q(.95),p99:q(.99),max:q(1)},before,after,view:presentation?.state??null,descriptor:presentation?.latest?{size:presentation.latest.size}:null,glSize:{width:PM.GL.gl.canvas.width,height:PM.GL.gl.canvas.height},rasterSize:enabled&&presentation.state?{width:PM.proj.w*presentation.state.zoom*presentation.state.devicePixelRatio,height:PM.proj.h*presentation.state.zoom*presentation.state.devicePixelRatio}:null,failures:PM.EngineDoc?.failures()??[] };
      }, { enabled, width });
      await app.app.evaluate(async ({ contentTracing }, tracePath) => { await contentTracing.stopRecording(tracePath); }, tracePath);
      const nativeDisplay = await app.app.evaluate(() => {const state=(globalThis as any).__maskDisplayState;clearInterval(state.timer);return state.changes;});
      const common: any = hidden ? null : await compositorTiming(tracePath, result.submits).catch(error=>({error:String(error)}));
      const capabilityLogs = app.diagnostics.console.filter(record => record.text.includes('display capabilities changed'));
      const report = enabled ? await app.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive')) : null;
      await appendFile(`${evidence}/path-mask-playback-${runTag}.jsonl`, JSON.stringify({...result,capabilityLogs,freeGiB,nativeDisplay,hidden,firstShow,displayBeforeShow,displayAfterShow,common,report})+'\n');
      console.log(JSON.stringify({...result,capabilityLogs,freeGiB,nativeDisplay,hidden,firstShow,displayBeforeShow,displayAfterShow,common,report}));
      expect(result.shown).toBeGreaterThan(0);
      if (common) {
        expect.soft(common.error).toBeUndefined();
        expect.soft(common.missing).toEqual({received:0,drawn:0,superseded:0});
        expect.soft(common.duplicates).toBe(0);
      }
      expect(result.failures).toEqual([]);
      if (enabled) {
        expect.soft(result.resizeAtStart.pending).toBeNull();
        expect.soft(result.generations).toEqual([result.before.generation]);
        expect.soft(result.after.generation).toBe(result.before.generation);
        expect.soft(result.after.viewSeq).toBe(result.before.viewSeq);
        expect.soft(result.after.sessions).toBe(result.before.sessions);
        expect((report as any)?.presenter?.renderErrors ?? 0).toBe(0);
        expect((report as any)?.presenter?.layerFailures ?? 0).toBe(0);
      }
      if (enabled) {
        expect(result.view.size).toEqual({width,height:width*9/16});
        expect(result.rasterSize).toEqual({width,height:width*9/16});
      }
      else expect(result.glSize).toEqual({width,height:width*9/16});
    } finally { await app.close(); }
  }
});
