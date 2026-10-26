import { expect, it } from 'vitest';
import { makePM } from './make-pm';
import { EFFECTS } from '../../../../extensions/effects-basic/effects';
import { sanitizeProject } from '../../core/validate/project';
import { copyEffects, effectPasteCommands } from '../../../../extensions/inspector/effect-clipboard';

function editor() {
 const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing','gl/shaders');
 for(const effect of EFFECTS)PM.Kernel.registerEffect('effects-basic',effect);
 PM.proj=PM.mkProject({w:64,h:64,dur:2});PM.time=0;
 PM.Kernel.services.register('shaderHooks',{syncShaderUniforms(){}});
 const owner=PM.mkLayer('shape',{name:'Owner'}),map=PM.mkLayer('shape',{name:'Map'});PM.proj.layers=[owner,map];PM.ProjectIndex.invalidate();
 return{PM,owner,map};
}

it('creates static layer inputs through typed commands, excludes them from channels, and preserves effect clipboard references',()=>{
 const{PM,owner,map}=editor();
 const added=PM.Edit.apply({type:'add_effect',target:owner.id,effect:'setMatte',inputs:{map:{layerId:map.id,stage:'masks'}}},{origin:'inspector'});
 expect(added.ok).toBe(true);const f=owner.fx[0];expect(f.p.map).toBeUndefined();expect(f.inputs.map).toEqual({layerId:map.id,stage:'masks'});
 expect(PM.allProps(owner).some((p:any)=>p.key===`${f.id}.map`)).toBe(false);
 expect(PM.Edit.apply({type:'set_property',target:owner.id,path:`${f.id}.map`,value:map.id,mode:'keyframe'},{origin:'inspector'}).ok).toBe(false);
 copyEffects([f]);const pasted=effectPasteCommands(map.id)[0]!;expect(pasted.inputs).toEqual(f.inputs);expect(pasted.parameters).not.toHaveProperty('map');
});

it('rolls back effect/matte/group cycles and malformed stages atomically',()=>{
 const{PM,owner,map}=editor();owner.fx=[PM.mkEffect('setMatte',undefined,owner)];owner.fx[0].inputs.map={layerId:map.id,stage:'effects'};
 const before=JSON.stringify(PM.proj);
 const result=PM.Edit.apply({type:'set_layer',target:map.id,patch:{matteSource:owner.id}},{origin:'inspector'});
 expect(result.ok).toBe(false);expect(result.message).toContain('cycle');expect(JSON.stringify(PM.proj)).toBe(before);
 const malformed=PM.Edit.apply({type:'set_effect',target:owner.id,effect:owner.fx[0].id,patch:{inputs:{map:{layerId:map.id,stage:'future'}}}},{origin:'inspector'});
 expect(malformed.ok).toBe(false);expect(JSON.stringify(PM.proj)).toBe(before);
});

it('typed serialization preserves missing and future reference records without introducing animated channels',()=>{
 const{PM,owner}=editor();const f=PM.mkEffect('setMatte',undefined,owner);f.version=999;f.inputs.map={layerId:'deleted-id',stage:'future-stage',future:{mode:2}};owner.fx=[f];
 const loaded=sanitizeProject(JSON.parse(JSON.stringify(PM.proj)));expect(loaded.layers[0]!.fx[0]!.inputs).toEqual(f.inputs);expect(loaded.layers[0]!.fx[0]!.version).toBe(999);
});


