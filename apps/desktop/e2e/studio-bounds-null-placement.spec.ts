import {test,expect} from './helpers/app';

async function nullOverlaySample(page:any,id:string){
  return page.evaluate((id:string)=>{
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer'),marker=viewer.nullMarkers().find((m:any)=>m.id===id),ctx=viewer.ov.getContext('2d'),ratio=viewer.ov.width/viewer.stage.clientWidth;
    const alpha=(point:any)=>Math.max(...[...ctx.getImageData(Math.round(point.x*ratio)-2,Math.round(point.y*ratio)-2,5,5).data].filter((_:any,i:number)=>i%4===3));
    const a=marker.corners[0],b=marker.corners[1];
    return {corner:alpha(a),edge:alpha({x:(a.x+b.x)/2,y:(a.y+b.y)/2}),pivot:alpha(marker),cursor:getComputedStyle(viewer.stage).cursor};
  },id);
}
async function checkNullHover(page:any,id:string,x:number,y:number){
  await expect.poll(()=>page.evaluate(()=>(window as any).PM.Kernel.services.get('viewer').isNavigating())).toBe(false);
  const idle=await nullOverlaySample(page,id);
  await page.mouse.move(x,y);
  await expect.poll(()=>page.evaluate(()=>(window as any).PM.Kernel.services.get('viewer').hoverLayerId())).toBe(id);
  await expect.poll(async()=>(await nullOverlaySample(page,id)).corner).toBeGreaterThan(idle.corner+50);
  const hovered=await nullOverlaySample(page,id);
  expect(hovered.edge).toBeGreaterThan(idle.edge+50);
  expect(hovered.pivot).toBe(0);expect(hovered.cursor).toBe('default');
  expect(await page.evaluate(()=>(window as any).PM.sel.layers)).toEqual([]);
  await page.mouse.move(0,0);
  await expect.poll(async()=>(await nullOverlaySample(page,id)).corner).toBe(idle.corner);
}

