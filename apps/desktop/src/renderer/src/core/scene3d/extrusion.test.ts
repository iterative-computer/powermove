// @vitest-environment happy-dom
import { describe,it,expect,vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { create } from 'fontkit';
import * as THREE from 'three';
import { fontOutlines,shapeOutlines,layerExtrusionOutlines } from './extrusion-outlines';
import { buildExtrusionGeometry,resolveContours } from './extrusion-geometry';
import { EXTRUSION_DEFAULTS,isExtrudedLayer,parseExtrusion } from './extrusion-data';
import { installExtrusionFont, requestExtrusionFont, prepareExtrusionFonts } from './extrusion-fonts';
import { compositionRuntime,compositionDepthIds,disposeSceneRuntimes } from './service';
import { makePM } from '../../legacy/__tests__/make-pm';
import { makeScene3DAPI } from './api';
import { sceneProperties } from './schema';
import { materialLayerLinks,validateCompositionMaps } from './material-maps';
import { remapLayerInputs } from '../effect-inputs';
const bytes=readFileSync('assets/fonts/Inter-Variable.ttf');
const fontBytes=bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength);
const font=create(bytes) as import('fontkit').Font;
const options={...EXTRUSION_DEFAULTS,depth:40};
function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({w:640,h:360,fps:30,dur:4});PM.time=0;PM.assets.get=(id:string)=>PM.assets.map.get(id);
  const api=makeScene3DAPI(PM,vi.fn());
  for(const [id,type] of [['title','text'],['box','shape'],['paint','solid']])expect(PM.Edit.apply({type:'add_layer',id,layerType:type,content:type==='text'?{text:'B8%',font:'Inter',size:72,color:'#FF0000'}:{w:100,h:100,color:'#00FF00'},properties:{'position.x':320,'position.y':180}}).ok).toBe(true);
  installExtrusionFont(PM,PM.L('title').d,fontBytes);
  return {PM,api};
}
describe('native text and shape extrusion',()=>{
  it('keeps valid artwork when another layer has an unreadable font', async()=>{
    const {PM,api}=editor(),log=vi.spyOn(console,'warn').mockImplementation(()=>{});
    vi.stubGlobal('queryLocalFonts',async()=>[]);
    try {
      api.setExtrusion('box',{enabled:true,depth:30});
      PM.Edit.apply({type:'set_content',target:'title',patch:{font:'Missing font regression'}});
      api.setExtrusion('title',{enabled:true,depth:40});
      await expect(prepareExtrusionFonts(PM,PM.proj,0)).resolves.toBeUndefined();
      const runtime=compositionRuntime(PM);
      expect(runtime.objects.has('box')).toBe(true);expect(runtime.objects.has('title')).toBe(false);
      expect(log).toHaveBeenCalledWith(expect.stringContaining('"title"'));
    } finally {log.mockRestore();vi.unstubAllGlobals();disposeSceneRuntimes(PM);}
  });
  it('keeps the 3D switch off when editing saved extrusion settings until Extrude is explicitly enabled',()=>{
    const {PM,api}=editor();
    expect(api.setExtrusion('title',{enabled:true,depth:40}).ok).toBe(true);
    expect(PM.Edit.apply({type:'set_layer',target:'title',patch:{threeD:false}}).ok).toBe(true);
    expect(compositionRuntime(PM).objects.has('title')).toBe(false);
    expect(api.setExtrusion('title',{depth:80}).ok).toBe(true);
    expect(PM.L('title').threeD).toBe(false);
    expect(isExtrudedLayer(PM.L('title'))).toBe(false);
    expect(compositionRuntime(PM).objects.has('title')).toBe(false);
    expect(api.getExtrusion('title')!.values.depth).toBe(80);
    expect(api.setExtrusion('title',{enabled:true}).ok).toBe(true);
    expect(isExtrudedLayer(PM.L('title'))).toBe(true);
    PM.hist.undo();expect(PM.L('title').threeD).toBe(false);
    PM.hist.redo();expect(isExtrudedLayer(PM.L('title'))).toBe(true);
    disposeSceneRuntimes(PM);
  });
  it.each(['B','8','%'])('triangulates %s with intact counters and positive cap normals',(text)=>{
    const outlines=fontOutlines(font,{text,size:100,font:'Inter',weight:500});
    expect(outlines.reduce((n,o)=>n+o.holes.length,0)).toBe(2);
    const geometry=buildExtrusionGeometry(outlines,options),position=geometry.getAttribute('position'),normal=geometry.getAttribute('normal');
    const cap=geometry.groups[0]!;let capArea=0;
    for(let i=cap.start;i<cap.start+cap.count;i+=3){
      const a=new THREE.Vector3().fromBufferAttribute(position,i),b=new THREE.Vector3().fromBufferAttribute(position,i+1),c=new THREE.Vector3().fromBufferAttribute(position,i+2);
      const cross=b.clone().sub(a).cross(c.clone().sub(a));expect(cross.z).toBeGreaterThan(0);capArea+=cross.z/2;
      const center=a.add(b).add(c).divideScalar(3);
      for(const outline of outlines)for(const hole of outline.holes){
        let inside=false;for(let j=0,k=hole.length-1;j<hole.length;k=j++){const p=hole[j]!,q=hole[k]!;if((p[1]>-center.y)!==(q[1]>-center.y) && center.x<(q[0]-p[0])*(-center.y-p[1])/(q[1]-p[1])+p[0])inside=!inside;}
        expect(inside).toBe(false);
      }
    }
    const area=(ring:number[][])=>Math.abs(THREE.ShapeUtils.area(ring.map(p=>new THREE.Vector2(p[0],p[1]))));
    const expected=outlines.reduce((sum,o)=>sum+area(o.outer)-o.holes.reduce((s,h)=>s+area(h),0),0);
    expect(capArea).toBeCloseTo(expected,2);expect(normal.getZ(0)).toBe(1);geometry.dispose();
  });
  it.each(['none','angular','concave','convex'] as const)('has finite unit normals, UVs and four stable surface groups for %s',bevelStyle=>{
    const geometry=buildExtrusionGeometry(shapeOutlines({shape:'star',w:160,h:160,points:5}),{...options,bevelStyle,bevelDepth:4});
    expect(geometry.groups.map(g=>g.materialIndex)).toEqual([0,1,2,3]);
    const n=geometry.getAttribute('normal'),uv=geometry.getAttribute('uv'),p=geometry.getAttribute('position');
    for(let i=0;i<n.count;i++){expect(Math.hypot(n.getX(i),n.getY(i),n.getZ(i))).toBeCloseTo(1,5);expect(Number.isFinite(p.getX(i)+p.getY(i)+p.getZ(i))).toBe(true);expect(uv.getX(i)).toBeGreaterThanOrEqual(-1e-6);expect(uv.getX(i)).toBeLessThanOrEqual(1.000001);expect(uv.getY(i)).toBeGreaterThanOrEqual(-1e-6);expect(uv.getY(i)).toBeLessThanOrEqual(1.000001);}
    expect(geometry.boundingBox!.min.z).toBe(-40);expect(geometry.boundingBox!.max.z).toBe(0);geometry.dispose();
  });
  it('resolves self-intersections, nested compound holes and outlined open strokes',()=>{
    const bow=resolveContours([[[0,0],[100,100],[0,100],[100,0]]]);expect(bow).toHaveLength(2);
    const compound=resolveContours([[[0,0],[100,0],[100,100],[0,100]],[[20,20],[20,80],[80,80],[80,20]]]);expect(compound[0]!.holes).toHaveLength(1);
    const line=shapeOutlines({shape:'line',w:100,h:100,stroke:8});expect(line).toHaveLength(1);expect(buildExtrusionGeometry(line,options).getAttribute('position').count).toBeGreaterThan(20);
  });
  it('leaves old documents flat; validates new data and raw channel edits atomically',()=>{
    const {PM,api}=editor(),before=PM.serialize();expect(api.getExtrusion('title')).toMatchObject({enabled:false,values:EXTRUSION_DEFAULTS});expect(PM.serialize()).toBe(before);
    expect(isExtrudedLayer(PM.L('title'))).toBe(false);expect(api.setExtrusion('title',{enabled:true,depth:40,bevelStyle:'convex'}).ok).toBe(true);expect(isExtrudedLayer(PM.L('title'))).toBe(true);
    for(const command of [{type:'set_property',target:'title',path:'geo.depth',value:-1},{type:'replace_keyframes',target:'title',path:'geo.bevelStyle',keyframes:[{time:0,value:'invalid'}]},{type:'set_content',target:'title',patch:{extrusion:{enabled:true,p:{depth:Infinity}}}}]){const snapshot=PM.serialize();expect(PM.Edit.apply(command).ok).toBe(false);expect(PM.serialize()).toBe(snapshot);}
    expect(()=>parseExtrusion({enabled:true,unknown:true})).toThrow();
  });
  it('supports surface maps, animation, cycles, locking and material notifications',()=>{
    const {PM,api}=editor(),listener=vi.fn();api.onMaterialsChange(listener);expect(api.setExtrusion('title',{enabled:true,depth:40}).ok).toBe(true);
    expect(api.listSurfaces('title').map(s=>s.surface)).toEqual(['front','bevel','side','back']);expect(api.listMaterials('title').map(s=>s.name)).toEqual(['Front','Bevel','Side','Back']);
    expect(api.setSurfaceMaterialMap('title','front','color',{layerId:'paint'}).ok).toBe(true);expect(api.setSurfaceMaterialMap('title','side','color',{layerId:'title'}).ok).toBe(false);
    expect(api.setSurfaceMaterialMap('title','front','normal',{layerId:'paint'}).ok).toBe(false);
    PM.L('paint').fx=[{inputs:{source:{layerId:'title'}}}];expect(api.setSurfaceMaterialMap('title','side','color',{layerId:'paint'}).ok).toBe(false);PM.L('paint').fx=[];
    expect(api.setSurfaceMaterialProperties('title','side',{color:'#0000FF',opacity:{v:100,kf:[{t:0,v:100},{t:1,v:0}]},roughness:.2,metallic:.5,emissionColor:'#112233',emissionIntensity:2,ambientResponse:25}).ok).toBe(true);
    expect(api.getSurfaceMaterialProperties('title','side',1)!.values).toMatchObject({opacity:0,color:'#0000FF',roughness:.2,metallic:.5});
    expect(PM.findProp(PM.L('title'),'ms.side.opacity')).toBeTruthy();expect(sceneProperties(PM.L('title')).map(p=>p.key)).toContain('geo.depth');expect(listener).toHaveBeenCalled();
    PM.L('title').lock=true;expect(api.setExtrusion('title',{depth:20}).ok).toBe(false);PM.L('title').lock=false;
    PM.L('title').locked_intent={'geo.depth':{}};expect(api.setExtrusion('title',{depth:20},{origin:'agent'}).ok).toBe(false);
    expect(()=>validateCompositionMaps(PM.proj)).not.toThrow();
  });
  it('survives save/reopen, duplicate/remap, deletion undo/redo and clear',()=>{
    const {PM,api}=editor();api.setExtrusion('title',{enabled:true,depth:{v:40,kf:[{t:0,v:40},{t:1,v:80}]}});api.setSurfaceMaterialMap('title','front','color',{layerId:'paint'});
    PM.replaceProject(PM.deserialize(PM.serialize()));expect(api.getExtrusion('title',1)!.values.depth).toBe(80);expect(materialLayerLinks(PM,'title')[0]).toMatchObject({surface:'front',status:'ready'});
    const duplicate=PM.cloneLayer(PM.L('title'));expect(duplicate.d.extrusion.p.depth.kf.map((k:any)=>k.v)).toEqual([40,80]);expect(sceneProperties(duplicate)[0].prop.kf[0].i).not.toBe(sceneProperties(PM.L('title'))[0].prop.kf[0].i);
    remapLayerInputs(duplicate,new Map([['paint','copy-paint']]));expect(duplicate.d.extrusion.surfaces.front.maps.color).toEqual({layerId:'copy-paint'});
    expect(PM.Edit.apply({type:'delete_layers',targets:['paint']}).ok).toBe(true);expect(materialLayerLinks(PM,'title')[0]!.status).toBe('missing');PM.hist.undo();expect(materialLayerLinks(PM,'title')[0]!.status).toBe('ready');PM.hist.redo();expect(materialLayerLinks(PM,'title')[0]!.status).toBe('missing');
    expect(api.setSurfaceMaterialMap('title','front','color',null).ok).toBe(true);expect(materialLayerLinks(PM,'title')).toEqual([]);
  });
  it('interpolates surface colors and emission colors, and keeps geometry through undo/repeated frames',()=>{
    const {PM,api}=editor();api.setExtrusion('title',{enabled:true,depth:40});
    expect(api.setSurfaceMaterialProperties('title','front',{color:{v:'#FF0000',kf:[{t:0,v:'#FF0000'},{t:1,v:'#0000FF'}]},emissionColor:{v:'#FF0000',kf:[{t:0,v:'#FF0000'},{t:1,v:'#0000FF'}]}}).ok).toBe(true);
    const middle=api.getSurfaceMaterialProperties('title','front',.5)!.values;
    expect(middle.color.toUpperCase()).toBe('#800080');expect(middle.emissionColor.toUpperCase()).toBe('#800080');
    const world=compositionRuntime(PM),first=(world.objects.get('title') as THREE.Mesh).geometry;
    api.setExtrusion('title',{depth:80});compositionRuntime(PM);expect((world.objects.get('title') as THREE.Mesh).geometry).not.toBe(first);
    PM.hist.undo();compositionRuntime(PM);expect((world.objects.get('title') as THREE.Mesh).geometry).toBe(first);disposeSceneRuntimes(PM);
  });
  it('preserves extrusion and source links when precomposing artwork together',()=>{
    const {PM,api}=editor();api.setExtrusion('title',{enabled:true,depth:40});api.setSurfaceMaterialMap('title','front','color',{layerId:'paint'});
    const result=PM.Comps.precompose(['title','paint'],{name:'Extruded artwork',open:false});expect(result).toBeTruthy();
    const nested=Object.values<any>(PM.proj.comps).find(comp=>comp.name==='Extruded artwork');
    expect(nested.layers.find((l:any)=>l.id==='title').d.extrusion.p.depth.v).toBe(40);expect(materialLayerLinks(PM,'title')[0]!.status).toBe('ready');expect(()=>validateCompositionMaps(PM.proj)).not.toThrow();
  });
  it('loads variable font bytes once and changes animated weight without another asynchronous font load',async()=>{
    const PM={invalidate:vi.fn()},blob=vi.fn(async()=>new Blob([fontBytes])),query=vi.fn(async()=>[{family:'Inter',fullName:'Inter Regular',postscriptName:'Inter-Regular',style:'Regular',blob}]);
    vi.stubGlobal('queryLocalFonts',query);
    try{
      const first=requestExtrusionFont(PM,{font:'Inter',weight:500});await first.pending;expect(first.error).toBeUndefined();
      const second=requestExtrusionFont(PM,{font:'Inter',weight:800});expect(second.pending).toBeUndefined();expect(second.font).toBe(first.font);
      expect(query).toHaveBeenCalledTimes(1);expect(blob).toHaveBeenCalledTimes(1);
      expect(fontOutlines(second.font!,{text:'B',size:100,weight:800})).not.toEqual(fontOutlines(first.font!,{text:'B',size:100,weight:200}));
    }finally{vi.unstubAllGlobals();}
  });
  it('updates extruded glyph poses, colors and opacity from native text animators',()=>{
    const {PM,api}=editor();api.setExtrusion('title',{enabled:true,depth:40});
    const layer=PM.L('title');layer.d.animators=[{id:'move',mode:'range',unit:'characters',p:Object.fromEntries(Object.entries({start:0,end:100,amount:100,x:40,y:10,opacity:50,color:'#00FF00'}).map(([k,v])=>[k,PM.P(v)]))}];
    const first=layerExtrusionOutlines(PM,layer,0)!;expect(first.outlines.length).toBeGreaterThan(0);expect(first.outlines.every(o=>o.opacity===.5 && o.color?.toUpperCase()==='#00FF00')).toBe(true);
    const geometry=buildExtrusionGeometry(first.outlines,first.options);expect(geometry.getAttribute('color').itemSize).toBe(4);expect(geometry.userData.hasVertexAlpha).toBe(true);
    layer.d.animators[0].p.x.v=80;const second=layerExtrusionOutlines(PM,layer,0)!;
    expect(second.key).not.toBe(first.key);expect(second.outlines[0]!.outer[0]![0]-first.outlines[0]!.outer[0]![0]).toBeCloseTo(40);
  });
  it('keeps native artwork whose ID matches the implicit studio light',()=>{
    const {PM,api}=editor();api.setExtrusion('title',{enabled:true,depth:40});PM.L('title').id='default_sun';PM.ProjectIndex.invalidate();PM.touch();
    const world=compositionRuntime(PM);expect((world.objects.get('default_sun') as THREE.Mesh).isMesh).toBe(true);expect(world.objects.get('default_sunx')).toBeInstanceOf(THREE.DirectionalLight);disposeSceneRuntimes(PM);
  });
  it('puts legacy 3D planes in the shared pixel-to-world camera space',()=>{
    const {PM,api}=editor();PM.L('box').d.shape='star';api.setExtrusion('box',{enabled:true,depth:40});
    PM.Edit.apply({type:'set_layer',target:'paint',patch:{threeD:true}});PM.Edit.apply({type:'set_property',target:'paint',path:'position.z',value:-180});
    PM.GL.bounds=()=>({x0:0,y0:0,x1:100,y1:100,w:100,h:100});PM.GL.renderMaterialComposition=()=>new Uint8Array(100*100*4).fill(255);
    const world=compositionRuntime(PM),plane=world.objects.get('paint') as THREE.Mesh;
    expect(plane).toBeTruthy();expect(plane.userData.nativePlane).toBe(true);expect(plane.position.z).toBe(3);expect(plane.geometry.getAttribute('position').count).toBe(6);
    expect(compositionDepthIds(PM,PM.L('box'),0)).toContain('paint');disposeSceneRuntimes(PM);
  });
  it('reuses cached geometry across camera/transform/material changes and updates outlines at animated content samples',()=>{
    const {PM,api}=editor();api.setExtrusion('title',{enabled:true,depth:40});const first=compositionRuntime(PM),mesh=first.objects.get('title') as THREE.Mesh,geometry=mesh.geometry;
    expect(compositionDepthIds(PM,PM.L('title'),0)).toContain('title');expect(first.bounds(640,360,['title'])).toBeTruthy();
    api.setSurfaceMaterialProperties('title','side',{color:'#0000FF'});PM.Edit.apply({type:'set_property',target:'title',path:'rotation.y',value:30});compositionRuntime(PM,1);expect(first.objects.get('title')).toBe(mesh);expect(mesh.geometry).toBe(geometry);
    PM.Edit.apply({type:'set_content',target:'title',patch:{text:'Changed'}});compositionRuntime(PM,1);expect((first.objects.get('title') as THREE.Mesh).geometry).not.toBe(geometry);
    const base=layerExtrusionOutlines(PM,PM.L('title'),0)!;PM.Edit.apply({type:'replace_keyframes',target:'title',path:'c.size',keyframes:[{time:0,value:72},{time:1,value:100}]});const large=layerExtrusionOutlines(PM,PM.L('title'),1)!;expect(large.key).not.toBe(base.key);
    disposeSceneRuntimes(PM);
  });
});
