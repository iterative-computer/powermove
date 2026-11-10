import {test,expect} from './helpers/app';

test('child spatial effects keep their local coordinates inside a transformed group material',async({session})=>{
  await session.openEditor();
  const result=await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({w:320,h:180,dur:2}));
    const group=PM.mkLayer('group',{from:0,dur:2});group.id='art';group.on=false;
    group.p['position.x'].v=1200;group.p['position.y'].v=-400;group.p.rotation.v=27;
    const paper=PM.mkLayer('shape',{from:0,dur:2,d:{shape:'rect',w:100,h:60,color:'#FFFFFF',stroke:0}});
    paper.group=group.id;paper.p['position.x'].v=50;paper.p['position.y'].v=30;
    const effect=PM.mkEffect('gradient',undefined,paper);
    effect.p.startX.v=0;effect.p.startY.v=0;effect.p.endX.v=100;effect.p.endY.v=0;effect.p.rampScatter.v=0;
    effect.p.startColor.v='#FF0000';effect.p.endColor.v='#0000FF';paper.fx=[effect];
    PM.proj.layers=[paper,group];PM.touch();
    const pixels=PM.GL.renderMaterialComposition(PM.proj,.5,200,120,.5,group.id);
    const sample=(x:number)=>[...pixels.slice((60*200+x)*4,(60*200+x)*4+4)];
    return {left:sample(50),right:sample(150),errors:[...PM.GL.errors]};
  });
  expect(result.left[0]).toBeGreaterThan(result.left[2]);
  expect(result.right[2]).toBeGreaterThan(result.right[0]);
  expect(result.left[3]).toBe(255);expect(result.right[3]).toBe(255);expect(result.errors).toEqual([]);
});

test('group materials composite nested layers locally, preserve alpha, effects and animation, and survive reopening',async({session})=>{
  await session.openEditor();
  const result=await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();
    PM.replaceProject(PM.mkProject({w:320,h:180,fps:30,dur:3}));
    const make=(id:string,type:string,d:any={},x=0,y=0)=>{const layer=PM.mkLayer(type,{name:id,d});layer.id=id;layer.p['position.x'].v=x;layer.p['position.y'].v=y;PM.proj.layers.push(layer);return layer;};
    const group=make('artwork','group',{},1200,-400),nested=make('nested','group',{},10,10);
    group.on=false;group.p.rotation.v=27;group.p['scale.x'].v=group.p['scale.y'].v=200;nested.group=group.id;
    const paper=make('paper','shape',{shape:'rect',w:100,h:60,color:'#FFFFFF',radius:0,stroke:0},50,30);paper.group=group.id;
    const red=make('red','shape',{shape:'rect',w:30,h:20,color:'#FF0000',radius:0,stroke:0},15,10);red.group=nested.id;red.p.opacity.v=50;
    const blue=make('blue','shape',{shape:'rect',w:20,h:20,color:'#0000FF',radius:0,stroke:0},60,10);blue.group=group.id;blue.from=1;blue.dur=1;
    const hidden=make('hidden','shape',{shape:'rect',w:100,h:60,color:'#00FF00',radius:0},50,30);hidden.group=group.id;hidden.on=false;
    PM.proj.layers=[hidden,blue,red,nested,paper,group];PM.touch();
    const capture=(time:number)=>PM.GL.renderMaterialComposition(PM.proj,time,200,120,time,'artwork');
    const sample=(pixels:Uint8Array,x:number,y:number)=>[...pixels.slice((y*200+x)*4,(y*200+x)*4+4)];
    const initial=capture(.5),later=capture(1.5);
    const mask=PM.mkMask('rect',PM.proj);mask.p.x.v=50;mask.p.y.v=30;mask.p.w.v=60;mask.p.h.v=60;mask.p.feather.v=0;group.masks=[mask];PM.touch();
    const masked=capture(.5);group.fx=[PM.mkEffect('invert',undefined,group)];PM.touch();const effected=capture(.5);
    const serial=PM.serialize();PM.replaceProject(PM.hydrateProject(PM.deserialize(serial)));const reopened=capture(.5);
    return {paper:sample(initial,90,90),red:sample(initial,40,40),before:sample(initial,120,20),later:sample(later,120,20),maskedOutside:sample(masked,10,60),maskedInside:sample(masked,90,90),effect:sample(effected,90,90),reopened:[...reopened].every((v:number,i:number)=>v===effected[i]),errors:[...PM.GL.errors.entries()]};
  });
  expect(result.paper).toEqual([255,255,255,255]);expect(result.red[0]).toBe(255);expect(result.red[1]).toBeGreaterThanOrEqual(126);expect(result.red[1]).toBeLessThanOrEqual(129);
  expect(result.before).toEqual([255,255,255,255]);expect(result.later).toEqual([0,0,255,255]);
  expect(result.maskedOutside[3]).toBe(0);expect(result.maskedInside).toEqual([255,255,255,255]);expect(result.effect).toEqual([0,0,0,255]);
  expect(result.reopened).toBe(true);expect(result.errors).toEqual([]);expect(session.diagnostics.pageErrors).toEqual([]);
});