test('all nine model null pivots, visible markers and gizmos agree in composition pixels',async({session})=>{
  await session.openEditor();
  const result=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('null-placement-proof'),viewer=PM.Kernel.services.get('viewer');PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({name:'Null placement',w:640,h:360,dur:3}));
    const check=(r:any)=>{if(!r.ok)throw Error(r.message);return r.data;};
    check(api.scene3d.edit({operation:'add_camera',camera:{projection:'orthographic',p:{x:0,y:0,z:6,zoom:1}}}));
    check(api.scene3d.edit({operation:'add_object',object:{id:'model',source:{primitive:'box',parameters:{width:2,height:1,depth:.4}},p:{x:1,y:.5}}}));
    viewer.fit=true;viewer.layout();api.scene3d.setMode('translate');
    const frame=()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))),out:any[]=[];
    for(let index=0;index<9;index++){
      const id=check(api.groups.createBoundsNull(['model'],index)).result.nullIds[0];PM.invalidate();await frame();
      const marker=viewer.nullMarkers().find((m:any)=>m.id===id),stage=viewer.stage.getBoundingClientRect(),canvas=document.querySelector('#gl')!.getBoundingClientRect();
      const expected={x:canvas.left-stage.left+(320+(index%3)*60)*canvas.width/640,y:canvas.top-stage.top+(120+Math.floor(index/3)*30)*canvas.height/360};
      const hover=viewer.sceneGizmo.hover(stage.left+expected.x,stage.top+expected.y);
      out.push({index,marker,expected,selected:PM.sel.layers[0]===id,covers:viewer.sceneGizmo.covers([id]),hover});
      PM.hist.undo();PM.invalidate();await frame();
    }
    const id=check(api.groups.createBoundsNull(['model'],4)).result.nullIds[0];
    const before=PM.renderFrameTo(0,320,180,{mblur:false}).toDataURL();PM.selectLayers([]);PM.invalidate();await frame();
    const unselected=viewer.nullMarkers().find((m:any)=>m.id===id),ctx=viewer.ov.getContext('2d'),ratio=viewer.ov.width/viewer.stage.clientWidth;
    const a=unselected.corners[0],b=unselected.corners[1],edge={x:a.x+(b.x-a.x)*.25,y:a.y+(b.y-a.y)*.25};
    const pixels=ctx.getImageData(Math.round(edge.x*ratio)-2,Math.round(edge.y*ratio)-2,5,5).data;
    const pivotPixels=ctx.getImageData(Math.round(unselected.x*ratio)-2,Math.round(unselected.y*ratio)-2,5,5).data;
    const anchorHidden=[...pivotPixels].every((value:any,i:number)=>i%4!==3||value===0);
    const visible=[...pixels].some((value:any,i:number)=>i%4===3&&value>0);
    viewer.showControls=false;PM.invalidate();await frame();const hidden=viewer.nullMarkers().length===0;
    viewer.showControls=true;PM.invalidate();await frame();
    const saved=PM.serialize();PM.replaceProject(PM.hydrateProject(PM.deserialize(saved)));PM.selectLayers([]);PM.invalidate();await frame();
    return {out,unselected,visible,hidden,anchorHidden,reopened:viewer.nullMarkers().some((m:any)=>m.id===id),renderUnchanged:before===PM.renderFrameTo(0,320,180,{mblur:false}).toDataURL()};
  });
  for(const item of result.out){
    expect(item.marker,`button ${item.index}`).toBeDefined();expect(Math.abs(item.marker.x-item.expected.x)).toBeLessThan(.1);expect(Math.abs(item.marker.y-item.expected.y)).toBeLessThan(.1);
    expect(item.selected).toBe(true);expect(item.covers).toBe(true);expect(item.hover,`gizmo at button ${item.index} pivot`).toBe(true);
  }
  expect(result.anchorHidden).toBe(true);expect(result.unselected.selected).toBe(false);expect(result.visible).toBe(true);expect(result.hidden).toBe(true);expect(result.reopened).toBe(true);expect(result.renderUnchanged).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('2D identity nulls display their controls at the chosen pivot, including deselection and clicking',async({session})=>{
  await session.openEditor();
  const state=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('flat-null-proof'),viewer=PM.Kernel.services.get('viewer');PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({name:'Flat null placement',w:640,h:360,dur:3}));
    PM.Edit.apply({type:'add_layer',id:'shape',layerType:'shape',content:{shape:'rect',w:100,h:60},properties:{'position.x':420,'position.y':230}});
    PM.L('shape').from=.75;PM.L('shape').dur=1.25;PM.setTime(1.1,{force:true});PM.touch();
    const r=api.groups.createBoundsNull(['shape'],8);if(!r.ok)throw Error(r.message);const id=r.data.result.nullIds[0];
    const node=PM.L(id),frame=()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))),outlines=[];
    const alpha=(point:any)=>{
      const ratio=viewer.ov.width/viewer.stage.clientWidth,ctx=viewer.ov.getContext('2d');
      const pixels=ctx.getImageData(Math.round(point.x*ratio)-2,Math.round(point.y*ratio)-2,5,5).data;
      return Math.max(...[...pixels].filter((_:any,i:number)=>i%4===3));
    };
    for(const zoom of [.5,1])for(const rotation of [0,35]){
      node.p.rotation.v=rotation;PM.touch();viewer.fit=false;viewer.zoom=zoom;viewer.layout();
      PM.selectLayers([id]);PM.invalidate();await frame();
      const selected=viewer.nullMarkers().find((m:any)=>m.id===id),a=selected.corners[0],b=selected.corners[1];
      const edge={x:a.x+(b.x-a.x)*.25,y:a.y+(b.y-a.y)*.25},selectedAlpha=alpha(edge);
      const before=PM.renderFrameTo(PM.time,320,180,{mblur:false}).toDataURL();
      PM.selectLayers([]);PM.invalidate();await frame();
      const unselected=viewer.nullMarkers().find((m:any)=>m.id===id);
      outlines.push({zoom,rotation,selected,unselected,selectedAlpha,unselectedAlpha:alpha(edge),renderUnchanged:before===PM.renderFrameTo(PM.time,320,180,{mblur:false}).toDataURL()});
    }
    node.p.rotation.v=0;PM.touch();
    const visibility=[];
    for(const time of [.5,.75,1.9,2,2.5]){
      PM.setTime(time,{force:true});PM.invalidate();await frame();visibility.push(viewer.nullMarkers().some((m:any)=>m.id===id));
    }
    PM.setTime(1.1,{force:true});
    PM.selectLayers([]);viewer.fit=true;viewer.layout();PM.invalidate();await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
    const marker=viewer.nullMarkers().find((m:any)=>m.id===id),bounds=viewer.worldBounds(PM.L(id),PM.time),rect=viewer.stage.getBoundingClientRect(),a=marker.corners[0],b=marker.corners[1];
    return {id,marker,bounds,outlines,visibility,timing:{from:node.from,dur:node.dur},x:rect.left+a.x+(b.x-a.x)*.25,y:rect.top+a.y+(b.y-a.y)*.25};
  });
  expect(state.timing).toEqual({from:.75,dur:1.25});expect(state.visibility).toEqual([false,true,true,false,false]);
  for(const outline of state.outlines){
    expect(outline.unselected.corners).toEqual(outline.selected.corners);
    expect(outline.selectedAlpha).toBeGreaterThan(0);expect(outline.unselectedAlpha).toBeGreaterThan(0);expect(outline.unselectedAlpha/outline.selectedAlpha).toBeGreaterThan(.25);expect(outline.unselectedAlpha/outline.selectedAlpha).toBeLessThan(.45);expect(outline.renderUnchanged).toBe(true);
  }
  expect((state.bounds.x0+state.bounds.x1)/2).toBeCloseTo(470);expect((state.bounds.y0+state.bounds.y1)/2).toBeCloseTo(260);
  await checkNullHover(session.page,state.id,state.x,state.y);
  await session.page.mouse.click(state.x,state.y);
  expect(await session.page.evaluate(()=>(window as any).PM.sel.layers)).toEqual([state.id]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for(const kind of ['model','extruded','plane'])test(`${kind} nulls retain their full projected outline and 3D gizmo after deselection and orbit`,async({session})=>{
  await session.openEditor();const page=session.page;
  const setup=await page.evaluate(async(kind)=>{
    const PM=(window as any).PM,api=PM.Kernel.api('null-camera-proof'),viewer=PM.Kernel.services.get('viewer');PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({name:'Null camera outline',w:640,h:360,dur:3}));
    const check=(r:any)=>{if(!r.ok)throw Error(r.message);return r.data;};
    check(api.scene3d.edit({operation:'add_camera',camera:{projection:'orthographic',p:{x:0,y:0,z:6,zoom:1}}}));
    if(kind==='model')check(api.scene3d.edit({operation:'add_object',object:{id:'source',source:{primitive:'box'},p:{x:1,y:.5}}}));
    else {
      check(PM.Edit.apply({type:'add_layer',id:'source',layerType:'shape',content:{shape:'rect',w:100,h:60},properties:{'position.x':380,'position.y':150}}));
      if(kind==='extruded')check(api.scene3d.setExtrusion('source',{enabled:true,depth:20}));
      else {check(PM.Edit.apply({type:'set_layer',target:'source',patch:{threeD:true}}));check(api.scene3d.edit({operation:'set_material_options',target:'source',patch:{acceptsLights:true}}));}
    }
    const id=check(api.groups.createBoundsNull(['source'],4)).result.nullIds[0];viewer.fit=true;viewer.layout();PM.invalidate();
    await new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r())));
    const selected=viewer.nullMarkers().find((m:any)=>m.id===id),before=PM.renderFrameTo(0,320,180,{mblur:false}).toDataURL();
    PM.selectLayers([]);PM.invalidate();await new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r())));
    const unselected=viewer.nullMarkers().find((m:any)=>m.id===id),ctx=viewer.ov.getContext('2d'),a=unselected.corners[0],b=unselected.corners[1],ratio=viewer.ov.width/viewer.stage.clientWidth;
    const edge={x:a.x+(b.x-a.x)*.25,y:a.y+(b.y-a.y)*.25},px=ctx.getImageData(Math.round(edge.x*ratio)-2,Math.round(edge.y*ratio)-2,5,5).data;
    const stage=viewer.stage.getBoundingClientRect();
    (window as any).__nullCameraAPI=api.scene3d;
    return {id,selected,unselected,painted:[...px].some((v:any,i:number)=>i%4===3&&v>0),renderUnchanged:before===PM.renderFrameTo(0,320,180,{mblur:false}).toDataURL(),covers:viewer.sceneGizmo.covers([id]),edge:{x:stage.left+edge.x,y:stage.top+edge.y},orbit:{x:stage.left+stage.width*.45,y:stage.top+stage.height*.55}};
  },kind);
  expect(setup.covers).toBe(true);expect(setup.selected.corners).toHaveLength(4);expect(setup.unselected.corners).toEqual(setup.selected.corners);expect(setup.painted).toBe(true);expect(setup.renderUnchanged).toBe(true);
  await checkNullHover(page,setup.id,setup.edge.x,setup.edge.y);
  await page.mouse.click(setup.edge.x,setup.edge.y);expect(await page.evaluate(()=>(window as any).PM.sel.layers)).toEqual([setup.id]);
  await page.evaluate(()=>{const api=(window as any).__nullCameraAPI;api.setView('editor');api.setNavigationMode('orbit');});
  await page.mouse.move(setup.orbit.x,setup.orbit.y);await page.mouse.down();await page.mouse.move(setup.orbit.x+55,setup.orbit.y-25,{steps:5});await page.mouse.up();
  const orbited=await page.evaluate(async(id)=>{const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer');(window as any).__nullCameraAPI.setNavigationMode('select');PM.selectLayers([]);PM.invalidate();await new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r())));return viewer.nullMarkers().find((m:any)=>m.id===id);},setup.id);
  expect(orbited.corners).toHaveLength(4);expect(orbited.corners).not.toEqual(setup.unselected.corners);expect(orbited.selected).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
