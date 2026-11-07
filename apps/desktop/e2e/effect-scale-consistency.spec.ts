import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { test, expect } from './helpers/app';

// Opt in to the complete catalog/zoom/resolution sweep; the ordinary suite
// keeps the effects which previously mixed screen and composition pixels.
test('effects retain composition-space sizes at viewer zoom and preview resolution', async ({ session }, testInfo) => {
  test.setTimeout(240_000);
  await session.openEditor();
  const full = process.env.PM_EFFECT_SCALE_CATALOG === '1';
  const result = await session.page.evaluate(async ({ full }) => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
    PM.pause(); PM.agentFrameCapture = true; PM.AutomaticPreview?.cancel(); PM.perf.auto = false;
    const project = PM.mkProject({ name: 'Effect scale regression', w: 192, h: 128, dur: 2 });
    project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    const layer = PM.mkLayer('shape', { d: { w: 144, h: 96, radius: 0, color: '#ffffff' },
      p: { 'position.x': 96, 'position.y': 64 } }, project);
    const provider=PM.mkLayer('shape',{name:'Hidden input',d:{w:144,h:96,radius:0,color:'#a0c0e0'},
      p:{'position.x':96,'position.y':64}},project);provider.on=false;
    const mask=PM.mkMask('rect',project);mask.on=false;mask.p.w.v=110;mask.p.h.v=64;provider.masks=[mask];
    project.layers = [layer,provider]; PM.replaceProject(project);
    const texture=document.createElement('canvas');texture.width=64;texture.height=64;
    const tc=texture.getContext('2d')!,gradient=tc.createLinearGradient(0,0,64,64);
    gradient.addColorStop(0,'#204080');gradient.addColorStop(1,'#f0c080');tc.fillStyle=gradient;tc.fillRect(0,0,64,64);
    const blob=await new Promise<Blob>(resolve=>texture.toBlob(blob=>resolve(blob!),'image/png'));
    const image=await PM.assets.add(new File([blob],'Scale texture.png',{type:'image/png'}),{silent:true});
    const lut=await PM.assets.add(new File(['LUT_1D_SIZE 2\n1 1 1\n0 0 0'],'Scale inverse.cube'),{silent:true});
    const names = full ? Object.keys(PM.FX).sort() : ['blur', 'glow', 'shadow', 'grain', 'motionblurDir', 'chroma', 'displace', 'sharpen'];
    const settings: Record<string, Record<string, number>> = {
      blur: { amount: 12 }, shadow: { distance: 12, softness: 12 }, glow: { radius: 12 }, grain: { amount: 60, size: 1.4 },
      motionblurDir: { amount: 18, angle: 35 }, chroma: { amount: 12 }, displace: { amount: 16 }, sharpen: { amount: 100 },
      unsharpMask: { radius: 3 }, gradient: { rampScatter: 15 },
    };
    const canvas = (pixels: Uint8Array, w: number, h: number) => {
      const cv = document.createElement('canvas'); cv.width = w; cv.height = h;
      const data = new Uint8ClampedArray(pixels.length);
      // renderToPixels/readPixels use bottom-left rows.
      for (let y = 0; y < h; y++) data.set(pixels.subarray(y*w*4,(y+1)*w*4),(h-1-y)*w*4);
      // The viewer drawing buffer is opaque; compare the same premultiplied
      // RGB presentation on black, rather than treating it as straight alpha.
      for(let i=3;i<data.length;i+=4)data[i]=255;
      cv.getContext('2d')!.putImageData(new ImageData(data,w,h),0,0); return cv;
    };
    const render = (w: number, h: number) => canvas(PM.GL.renderToPixels(.25,w,h,{transparent:true,mblur:false}),w,h);
    const compare = (actual: HTMLCanvasElement, expected: HTMLCanvasElement, region: number[]) => {
      const a = actual.getContext('2d')!.getImageData(0,0,actual.width,actual.height).data;
      const b = expected.getContext('2d')!.getImageData(0,0,expected.width,expected.height).data;
      let sum=0, max=0,samples=0;
      for(let y=region[1]!;y<region[3]!;y++)for(let x=region[0]!;x<region[2]!;x++)for(let c=0;c<4;c++){
        const i=(y*actual.width+x)*4+c,d=Math.abs(a[i]!-b[i]!);sum+=d;max=Math.max(max,d);samples++;
      }
      return { mae: sum/samples, max, samples };
    };
    const rows: any[] = [], images: any[] = [];
    for (const type of names) {
      const ramp=PM.mkEffect('gradient',undefined,layer);
      ramp.p.startColor.v='#2e52a6';ramp.p.endColor.v='#ffcc40';ramp.p.endX.v=144;ramp.p.endY.v=96;
      const fx = PM.mkEffect(type,undefined,layer); layer.fx = [ramp,fx];
      const params=PM.Kernel.effects.get(type).params;
      provider.masks=params.some((p:any)=>p.type==='path')?[mask]:[];
      project.layers=params.some((p:any)=>p.type==='layer'||p.type==='path')?[layer,provider]:[layer];
      for(const param of params){
        if(param.type==='layer')fx.inputs[param.k]={layerId:provider.id,stage:'source'};
        if(param.type==='path')fx.inputs[param.k]={kind:'path',layerId:provider.id,pathId:mask.id};
        if(param.type==='asset')fx.inputs[param.k]={kind:'asset',assetId:param.assetKind==='lut'?lut.id:image.id};
      }
      for (const [key,value] of Object.entries(settings[type] || {})) if(fx.p[key])fx.p[key].v=value;
      PM.touch(); const reference = render(192,128);
      for (const zoom of (full ? [.125,.25,.5,1,2,4,8,'fit'] : [.25,1,4])) {
        for (const quality of (full ? [1,.5,.25,'auto'] : [1,.25])) {
          viewer.fit = zoom==='fit'; viewer.zoom = zoom==='fit'?1:zoom; viewer.pan=[0,0];
          PM.quality = quality==='auto'?1:quality; PM.previewResolution=quality==='auto'?'auto':String(quality); viewer.layout();
          PM.GL.render(.25,{mblur:false,exporting:true});
          const cv=PM.GL.canvas, gl=PM.GL.gl, bytes=new Uint8Array(cv.width*cv.height*4);
          gl.readPixels(0,0,cv.width,cv.height,gl.RGBA,gl.UNSIGNED_BYTE,bytes);
          const actual=canvas(bytes,cv.width,cv.height), expected=document.createElement('canvas');
          expected.width=cv.width;expected.height=cv.height;
          const ctx=expected.getContext('2d')!;ctx.imageSmoothingQuality='high';ctx.fillStyle='#000';ctx.fillRect(0,0,cv.width,cv.height);
          const viewport=PM.GL.previewViewport;
          const rect=viewport?{x:viewport.x,y:viewport.y,w:viewport.width,h:viewport.height}:{x:0,y:0,w:192,h:128};
          if(cv.width<rect.w||cv.height<rect.h){
            // Explicit area resampling: Canvas's default low-quality 4:1
            // resize only samples the central 2x2 pixels, aliasing fine grain.
            const source=reference.getContext('2d')!.getImageData(0,0,192,128).data;
            const data=ctx.createImageData(cv.width,cv.height),dx=rect.w/cv.width,dy=rect.h/cv.height;
            for(let y=0;y<cv.height;y++)for(let x=0;x<cv.width;x++){
              const loX=rect.x+x*dx,loY=rect.y+y*dy,hiX=loX+dx,hiY=loY+dy,at=(y*cv.width+x)*4;
              const color=[0,0,0];
              for(let sy=Math.floor(loY);sy<Math.ceil(hiY);sy++)for(let sx=Math.floor(loX);sx<Math.ceil(hiX);sx++){
                if(sx<0||sx>=192||sy<0||sy>=128)continue;
                const weight=(Math.min(sx+1,hiX)-Math.max(sx,loX))*(Math.min(sy+1,hiY)-Math.max(sy,loY))/(dx*dy);
                for(let c=0;c<3;c++)color[c]!+=source[(sy*192+sx)*4+c]!*weight;
              }
              for(let c=0;c<3;c++)data.data[at+c]=Math.round(color[c]!);
              data.data[at+3]=255;
            }
            ctx.putImageData(data,0,0);
          }else{ctx.imageSmoothingQuality='low';ctx.drawImage(reference,rect.x,rect.y,rect.w,rect.h,0,0,cv.width,cv.height);}
          // Overscan is deliberately disposable padding. Compare the picture
          // visible in the viewer, not the artificial outer edge of its FBO.
          let region=[0,0,cv.width,cv.height];
          if(viewport){
            const stage=viewer.stage.getBoundingClientRect(),left=parseFloat(viewer.inner.style.left),top=parseFloat(viewer.inner.style.top),z=viewer.zoom;
            const px=(x:number)=>Math.max(0,Math.min(cv.width,Math.round((x-viewport.x)*cv.width/viewport.width)));
            const py=(y:number)=>Math.max(0,Math.min(cv.height,Math.round((y-viewport.y)*cv.height/viewport.height)));
            region=[px(Math.max(0,-left)/z),py(Math.max(0,-top)/z),px(Math.min(192,(stage.width-left)/z)),py(Math.min(128,(stage.height-top)/z))];
          }
          rows.push({type,zoom,quality,inputs:Object.keys(fx.inputs??{}),cropped:!!viewport,width:cv.width,height:cv.height,...compare(actual,expected,region)});
          if(['blur','glow','shadow','grain','displace'].includes(type)&&[.25,1,4].includes(zoom as number)&&(quality===1||quality==='auto'))
            images.push({type,zoom,quality,region,image:actual.toDataURL(),expected:expected.toDataURL()});
        }
      }
    }
    return { rows, images, errors: [...PM.GL.errors] };
  }, { full });
  const destination = process.env.PM_EFFECT_SCALE_OUTPUT;
  if (destination) {
    await mkdir(destination,{recursive:true});
    await writeFile(path.join(destination,'table.json'),JSON.stringify(result.rows,null,2));
    const grouped = Map.groupBy(result.rows,row=>row.type);
    await writeFile(path.join(destination,'table.md'), '| Effect | Worst mean difference (0–255) |\n|---|---:|\n'+[...grouped].map(([type,rows])=>`| ${type} | ${Math.max(...rows.map(r=>r.mae)).toFixed(3)} |`).join('\n')+'\n');
    for (const item of result.images) {
      await writeFile(path.join(destination,`${item.type}-${item.zoom}-${item.quality}.png`),Buffer.from(item.image.split(',')[1],'base64'));
      await writeFile(path.join(destination,`${item.type}-${item.zoom}-${item.quality}-reference.png`),Buffer.from(item.expected.split(',')[1],'base64'));
    }
    await writeFile(path.join(destination,'image-regions.json'),JSON.stringify(result.images.map(({image,expected,...item})=>item),null,2));
  }
  await testInfo.attach('effect-scale-table',{body:JSON.stringify(result.rows,null,2),contentType:'application/json'});
  expect(result.errors).toEqual([]);
  for (const row of result.rows) expect(row.mae,JSON.stringify(row)).toBeLessThan(3);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('extension shader scale stays aligned on scaled, 3D, source and precomp owners', async ({session},testInfo) => {
  await session.openEditor();
  const result=await session.page.evaluate(()=>{
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');
    PM.pause();PM.agentFrameCapture=true;PM.AutomaticPreview?.cancel();PM.perf.auto=false;
    PM.Kernel.registerEffect('scale-regression',{
      id:'scaleProbe',label:'Scale probe',group:'Test',params:[],sourceProcessing:{},viewportSafe:true,viewportPaddingPixels:4,
      // Generated shaders, as used by extension/validate_effect authors.
      frag:`vec4 c=texture(u_tex,v_st);
        vec2 p=effectPixel();
        float stripe=mod(floor(p.x/8.),2.);
        vec4 neighbor=texture(u_tex,v_st+vec2(4.,0.)*u_effectScale*u_texel);
        o=vec4(mix(vec3(.2),vec3(.8,.4,.2),stripe)*c.a*.7+neighbor.rgb*.3,c.a);`,
    });
    const rows:any[]=[];
    for(const context of ['plain','scaled','3D','source','precomp']){
      const p=PM.mkProject({w:192,h:128,dur:2});p.backgroundFill={type:'none',stops:[],angle:0};
      const layer=PM.mkLayer('shape',{d:{w:96,h:64,radius:0,color:'#ffffff'},p:{'position.x':96,'position.y':64}},p);
      layer.fx=[PM.mkEffect('scaleProbe',undefined,layer)];
      if(context==='scaled'){layer.p['scale.x'].v=150;layer.p['scale.y'].v=75;}
      if(context==='3D')layer.threeD=true;
      if(context==='source'){
        layer.effectSpace='source';const blur=PM.mkEffect('blur',undefined,layer);blur.p.amount.v=8;layer.fx.unshift(blur);
      }
      p.layers=[layer];PM.replaceProject(p);
      if(context==='precomp')PM.Comps.precompose([layer.id],{sizeToSelection:false});
      const make=(pixels:Uint8Array,w:number,h:number)=>{
        const cv=document.createElement('canvas');cv.width=w;cv.height=h;const data=new Uint8ClampedArray(pixels);
        for(let i=3;i<data.length;i+=4)data[i]=255;
        // Both render paths are bottom-up; flip only for Canvas resampling.
        const top=new Uint8ClampedArray(data.length);for(let y=0;y<h;y++)top.set(data.subarray(y*w*4,(y+1)*w*4),(h-1-y)*w*4);
        cv.getContext('2d')!.putImageData(new ImageData(top,w,h),0,0);return cv;
      };
      const reference=make(PM.GL.renderToPixels(0,192,128,{transparent:true,mblur:false}),192,128);
      for(const quality of [1,.5,.25])for(const zoom of [.25,1,4]){
        PM.quality=quality;PM.previewResolution=String(quality);viewer.fit=false;viewer.zoom=zoom;viewer.pan=[0,0];PM.touch();viewer.layout();
        PM.GL.render(0,{mblur:false,exporting:true});const cv=PM.GL.canvas,gl=PM.GL.gl,pixels=new Uint8Array(cv.width*cv.height*4);
        gl.readPixels(0,0,cv.width,cv.height,gl.RGBA,gl.UNSIGNED_BYTE,pixels);
        const actual=make(pixels,cv.width,cv.height),expected=document.createElement('canvas');expected.width=cv.width;expected.height=cv.height;
        const ctx=expected.getContext('2d')!;ctx.fillStyle='#000';ctx.fillRect(0,0,cv.width,cv.height);const v=PM.GL.previewViewport;
        if(v)ctx.drawImage(reference,v.x,v.y,v.width,v.height,0,0,cv.width,cv.height);else ctx.drawImage(reference,0,0,cv.width,cv.height);
        const a=actual.getContext('2d')!.getImageData(0,0,cv.width,cv.height).data,b=ctx.getImageData(0,0,cv.width,cv.height).data;
        let sum=0;for(let i=0;i<a.length;i++)sum+=Math.abs(a[i]!-b[i]!);rows.push({context,zoom,quality,mae:sum/a.length});
      }
    }
    return{rows,errors:[...PM.GL.errors]};
  });
  await testInfo.attach('extension-context-scale',{body:JSON.stringify(result,null,2),contentType:'application/json'});
  expect(result.errors).toEqual([]);
  for(const row of result.rows)expect(row.mae,JSON.stringify(row)).toBeLessThan(6);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('wide kernels at 800% reuse a bounded full-frame preview matching export',async({session})=>{
  await session.openEditor();
  const result=await session.page.evaluate(()=>{
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');
    PM.pause();PM.agentFrameCapture=true;PM.AutomaticPreview?.cancel();PM.perf.auto=false;PM.quality=1;PM.previewResolution='1';
    const project=PM.mkProject({w:1024,h:576,dur:2});project.backgroundFill={type:'none',stops:[],angle:0};
    const layer=PM.mkLayer('shape',{d:{w:400,h:200,radius:0,color:'#4080ff'},p:{'position.x':512,'position.y':288}},project);
    const blur=PM.mkEffect('blur',undefined,layer);blur.p.amount.v=400;layer.fx=[blur];project.layers=[layer];PM.replaceProject(project);
    const reference=PM.GL.renderToPixels(0,1024,576,{transparent:true,mblur:false});
    viewer.fit=false;viewer.zoom=8;viewer.pan=[0,0];viewer.layout();PM.GL.render(0,{exporting:true,mblur:false});
    const cv=PM.GL.canvas,gl=PM.GL.gl,pixels=new Uint8Array(cv.width*cv.height*4);gl.readPixels(0,0,cv.width,cv.height,gl.RGBA,gl.UNSIGNED_BYTE,pixels);
    let max=0;for(let i=0;i<pixels.length;i++)if(i%4!==3)max=Math.max(max,Math.abs(pixels[i]!-reference[i]!));
    return{width:cv.width,height:cv.height,viewport:PM.GL.previewViewport,max,errors:[...PM.GL.errors]};
  });
  expect(result).toEqual({width:1024,height:576,viewport:null,max:0,errors:[]});
  expect(session.diagnostics.pageErrors).toEqual([]);
});
