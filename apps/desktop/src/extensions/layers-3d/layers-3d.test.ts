import {describe,expect,it,vi} from 'vitest';
import activate from './index';
import {addPrimitive,addLight,addCamera,importModelIntoScene,duplicateItems,removeItems} from './scene-actions';
import {createObject,createLight,createScene} from '../../renderer/src/core/scene3d/schema';
import {MODEL_RECIPES} from '../../renderer/src/core/scene3d/modeling';
import {MATERIAL_FIELDS,lightFields,cameraFields} from './scene-model';
import {tabsFor} from './property-tabs';
function harness(){
  const definitions:any[]=[],commands:any[]=[],sections:any[]=[],menus:any[]=[],layers:any[]=[];
  let selected:string[]=[];
  const api:any={
    scene3d:{modelRecipes:MODEL_RECIPES,createObject,createLight,createScene,prepareImport:vi.fn(async(files:File[])=>files[0]),convert:vi.fn(),edit:vi.fn(()=>({ok:true,message:'',data:{result:{id:'created'}}}))},
    layers:{register:(d:any)=>definitions.push(d)},commands:{register:(c:any)=>commands.push(c),run:vi.fn()},
    inspector:{registerSection:(s:any)=>sections.push(s)},palette:{registerProvider:vi.fn()},menus:{contribute:(location:any,items:any)=>menus.push({location,items})},
    project:{apply:vi.fn(),get:()=>({assets:{}})},model:{layer:(id:string)=>layers.find(l=>l.id===id),curComp:()=>({layers})},
    selection:{layers:()=>selected,select:vi.fn((ids:string[])=>{selected=ids;})},
    assets:{pick:vi.fn(),import:vi.fn(async()=>({id:'asset'}))},transport:{invalidate:vi.fn()},ui:{toast:vi.fn(),menu:vi.fn()},
    edit:{begin:vi.fn(),commit:vi.fn(()=>({ok:true})),cancel:vi.fn()}
  };activate(api);return {api,definitions,commands,sections,menus,layers};
}
describe('3D layer UI and commands',()=>{
  it('registers individual models, lights and cameras with native inspectors and add menus',()=>{
    const {definitions,commands,sections,menus,layers}=harness();
    for(const role of ['object','light','camera'])expect(definitions).toContainEqual(expect.objectContaining({id:`powermove.3d.${role}`,renderer:{kind:'layer3d',role}}));
    expect(commands.some(c=>c.label==='New 3D Scene')).toBe(false);
    expect(menus.map(m=>m.location)).toEqual(['timeline:context','viewer:context']);
    layers.push({id:'model',type:'extension',d:{definition:'powermove.3d.object'}});
    expect(sections[0].when({layerIds:['model']})).toBe(true);expect(sections[0].when({layerIds:['video']})).toBe(false);
  });
  it('creates each primitive, light and camera directly without making a container',()=>{
    const {api}=harness();addPrimitive(api,'torus');addLight(api,'spot');addCamera(api);
    expect(api.scene3d.edit.mock.calls.map((c:any)=>c[0].operation)).toEqual(['add_object','add_light','add_camera']);
    expect(api.scene3d.edit.mock.calls[0][0]).toMatchObject({object:{source:{primitive:'torus'}}});
    expect(api.selection.select).toHaveBeenCalledWith(['created']);
  });
  it('packs OBJ/MTL/textures and imports exactly one model layer',async()=>{
    const {api}=harness(),obj=new File(['v 0 0 0'],'model.obj'),mtl=new File(['newmtl paint'],'model.mtl'),packed=new File(['packed'],'model.glb');
    api.scene3d.prepareImport.mockResolvedValue(packed);
    const result=await importModelIntoScene(api,[obj,mtl]);expect(result.ok).toBe(true);
    expect(api.scene3d.prepareImport).toHaveBeenCalledWith([obj,mtl]);
    expect(api.assets.import).toHaveBeenCalledWith(packed,{layerDefinition:'powermove.3d.object'});
    expect(api.scene3d.edit).toHaveBeenCalledTimes(1);expect(api.scene3d.edit.mock.calls[0][0]).toMatchObject({operation:'add_object',object:{source:{assetId:'asset'},useSourceMaterials:true}});
  });
  it('cancels a failed multi-layer operation without changing selection',()=>{
    const {api,layers}=harness();layers.push({id:'a',d:{definition:'powermove.3d.object'}},{id:'b',d:{definition:'powermove.3d.light'}});
    api.selection.select(['a','b']);api.selection.select.mockClear();
    api.scene3d.edit.mockReturnValueOnce({ok:true,message:'',data:{result:{id:'copy'}}}).mockReturnValueOnce({ok:false,message:'Locked'});
    expect(duplicateItems(api)).toBe(false);expect(api.edit.cancel).toHaveBeenCalledOnce();expect(api.edit.commit).not.toHaveBeenCalled();expect(api.selection.select).not.toHaveBeenCalled();
  });
  it('deletes real camera layers in the same transaction as models',()=>{
    const {api,layers}=harness();layers.push({id:'camera',d:{definition:'powermove.3d.camera'}},{id:'model',d:{definition:'powermove.3d.object'}});
    api.selection.select(['camera','model']);expect(removeItems(api)).toBe(true);
    expect(api.scene3d.edit.mock.calls.map((c:any)=>c[0].target)).toEqual(['camera','model']);expect(api.edit.commit).toHaveBeenCalledOnce();expect(api.selection.select).toHaveBeenLastCalledWith([]);
  });
  it('shows material, light and camera fields for the selected layer role',()=>{
    expect(MATERIAL_FIELDS.map(f=>f.key)).toContain('roughness');expect(MATERIAL_FIELDS.map(f=>f.key)).toContain('emissive');
    expect(lightFields('spot').map(f=>f.key)).toContain('angle');expect(lightFields('point').map(f=>f.key)).not.toContain('angle');
    expect(cameraFields('perspective').map(f=>f.key)).toContain('fov');expect(cameraFields('orthographic').map(f=>f.key)).toContain('zoom');
  });
  it('offers Blender Properties tabs that fit each kind of 3D layer',()=>{
    expect(tabsFor({object:true,light:false,camera:false,model:false})).toEqual(['render','world','object','material']);
    expect(tabsFor({object:true,light:false,camera:false,model:true})).toEqual(['render','world','object','modifiers','material']);
    expect(tabsFor({object:false,light:false,camera:false,model:true})).toEqual(['render','world','modifiers']);
    expect(tabsFor({object:false,light:true,camera:false,model:false})).toEqual(['render','world','data']);
    expect(tabsFor({object:false,light:false,camera:true,model:false})).toEqual(['render','world','data']);
  });
});
