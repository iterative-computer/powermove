import { expect, test } from './helpers/app';

test('agent frame capture preserves the displayed canvas before the next animation frame', async ({ session }) => {
  await session.openEditor();
  await session.page.waitForFunction(() => Boolean((window as any).PM.GL.canvas));
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause();
    const gl = PM.GL.gl as WebGL2RenderingContext;
    const canvas = PM.GL.canvas;
    const width = canvas.width, height = canvas.height;
    // A distinctive displayed frame, independent of the sample project's content.
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.clearColor(.8, .2, .4, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    const pixel = () => {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      const bytes = new Uint8Array(4);
      gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
      return [...bytes];
    };
    const before = pixel();
    const image = PM.Export.snapshot(0, 160);
    const after = pixel();
    const dimensions = [canvas.width, canvas.height];
    const originalRender = PM.GL.renderProject;
    let failurePixel: number[];
    try {
      PM.GL.renderProject = () => { throw new Error('capture failure'); };
      try { PM.Export.snapshot(0, 160); } catch {}
      failurePixel = pixel();
    } finally { PM.GL.renderProject = originalRender; }
    return { before, after, failurePixel, dimensions, expectedDimensions: [width, height], image };
  });
  expect(result.image).toMatch(/^data:image\/jpeg;base64,/);
  expect(result.after).toEqual(result.before);
  expect(result.failurePixel).toEqual(result.before);
  expect(result.dimensions).toEqual(result.expectedDimensions);
});

test('offscreen captures match opaque presentation without resizing or replacing a zoomed preview', async ({session}) => {
  await session.openEditor();
  const {page} = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({name:'Capture equivalence',w:640,h:360,fps:30,dur:2});
    project.bg = '#142839';
    project.layers = [
      PM.mkLayer('shape',{d:{shape:'rect',w:241,h:119,radius:13,color:'#dd5522'},p:{'position.x':310,'position.y':170,'rotation':17,'opacity':53}}),
      PM.mkLayer('text',{d:{text:'Capture Aa',size:37,color:'#aadcff'},p:{'position.x':180,'position.y':115}}),
    ];
    window.dispatchEvent(new CustomEvent('pm-open-project',{detail:project})); PM.ProjectsScreen.hide(); PM.agentFrameCapture=true;
  });
  await page.waitForFunction(()=>{const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');return Boolean(PM.GL?.gl&&viewer?.stage);});
  const result = await page.evaluate(() => {
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer'),GL=PM.GL;
    const results=[];
    for(const transparent of [false,true]) {
      PM.proj.backgroundFill = {type:transparent?'none':'solid',stops:[{color:'#142839',position:0}],angle:0};
      viewer.fit=false;viewer.zoom=4.371588852276498;viewer.pan=[17.25,-11.5];viewer.layout();GL.render(0,{mblur:false});
      const before=viewer.el.toDataURL(),viewport=GL.previewViewport,dimensions=[viewer.el.width,viewer.el.height];
      const resize=GL.resize;let resizes=0;
      GL.resize=(...args:any[])=>{resizes++;return resize(...args);};
      let captured:HTMLCanvasElement;
      try { captured=PM.renderFrameTo(0,320,180,{mblur:false}); } finally {GL.resize=resize;}
      const preserved=before===viewer.el.toDataURL()&&viewport===GL.previewViewport&&dimensions[0]===viewer.el.width&&dimensions[1]===viewer.el.height;
      const actual=captured!.getContext('2d')!.getImageData(0,0,320,180).data;
      // The old capture's pixel reference: normal opaque presentation at the
      // target dimensions, copied to Canvas2D. Only the test resizes the viewer.
      GL.resize(320,180,null);GL.render(0,{mblur:false});
      const reference=document.createElement('canvas');reference.width=320;reference.height=180;
      const context=reference.getContext('2d')!;context.drawImage(GL.canvas,0,0);
      const expected=context.getImageData(0,0,320,180).data;
      let max=0,different=0;for(let i=0;i<actual.length;i++){const delta=Math.abs(actual[i]!-expected[i]!);max=Math.max(max,delta);if(delta)different++;}
      results.push({preserved,resizes,max,different});
    }
    return results;
  });
  expect(result).toEqual([{preserved:true,resizes:0,max:0,different:0},{preserved:true,resizes:0,max:0,different:0}]);
  expect(session.diagnostics.pageErrors).toEqual([]);
  await page.evaluate(async()=>await (window as any).PM.flushProject());
  await session.app.evaluate(({app})=>app.exit(0)).catch(()=>undefined);
});

test('autosave persists during panning and captures its thumbnail after navigation settles', async ({session}) => {
  await session.openEditor();
  const {page}=session;
  await page.evaluate(()=>{
    const PM=(window as any).PM;
    PM.agentFrameCapture=true;
    window.dispatchEvent(new CustomEvent('pm-open-project',{detail:PM.mkProject({name:'Thumbnail navigation',w:640,h:360,fps:30,dur:2})}));
    PM.ProjectsScreen.hide();
  });
  await page.waitForFunction(()=>{const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');return Boolean(PM.GL?.gl&&viewer?.stage);});
  const result=await page.evaluate(async()=>{
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');
    viewer.fit=false;viewer.zoom=4.37;viewer.pan=[0,0];viewer.layout();
    const original=PM.Export.snapshot;let captures=0;
    PM.Export.snapshot=(...args:any[])=>{captures++;return original(...args);};
    PM.agentFrameCapture=false;
    try {
      PM.autosave();
      const end=performance.now()+1100;
      while(performance.now()<end){
        viewer.pan[0]+=.25;viewer.layout(true);
        await new Promise(resolve=>setTimeout(resolve,16));
      }
      const during=captures;
      const saved=!!PM.Projects.get(PM.proj.id);
      await new Promise(resolve=>setTimeout(resolve,850));
      return {during,saved,after:captures};
    }finally{PM.Export.snapshot=original;}
  });
  expect(result).toEqual({during:0,saved:true,after:1});
  await page.evaluate(async()=>await (window as any).PM.flushProject());
  await session.app.evaluate(({app})=>app.exit(0)).catch(()=>undefined);
});
