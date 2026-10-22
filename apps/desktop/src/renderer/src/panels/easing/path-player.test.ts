// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { makePM } from '../../legacy/__tests__/make-pm';
import { createEngine } from '../../player/player';
import { buildWebExport } from '../../player/export-web';
import { cleanupPath, positionController } from './position';
function fixture(controller=false){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({fps:30,dur:4,w:400,h:400});
  const layer=PM.mkLayer('solid',{from:0,dur:4});PM.addLayer(layer,0);
  PM.animate(layer,'position.x',[{t:0,v:0},{t:2,v:200}]);PM.animate(layer,'position.y',[{t:0,v:0},{t:2,v:200}]);
  PM.Edit.apply({type:'set_easing',keyframes:layer.p['position.y'].kf.map((k:any)=>k.i),curve:[1/3,0,2/3,1/3]});
  PM.sel.keys=layer.p['position.x'].kf.map((k:any)=>k.i);
  expect(PM.Edit.apply(controller?positionController(PM):cleanupPath(PM)).ok).toBe(true);
  return {PM,id:layer.id};
}
for(const controller of [false,true])describe(controller?'path controller portability':'cleaned path portability',()=>{
  it('renders identical source curves in the independent web player after JSON serialization',async()=>{
    const {PM,id}=fixture(controller), exported=createEngine(JSON.parse(JSON.stringify(PM.proj)));
    exported.MediaStore={get:async()=>null};
    const result=await buildWebExport(exported);
    const player=createEngine(JSON.parse(JSON.stringify(result.scene.project)));
    const layer=player.proj.layers.find((l:any)=>l.id===id);
    for(let i=0;i<=60;i++){
      const time=i/30,x=player.ev(layer,'position.x',time),y=player.ev(layer,'position.y',time);
      expect(x).toBeCloseTo(PM.ev(PM.L(id),'position.x',time),9);expect(y).toBeCloseTo(x*x/200,7);
      expect(player.expressionErrors.get(layer.p['position.x'])).toBeUndefined();
    }
    expect(new TextDecoder().decode(result.files.get('player.js'))).toContain('motionPath');
  });
});
