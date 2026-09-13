import { test, expect } from './helpers/app';

test.beforeEach(async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Zoom raster checks', w: 800, h: 500, fps: 30, dur: 3 }) }));
    PM.ProjectsScreen.hide(); PM.agentFrameCapture = true;
  });
  await session.page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean((window as any).PM?.GL?.gl && viewer?.stage); });
});
test.afterEach(async ({ session }) => {
  await session.page.evaluate(async () => await (window as any).PM.flushProject());
  await session.app.evaluate(({ app }) => app.exit(0)).catch(() => undefined);
});

test('cropped primitive previews preserve reference pixels across pans, rotations, strokes and fills', async ({ session }) => {
  const results = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const results = [];
    const read = (sourceClipping: boolean) => {
      PM.GL.render(0, { mblur: false, sourceClipping });
      const gl = PM.GL.gl, pixels = new Uint8Array(320 * 240 * 4);
      gl.readPixels(0, 0, 320, 240, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      return pixels;
    };
    for (const shape of ['rect', 'ellipse', 'star', 'polygon', 'line']) for (const angle of [0, 17, 90]) {
      const layer = PM.mkLayer('shape', { d: { shape, w: 600.25, h: 400.15, radius: 42, stroke: 7, strokeColor: '#225599', color: '#bb6633', points: 5 }, p: { 'position.x': 450, 'position.y': 240, rotation: angle, opacity: 67, 'scale.x': angle === 90 ? -100 : 100 } });
      PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.touch();
      for (const x of [80, 100.25, 250, 650]) {
        PM.GL.resize(320, 240, { x, y: 65.75, width: 160, height: 120, compWidth: 800, compHeight: 500 });
        const reference = read(false), cropped = read(true);
        let different = 0, max = 0;
        cropped.forEach((value, index) => { const delta = Math.abs(value - reference[index]!); if (delta) different++; max = Math.max(max, delta); });
        results.push({ shape, angle, x, different, max });
      }
    }
    return results;
  });
  console.log('PIXEL_DIFFS', JSON.stringify(results.filter(result => result.different)));
  expect(results.every(result => result.max <= 1)).toBe(true);
  expect(results.every(result => result.different < 320 * 240 * 4 * .01)).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('large rectangle interiors avoid bitmaps and edits, Undo and export remain intact', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.mkLayer('shape', { d: { shape: 'rect', w: 3000, h: 2000, radius: 100, stroke: 10, color: '#225588' }, p: { 'position.x': 400, 'position.y': 250, opacity: 45 } });
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.touch();
    PM.GL.resize(320, 240, { x: 300, y: 190, width: 160, height: 120, compWidth: 800, compHeight: 500 });
    const image = (options = {}) => {
      PM.GL.resize(320, 240, { x: 300, y: 190, width: 160, height: 120, compWidth: 800, compHeight: 500 });
      PM.GL.render(0, { mblur: false, ...options }); return PM.GL.canvas.toDataURL();
    };
    const reference = image({ sourceClipping: false });
    PM.rasterClear();
    const optimized = image();
    const stats = PM.rasterStats();
    PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.color', value: '#55aa33', mode: 'static', preserveHandEdits: false });
    const edited = image(); PM.hist.undo(); const undone = image();
    const exportA = PM.GL.renderToPixels(0, 160, 100, { mblur: false });
    PM.rasterClear();
    const exportB = PM.GL.renderToPixels(0, 160, 100, { mblur: false });
    return { same: reference === optimized, stats, changed: edited !== optimized, undone: undone === optimized, exportSame: exportA.every((v: number, i: number) => v === exportB[i]) };
  });
  expect(result).toMatchObject({ same: true, changed: true, undone: true, exportSame: true, stats: { size: 0, bytes: 0 } });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('800% Retina text culling preserves alignment, wrapping, glyph edges and offscreen behavior', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM, results = [];
    const read = (sourceClipping: boolean) => {
      PM.GL.render(0, { mblur: false, sourceClipping });
      const gl = PM.GL.gl, pixels = new Uint8Array(320 * 240 * 4);
      gl.readPixels(0, 0, 320, 240, gl.RGBA, gl.UNSIGNED_BYTE, pixels); return pixels;
    };
    for (const align of ['left', 'center', 'right']) for (const angle of [0, 17, 90]) for (const paragraph of [false, true]) {
      const layer = PM.mkLayer('text', { d: { text: 'Typography café Ågj — 800%\nSecond line', font: 'Arial', size: 18.5, weight: 500, italic: true, tracking: .35, leading: 1.23, align, color: '#aabbcc', boxWidth: paragraph ? 115 : 0, boxHeight: paragraph ? 65 : 0 }, p: { 'position.x': 400, 'position.y': 250, rotation: angle, opacity: 67, 'scale.x': angle === 90 ? -100 : 100 } });
      PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.touch();
      for (const x of [380.25, 400.75, 425.125, 700]) {
        PM.GL.resize(320, 240, { x, y: 252.125, width: 20, height: 15, compWidth: 800, compHeight: 500 });
        const reference = read(false); PM.rasterClear(); const cropped = read(true);
        let different = 0, max = 0;
        cropped.forEach((value, index) => { const delta = Math.abs(value - reference[index]!); if (delta) different++; max = Math.max(max, delta); });
        results.push({ align, angle, paragraph, x, different, max, bytes: PM.rasterStats().bytes });
      }
    }
    return results;
  });
  console.log('TEXT_PIXEL_DIFFS', JSON.stringify(result.filter(r => r.different)));
  expect(result.every(r => r.different === 0)).toBe(true);
  expect(result.filter(r => r.x === 700).every(r => r.bytes === 0)).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('opaque coverage preserves shadow, text and edge pixels across fractional views and edits', async ({session}) => {
  const result = await session.page.evaluate(() => {
    const PM=(window as any).PM, GL=PM.GL;
    const shadows=Array.from({length:32},(_,i)=>PM.mkLayer('shape',{d:{shape:'rect',w:700+i,h:420+i/2,radius:25+i/2,color:'#000000'},p:{'position.x':400,'position.y':255,opacity:.9}}));
    const plate=PM.mkLayer('shape',{d:{shape:'rect',w:650,h:375,radius:25,stroke:2,strokeColor:'#aabbee',color:'#eeeeef'},p:{'position.x':400,'position.y':250}});
    const text=PM.mkLayer('text',{d:{text:'Media Aa',size:28,color:'#222222'},p:{'position.x':180,'position.y':130}});
    PM.proj.layers=[text,plate,...shadows];PM.ProjectIndex.invalidate();PM.touch();
    const results=[];
    const image=(enabled:boolean)=>{GL.render(0,{mblur:false,occlusionCulling:enabled});const pixels=new Uint8Array(480*320*4);GL.gl.readPixels(0,0,480,320,GL.gl.RGBA,GL.gl.UNSIGNED_BYTE,pixels);return pixels;};
    for(const opacity of [100,75]) for(const rotation of [0,17]) for(const x of [45.25,95.75,300.5]) for(const y of [40.75,150.25]) {
      plate.p.opacity.v=opacity;plate.p.rotation.v=rotation;PM.touch();
      GL.resize(480,320,{x,y,width:150,height:100,compWidth:800,compHeight:500});
      const reference=image(false), optimized=image(true);
      let different=0,max=0;for(let i=0;i<reference.length;i++){const d=Math.abs(reference[i]!-optimized[i]!);if(d)different++;max=Math.max(max,d);}
      results.push({opacity,rotation,x,y,different,max});
    }
    return results;
  });
  console.log('OCCLUSION_PIXELS',JSON.stringify(result.filter(x=>x.different)));
  expect(result.every(x=>x.different===0)).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
