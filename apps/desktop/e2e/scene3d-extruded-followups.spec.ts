import {mkdir,writeFile,readFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import path from 'node:path';
import {test,expect,repoRoot} from './helpers/app';
const shots='/tmp/pm-figuate-feedback/shots/extruded-followups';
const before=process.env.PM_FOLLOWUP_BEFORE==='1';
test.beforeAll(async()=>{await mkdir(shots,{recursive:true});});
async function setup(session:any,kind='text'){
  await session.openEditor();
  await session.page.evaluate((kind:string)=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({name:'Extrusion followups',w:960,h:540,dur:3,fps:30}));
    const api=PM.Kernel.api('followup-proof').scene3d;
    (window as any).followupAPI=api;
    if(kind==='model')api.edit({operation:'add_object',object:{id:'receiver',source:{primitive:'plane',parameters:{width:5,height:1.5}},material:{p:{color:'#DAD7FF'}}}});
    else{
      const add=PM.Edit.apply({type:'add_layer',id:'receiver',layerType:kind,content:kind==='text'?{text:'Powermove',font:'Arial',weight:700,size:100,align:'center',color:'#DAD7FF'}:{shape:'star',w:230,h:200,color:'#DAD7FF'},properties:{'position.x':480,'position.y':270}});if(!add.ok)throw new Error(add.message);
      const r=api.setExtrusion('receiver',{enabled:true,depth:40,bevelStyle:'angular',bevelDepth:3});if(!r.ok)throw new Error(r.message);
    }
    api.edit({operation:'add_camera',camera:{p:{x:3,y:2,z:8}}});
    api.edit({operation:'add_light',light:{type:'point',p:{x:-3,y:3,z:5,intensity:100}}});
    PM.selectLayers(['receiver']);PM.invalidate();
  },kind);
  await session.page.evaluate(async()=>{const PM=(window as any).PM;await PM.prepareFrame(0,{mblur:false});PM.GL.render(0,{mblur:false,previewReuse:false});});
}
async function shot(page:any,name:string){
  await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));
  await page.locator('#gl').evaluate((c:HTMLCanvasElement)=>c.toDataURL());
  await page.screenshot({path:`${shots}/${name}.png`});
}
async function handle(page:any,axis:string){
  return page.evaluate((axis:string)=>{
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer'),rect=document.querySelector('#stage')!.getBoundingClientRect();
    const canvas=document.querySelector('[data-scene-gizmo]') as HTMLElement;
    for(let y=rect.top+8;y<rect.bottom-8;y+=3)for(let x=rect.left+8;x<rect.right-8;x+=3)
      if(viewer.sceneGizmo.hover(x,y)&&canvas.dataset.gizmoAxis===axis)return {x,y};
    return null;
  },axis);
}
test('native gizmo drags all axes in the real viewer, with one undo and rendered midway PNGs',async({session})=>{
  test.setTimeout(90000);await setup(session);
  const page=session.page;
  await shot(page,before?'01-before-extruded-text-gizmo':'02-after-extruded-text-selected');
  for(const kind of ['text','shape']){
    if(kind==='shape')await setup(session,kind);
    for(const mode of ['translate','rotate','scale']){
      await page.evaluate(mode=>(window as any).followupAPI.setMode(mode),mode);
      for(const axis of ['X','Y','Z']){
        const point=await handle(page,axis);expect(point,`${kind} ${mode} has ${axis} handle`).not.toBeNull();
        const initial=await page.evaluate(()=>{const PM=(window as any).PM;return {pose:JSON.stringify(PM.L('receiver').p),index:PM.hist.mark().index};});
        await page.mouse.move(point!.x,point!.y);await page.mouse.down();await page.mouse.move(point!.x+30,point!.y-20,{steps:6});
        expect(await page.evaluate(()=>(window as any).PM.Kernel.services.get('viewer').sceneGizmo.active())).toBe(true);
        expect(await page.evaluate(()=>JSON.stringify((window as any).PM.L('receiver').p))).not.toBe(initial.pose);
        await shot(page,`02-after-${kind}-${mode}-${axis.toLowerCase()}-mid-drag`);
        const rendered=await page.evaluate(()=>{const PM=(window as any).PM,c=PM.renderFrameTo(0,960,540,{mblur:false});return c.toDataURL();});
        await writeFile(`${shots}/02-after-${kind}-${mode}-${axis.toLowerCase()}-render.png`,Buffer.from(rendered.split(',')[1],'base64'));
        await page.mouse.up();
        expect(await page.evaluate(()=>(window as any).PM.hist.mark().index)).toBe(initial.index+1);
        await page.evaluate(()=>(window as any).PM.hist.undo());
        expect(await page.evaluate(()=>JSON.stringify((window as any).PM.L('receiver').p))).toBe(initial.pose);
      }
    }
  }
  await expect(page.locator('#composition-3d-controls')).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for(const kind of ['text','model'])test(`${kind} stays visible through first video material load and seeking, from layer and composition sources`,async({session})=>{
  test.setTimeout(90000);await setup(session,kind);const page=session.page;
  const file=path.join(session.userData,'material.webm'),frames=Buffer.concat(Array.from({length:10},(_,i)=>{
    const frame=Buffer.alloc(32*32*3);for(let p=0;p<frame.length;p+=3)frame[p+(i<5?0:1)]=255;return frame;
  }));
  execFileSync(path.join(repoRoot,'node_modules/ffmpeg-static/ffmpeg'),['-v','error','-f','rawvideo','-pix_fmt','rgb24','-s','32x32','-r','10','-i','pipe:0','-c:v','libvpx-vp9','-lossless','1','-g','1',file],{input:frames});
  await page.evaluate(async(bytes:number[])=>{
    const PM=(window as any).PM,asset=await PM.assets.add(new File([new Uint8Array(bytes)],'material.webm',{type:'video/webm'}),{silent:true});
    PM.Edit.apply({type:'add_layer',id:'video',layerType:'video',visible:false,content:{asset:asset.id,w:32,h:32}});
    const comp=PM.Comps.create({name:'Video material',w:32,h:32,dur:1},{open:false});
    const layer=PM.mkLayer('video');layer.id='comp-video';layer.d={...layer.d,asset:asset.id,w:32,h:32};layer.p['position.x'].v=16;layer.p['position.y'].v=16;
    PM.proj.comps[comp].layers.push(layer);PM.touch();(window as any).materialComp=comp;
  },[...await readFile(file)]);
  for(const source of ['layer','comp']){
    const proof=await page.evaluate((source:string)=>{
      const PM=(window as any).PM,api=(window as any).followupAPI;
      const render=()=>{const c=PM.renderFrameTo(PM.time,960,540,{mblur:false}),data=c.getContext('2d').getImageData(0,0,960,540).data;let pixels=0,colored=0;for(let i=0;i<data.length;i+=4){if(Math.max(data[i],data[i+1],data[i+2])>30)pixels++;if(data[i]>data[i+1]+40 || data[i+1]>data[i]+40)colored++;}return {pixels,colored,png:c.toDataURL()};};
      api.setMaterialMap('receiver',0,'color',null);const original=render();
      const proto=HTMLMediaElement.prototype,ready=Object.getOwnPropertyDescriptor(proto,'readyState')!;
      const map=source==='layer'?{layerId:'video'}:{compositionId:(window as any).materialComp};
      let first:any,counts:number[]=[];
      try{
        Object.defineProperty(proto,'readyState',{configurable:true,get:()=>0});
        const r=api.setMaterialMap('receiver',0,'color',map);if(!r.ok)throw new Error(r.message);
        for(let i=0;i<5;i++){const frame=render();counts.push(frame.pixels);first=frame;}
      }finally{Object.defineProperty(proto,'readyState',ready);}
      (window as any).loadingMaterialRender=render;
      return {original:original.pixels,counts,png:first.png};
    },source);
    await writeFile(`${shots}/${before?'01-before':'02-after'}-${kind}-${source}-video-loading.png`,Buffer.from(proof.png.split(',')[1],'base64'));
    for(const count of proof.counts)expect(count).toBeGreaterThan(proof.original*.95);
    await expect.poll(()=>page.evaluate(()=>{const PM=(window as any).PM;PM.touch();const p=(window as any).loadingMaterialRender();return p.colored;})).toBeGreaterThan(1000);
    const seek=await page.evaluate(()=>{
      const PM=(window as any).PM,proto=HTMLMediaElement.prototype,descriptor=Object.getOwnPropertyDescriptor(proto,'seeking')!,render=(window as any).loadingMaterialRender;
      render();const counts:number[]=[];let png='';
      try{Object.defineProperty(proto,'seeking',{configurable:true,get:()=>true});for(let i=0;i<5;i++){PM.time=.1+i*.1;PM.touch();const frame=render();counts.push(frame.pixels);png=frame.png;}}
      finally{Object.defineProperty(proto,'seeking',descriptor);}
      return {counts,png};
    });
    for(const count of seek.counts)expect(count).toBeGreaterThan(proof.original*.95);
    await writeFile(`${shots}/02-after-${kind}-${source}-video-seeking.png`,Buffer.from(seek.png.split(',')[1],'base64'));
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
