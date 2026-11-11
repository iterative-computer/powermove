import { describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { makePM } from '../../legacy/__tests__/make-pm';
import { world3D } from '../../legacy/core/space-3d';
import { LAYER3D_DEFINITIONS, layer3DAssetIds } from './layers';
import { editScene } from './operations';
import { compositionRuntime } from './service';
import { createLight } from './schema';
import { gizmoModes } from './gizmo';
import { lightViewerState } from './light-view';

function editor() {
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({fps:30,dur:5});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  const texture=new THREE.Texture();PM.proj.assets.hdri={id:'hdri',kind:'image',format:'exr',name:'Studio.exr'};PM.assets={get:()=>({environmentTexture:texture})};
  return {PM,texture};
}

describe('Environment Light layers',()=>{
  it('creates a standalone light with studio lighting and no camera',()=>{
    const {PM}=editor();expect(createLight('env','environment').p.intensity.v).toBe(100);
    expect(editScene(PM,{operation:'add_light',light:{id:'env',type:'environment'},name:'Environment Light'}).ok).toBe(true);
    expect(PM.proj.layers).toHaveLength(1);expect(PM.L('env').d.data.light.type).toBe('environment');
    const runtime=compositionRuntime(PM);expect(runtime.scene.environment).not.toBeNull();expect(runtime.environmentLightId).toBe('env');
    expect(runtime.scene.environmentIntensity).toBe(1);expect((runtime.objects.get('env') as THREE.Light).isLight).not.toBe(true);
    expect(lightViewerState(PM,'env',0,{x:0,y:0,width:640,height:360})).toBeNull();expect(gizmoModes('light','environment')).toEqual(['rotate']);
    PM.hist.undo();expect(PM.proj.layers).toHaveLength(0);PM.hist.redo();expect(PM.L('env').d.data.light.type).toBe('environment');
  });
  it('owns its source, intensity and native XYZ rotation, including parent transforms',()=>{
    const {PM,texture}=editor();editScene(PM,{operation:'add_camera',id:'camera'});const camera=JSON.stringify(PM.L('camera').d);
    expect(editScene(PM,{operation:'add_light',light:{id:'env',type:'environment',map:'hdri',p:{intensity:150}}}).ok).toBe(true);
    expect(layer3DAssetIds(PM.L('env'))).toEqual(['hdri']);
    PM.Edit.apply({type:'add_layer',id:'rig',layerType:'null',threeD:true});PM.L('env').parent='rig';
    for(const [id,path,value] of [['rig','rotation.y',20],['env','rotation.x',30],['env','rotation.y',60],['env','rotation',10]] as const)expect(PM.Edit.apply({type:'set_property',target:id,path,value}).ok).toBe(true);
    const runtime=compositionRuntime(PM);expect(runtime.scene.environment).toBe(texture);expect(runtime.scene.environmentIntensity).toBe(1.5);
    const expected=new THREE.Matrix4().extractRotation(new THREE.Matrix4().fromArray(world3D(PM,PM.L('env'),0)));
    const actual=new THREE.Matrix4().makeRotationFromEuler(runtime.scene.environmentRotation);
    actual.elements.forEach((v,i)=>expect(v).toBeCloseTo(expected.elements[i]!,6));expect(JSON.stringify(PM.L('camera').d)).toBe(camera);
    expect(editScene(PM,{operation:'update_light',target:'env',patch:{map:'missing'}}).ok).toBe(false);
    PM.L('env').on=false;PM.touch();expect(compositionRuntime(PM).scene.environment).toBeNull();
    PM.L('env').on=true;PM.L('env').from=2;PM.touch();expect(compositionRuntime(PM,1).scene.environment).toBeNull();expect(compositionRuntime(PM,3).scene.environment).toBe(texture);
  });
  it('uses one active environment at a time and animates intensity normally',()=>{
    const {PM,texture}=editor();editScene(PM,{operation:'add_light',light:{id:'env',type:'environment',map:'hdri'}});
    expect(editScene(PM,{operation:'update_light',target:'env',patch:{p:{intensity:{v:100,kf:[{t:0,v:100},{t:1,v:200}],expr:null}}}}).ok).toBe(true);
    expect(compositionRuntime(PM,.5).scene.environmentIntensity).toBeCloseTo(1.5);
    editScene(PM,{operation:'add_light',light:{id:'top',type:'environment',p:{intensity:50}}});PM.proj.layers.sort((a:any,b:any)=>a.id==='top'?-1:b.id==='top'?1:0);PM.touch();
    expect(compositionRuntime(PM,.5).environmentLightId).toBe('top');expect(compositionRuntime(PM,.5).scene.environmentIntensity).toBe(.5);
    PM.L('top').on=false;PM.touch();expect(compositionRuntime(PM,.5).scene.environment).toBe(texture);
    PM.L('env').lock=true;expect(editScene(PM,{operation:'update_light',target:'env',patch:{map:null}}).ok).toBe(false);
  });
});