it('edits coordinate space atomically and preserves it through save, copy and duplication',()=>{
 const{PM,owner,map}=editor();const f=PM.mkEffect('setMatte',undefined,owner);owner.fx=[f];
 const changed=PM.Edit.apply({type:'set_effect',target:owner.id,effect:f.id,patch:{inputs:{map:{layerId:map.id,stage:'source',space:'source'}}}},{origin:'inspector'});
 expect(changed.ok).toBe(true);expect(f.inputs.map.space).toBe('source');
 const saved=JSON.parse(JSON.stringify(PM.proj));expect(sanitizeProject(saved).layers[0]!.fx[0]!.inputs!.map!.space).toBe('source');
 copyEffects([f]);expect(effectPasteCommands(map.id)[0]!.inputs!.map!.space).toBe('source');
 const before=JSON.stringify(PM.proj);expect(PM.Edit.apply({type:'set_effect',target:owner.id,effect:f.id,patch:{inputs:{map:{layerId:map.id,stage:'source',space:'invalid'}}}},{origin:'inspector'}).ok).toBe(false);expect(JSON.stringify(PM.proj)).toBe(before);
});


it('preserves explicit source owner mode and typed references through history, clipboard and hydration',()=>{
 const{PM,owner}=editor();const f=PM.mkEffect('texturize',undefined,owner);owner.fx=[f];
 expect(PM.Edit.apply({type:'set_layer',target:owner.id,patch:{effectSpace:'source'}},{origin:'inspector'}).ok).toBe(true);
 expect(owner.effectSpace).toBe('source');expect(PM.Edit.apply({type:'set_layer',target:owner.id,patch:{effectSpace:'invalid'}},{origin:'inspector'}).ok).toBe(false);
 expect(PM.Edit.apply({type:'set_effect',target:owner.id,effect:f.id,patch:{inputs:{texture:{kind:'asset',assetId:'saved-image'}}}},{origin:'inspector'}).ok).toBe(true);
 const current=PM.L(owner.id).fx[0];copyEffects([current]);expect(effectPasteCommands(owner.id)[0]!.inputs!.texture).toEqual({kind:'asset',assetId:'saved-image'});
 const restored=sanitizeProject(JSON.parse(JSON.stringify(PM.proj)));expect(restored.layers[0]!.effectSpace).toBe('source');expect(restored.layers[0]!.fx[0]!.inputs!.texture).toEqual(current.inputs.texture);
});

it('LUT references remain static through typed editing, clipboard, save and duplicate history',()=>{
 const{PM,owner}=editor();const result=PM.Edit.apply({type:'add_effect',target:owner.id,effect:'applyColorLut',inputs:{lut:{kind:'asset',assetId:'grade'}}},{origin:'inspector'});expect(result.ok).toBe(true);const current=owner.fx[0];expect(current.p.lut).toBeUndefined();copyEffects([current]);expect(effectPasteCommands(owner.id)[0]!.inputs!.lut).toEqual(current.inputs.lut);expect(sanitizeProject(JSON.parse(JSON.stringify(PM.proj))).layers[0]!.fx[0]!.inputs!.lut).toEqual(current.inputs.lut);
});

it.each(['blend','calculations','channelCombiner'])('%s has static versioned layer refs and editable native parameters, without changing historical definitions',type=>{
 const{PM,owner,map}=editor(),result=PM.Edit.apply({type:'add_effect',target:owner.id,effect:type,inputs:{map:{layerId:map.id,stage:'effects',space:'source'}}},{origin:'inspector'});expect(result.ok).toBe(true);const f=owner.fx[0];expect(f.version).toBe(type==='channelCombiner'?2:1);expect(f.p.map).toBeUndefined();expect(PM.allProps(owner).some((p:any)=>p.key===`${f.id}.map`)).toBe(false);copyEffects([f]);expect(effectPasteCommands(map.id)[0]!.inputs).toEqual(f.inputs);expect(sanitizeProject(JSON.parse(JSON.stringify(PM.proj))).layers[0]!.fx[0]!.inputs).toEqual(f.inputs);
 const before=JSON.stringify(PM.proj);expect(PM.Edit.apply({type:'set_effect',target:owner.id,effect:f.id,patch:{inputs:{map:{layerId:owner.id,stage:'effects'}}}},{origin:'inspector'}).ok).toBe(false);expect(JSON.stringify(PM.proj)).toBe(before);
});
