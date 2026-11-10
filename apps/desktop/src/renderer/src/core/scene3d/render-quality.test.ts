import { expect, it } from 'vitest';
import { makePM } from '../../legacy/__tests__/make-pm';
import { editScene } from './operations';
import { resolveStudioQuality } from '../../../../shared/studio-quality';
function editor() {
  const PM = makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj = PM.mkProject({name:'Quality'}); PM.time = 0; return PM;
}
it('saves per composition and round-trips undo/redo without creating a camera', () => {
  const PM=editor(),first=PM.proj.compId;
  const before=PM.serialize();
  expect(editScene(PM,{operation:'set_render_quality',preset:'draft'}).ok).toBe(true);
  expect(PM.proj.studioQuality).toEqual(resolveStudioQuality('draft'));
  expect(PM.proj.layers).toHaveLength(0);
  expect(PM.hist.undo()).toBe(true);expect(PM.serialize()).toBe(before);
  expect(PM.hist.redo()).toBe(true);expect(PM.proj.studioQuality.preset).toBe('draft');
  const second=PM.Comps.create({name:'Second'});expect(PM.proj.studioQuality.preset).toBe('production');
  editScene(PM,{operation:'set_render_quality',preset:'preview',patch:{maxSamples:48}});
  PM.Comps.open(first);expect(PM.proj.studioQuality.preset).toBe('draft');
  PM.Comps.open(second);expect(PM.proj.studioQuality).toMatchObject({preset:'custom',maxSamples:48});
  PM.proj=PM.deserialize(PM.serialize());expect(PM.proj.studioQuality.maxSamples).toBe(48);
});
it('rejects invalid settings and stale agent revisions atomically', () => {
  const PM=editor(),before=PM.serialize();
  for(const patch of [{minSamples:128,maxSamples:16},{maxBounces:0},{denoise:'yes'},{__bad:1}]) {
    expect(editScene(PM,{operation:'set_render_quality',patch}).ok).toBe(false);
    expect(PM.serialize()).toBe(before);
  }
  expect(editScene(PM,{operation:'set_render_quality',preset:'draft'},{origin:'agent',baseRevision:PM.proj.revision+1}).ok).toBe(false);
  expect(PM.serialize()).toBe(before);
});

it('carries an explicit legacy camera sample choice into composition quality',()=>{
  const PM=editor();PM.layerDefinition=()=>({params:[],renderer:{kind:'layer3d'}});
  const result=editScene(PM,{operation:'set_environment',patch:{samples:16}});
  expect(result.ok).toBe(true);expect(PM.proj.studioQuality).toMatchObject({preset:'custom',maxSamples:16,minSamples:16});
});

it('merges a direct guarded quality patch with the active composition',()=>{
 const PM=editor();editScene(PM,{operation:'set_render_quality',preset:'draft'});
 expect(PM.Edit.apply({type:'set_composition',patch:{studioQuality:{maxSamples:12}}}).ok).toBe(true);
 expect(PM.proj.studioQuality).toMatchObject({preset:'custom',noiseThreshold:.12,minSamples:4,maxSamples:12});
});
