import {test,expect} from './helpers/app';

test('CSS colored compound SVG and open fills match browser SVG rendering after import and reopen',async({session})=>{
  await session.openEditor();
  const result=await session.page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({name:'SVG fidelity',w:200,h:160,fps:30,dur:3}));
    const source=`<svg xmlns="http://www.w3.org/2000/svg" width="200" height="160" viewBox="0 0 200 160"><style>.red{fill:#e60012}.outline{fill:none;stroke:#1464f4;stroke-width:12;stroke-linecap:round;stroke-linejoin:round}</style><g class="red"><path fill-rule="evenodd" d="M10 10H90V80H10Z M30 30H70V60H30Z"/><path d="M110 10H190V80H110Z M130 30V60H170V30Z"/><path d="M10 100H80L40 150"/></g><path class="outline" d="M110 140L140 100L180 140"/></svg>`;
    await PM.importFiles([new File([source],'compound.svg',{type:'image/svg+xml'})]);
    const layer=PM.proj.layers[0];
    const image=new Image();image.src=URL.createObjectURL(new Blob([source],{type:'image/svg+xml'}));await image.decode();
    const canvas=document.createElement('canvas');canvas.width=800;canvas.height=640;const ctx=canvas.getContext('2d')!;ctx.drawImage(image,0,0,800,640);
    const reference=ctx.getImageData(0,0,800,640).data;
    const render=()=>{
      const pixels=PM.GL.renderToPixels(0,800,640,{transparent:true,mblur:false}),out=new Uint8Array(pixels.length);
      for(let y=0;y<640;y++)out.set(pixels.subarray(y*800*4,(y+1)*800*4),(639-y)*800*4);
      return out;
    };
    const before=render();let bad=0;for(let i=0;i<reference.length;i+=4){if(Math.abs(reference[i+3]-before[i+3])>20||reference[i+3]>240&&Math.max(...[0,1,2].map(c=>Math.abs(reference[i+c]-before[i+c])))>10)bad++;}
    const sample=(x:number,y:number)=>[...before.subarray((y*4*800+x*4)*4,(y*4*800+x*4)*4+4)];
    const saved=PM.serialize();PM.replaceProject(PM.hydrateProject(PM.deserialize(saved)));const reopened=render();
    return {badFraction:bad/(800*640),red:sample(20,20),hole:sample(40,40),secondHole:sample(140,40),openFill:sample(40,120),
      paths:layer.d.paths.length,round:layer.d.paths.at(-1).p.cap.v,identical:before.every((v,i)=>v===reopened[i]),errors:[...PM.GL.errors.entries()]};
  });
  expect(result.badFraction).toBeLessThan(.012);expect(result.red).toEqual([230,0,18,255]);expect(result.hole[3]).toBe(0);expect(result.secondHole[3]).toBe(0);
  expect(result.openFill).toEqual([230,0,18,255]);expect(result.round).toBe('ROUND');expect(result.identical).toBe(true);expect(result.errors).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('editable path caps and joins change through native controls, undo once, and survive save',async({session},testInfo)=>{
  await session.openEditor();
  await session.page.emulateMedia({reducedMotion:'reduce'});
  await session.page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({name:'Path stroke controls',w:240,h:160,dur:3}));
    await PM.importFiles([new File(['<svg xmlns="http://www.w3.org/2000/svg" width="240" height="160"><path fill="none" stroke="#e60012" stroke-width="20" d="M40 120L120 40L200 120"/></svg>'],'stroke.svg',{type:'image/svg+xml'})]);
    (window as any).__strokeBefore=PM.renderFrameTo(0,480,320,{mblur:false}).toDataURL();
    (window as any).__strokeHistory=PM.hist.list().length;
  });
  const panel=session.page.locator('#panel-inspector');
  for(const name of ['Start','End','Join']){
    const trigger=panel.getByRole('combobox',{name,exact:true});
    await trigger.locator('..').locator('select').selectOption({label:'Round'},{force:true});
  }
  const result=await session.page.evaluate(()=>{
    const PM=(window as any).PM,layer=PM.proj.layers[0],p=layer.d.paths[0].p;
    const frame=()=>PM.renderFrameTo(0,480,320,{mblur:false}).toDataURL();
    const after=frame(),count=PM.hist.list().length-(window as any).__strokeHistory,values=[p.startCap.v,p.endCap.v,p.join.v];
    PM.hist.undo();const undo=PM.proj.layers[0].d.paths[0].p.join.v;PM.hist.redo();
    PM.replaceProject(PM.hydrateProject(PM.deserialize(PM.serialize())));return {values,count,undo,changed:after!==(window as any).__strokeBefore,reopened:frame()===after};
  });
  expect(result).toEqual({values:['ROUND','ROUND','round'],count:3,undo:'miter',changed:true,reopened:true});
  await panel.getByRole('combobox',{name:'Join',exact:true}).scrollIntoViewIfNeeded();
  for(const theme of ['light','dark']){
    await session.page.evaluate(theme=>{document.documentElement.dataset.theme=theme;},theme);
    await panel.screenshot({path:testInfo.outputPath(`stroke-controls-${theme}.png`)});
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('line shapes expose round caps without changing their direction or color',async({session})=>{
  await session.openEditor();
  await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();PM.replaceProject(PM.mkProject({name:'Line caps',w:240,h:160,dur:3}));
    const result=PM.Edit.apply({type:'add_layer',id:'line',layerType:'shape',content:{shape:'line',w:160,h:30,color:'#E60012',stroke:20},select:true});if(!result.ok)throw new Error(result.message);
    const raw=PM.GL.renderToPixels(0,240,160,{transparent:true,mblur:false});(window as any).__lineBefore=[raw[(79*240+120)*4],raw[(79*240+34)*4+3]];
  });
  const start=session.page.locator('#panel-inspector').getByRole('combobox',{name:'Start',exact:true});
  await start.locator('..').locator('select').selectOption({label:'None'},{force:true});
  const result=await session.page.evaluate(()=>{
    const PM=(window as any).PM,raw=PM.GL.renderToPixels(0,240,160,{transparent:true,mblur:false});
    const pixel=(x:number,y:number)=>[...raw.subarray(((159-y)*240+x)*4,((159-y)*240+x)*4+4)];
    const cap=PM.L('line').d.strokeStyle.startCap;
    return {before:(window as any).__lineBefore,center:pixel(120,80),end:pixel(34,80),start:typeof cap==='string'?cap:cap.v};
  });
  expect(result.before).toEqual([230,255]);expect(result.center).toEqual([230,0,18,255]);expect(result.end[3]).toBe(0);expect(result.start).toBe('NONE');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('untouched old SVG imports migrate without a user edit or repeated parsing',async({session})=>{
  await session.openEditor();
  const result=await session.page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({name:'Legacy SVG repair',w:200,h:160,dur:3}));
    await PM.importFiles([new File(['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 160"><style>.red{fill:#e60012}</style><path class="red" d="M10 10H190V150H10Z M40 40V120H160V40Z"/></svg>'],'old.svg',{type:'image/svg+xml'})]);
    const layer=PM.proj.layers[0];delete layer.d.svgImportVersion;
    for(const path of layer.d.paths){for(const key of ['fillGroup','fillOpen','fillRule','cap','join','miterLimit'])delete path.p[key];path.p.fill.v='#000000';}
    const animated=PM.Edit.apply({type:'replace_keyframes',target:layer.id,path:'rotation',keyframes:[{time:0,value:0},{time:2,value:30}]});if(!animated.ok)throw new Error(animated.message);
    PM.ev(layer,'rotation',.5);PM.touch();
    const ids=layer.d.paths.map((p:any)=>[p.id,p.vertices.map((v:any)=>v.id)]),animation=JSON.stringify(layer.p),count=PM.hist.list().length;
    PM.assets.clear();await PM.assets.restoreProject(PM.proj);
    const repaired=PM.L(layer.id),color=repaired.d.paths[0].p.fill.v,noUndo=PM.hist.list().length===count;
    const sameIds=JSON.stringify(ids)===JSON.stringify(repaired.d.paths.map((p:any)=>[p.id,p.vertices.map((v:any)=>v.id)])),sameAnimation=JSON.stringify(repaired.p)===animation;
    const checked=repaired.d.svgImportCheckedVersion;
    PM.assets.clear();await PM.assets.restoreProject(PM.proj);
    return {color,noUndo,sameIds,sameAnimation,checked,stillNoUndo:PM.hist.list().length===count};
  });
  expect(result).toEqual({color:'#E60012',noUndo:true,sameIds:true,sameAnimation:true,checked:2,stillNoUndo:true});
  expect(session.diagnostics.pageErrors).toEqual([]);
});
