// @vitest-environment happy-dom
import {describe,expect,it,vi} from 'vitest';
import * as THREE from 'three';
import {parseSvg} from '../../legacy/core/svg-import';
import {pathRasterGeometry} from '../../legacy/core/vector-paths';
import {materialLayerStatus,validateCompositionMaps} from './material-maps';
import {materialCompositionSample} from './composition-textures';
import {buildExtrusionGeometry} from './extrusion-geometry';
import {EXTRUSION_DEFAULTS} from './extrusion-data';
import {SceneRuntime} from './runtime';
import {parseScene} from './schema';

const square={outer:[[0,0],[20,0],[20,20],[0,20]] as [number,number][],holes:[]};
describe('Studio material parity safety',()=>{
  it('grows capture detail within a pixel and working-byte budget, including close-camera demands',()=>{
    const PM:any={proj:{compId:'root',comps:{paint:{id:'paint',w:64,h:64,fps:30,dur:2,layers:[]}}},GL:{maxTextureSize:16384},Memory:{budget:()=>128*1024*1024}};
    const sample=(materialCompositionSample as any)(PM,{from:0,d:{}},{compositionId:'paint'},0,{width:16384,height:16384});
    expect(sample.width).toBeGreaterThan(1024);
    expect(sample.width*sample.height).toBeLessThanOrEqual(4*1024*1024);
    // Capture targets, staging and mip storage must fit the preview allowance.
    expect(sample.width*sample.height*64).toBeLessThanOrEqual(128*1024*1024);
  });

  it('rejects nested 3D group members and dependencies before starting a capture',()=>{
    const receiver={id:'receiver',d:{data:{object:{material:{maps:{}}}}}},group={id:'art',type:'group'},nested={id:'nested',type:'group',group:'art'};
    const leaf:any={id:'leaf',type:'shape',group:'nested',threeD:false,d:{}};
    const project:any={compId:'root',layers:[receiver,group,nested,leaf],comps:{}};
    expect(materialLayerStatus(project,'receiver','art')).toBe('ready');
    leaf.threeD=true;expect(materialLayerStatus(project,'receiver','art')).toBe('ineligible');
    (receiver.d.data.object.material.maps as any).color={layerId:'art'};
    expect(()=>validateCompositionMaps(project)).toThrow(/3D|eligible/);
    leaf.threeD=false;leaf.matteSources=[{source:'receiver',mode:'alpha'}];
    expect(materialLayerStatus(project,'receiver','art')).toBe('cycle');
  });

  it.each(['#FF000080','transparent'])('keeps RGB and opacity when extruding %s',color=>{
    const geometry=buildExtrusionGeometry([{...square,color,opacity:.5}],EXTRUSION_DEFAULTS);
    const colors=geometry.getAttribute('color');
    expect(colors.getX(0)).toBeCloseTo(color==='transparent'?0:1);
    expect(colors.getY(0)).toBe(0);expect(colors.getZ(0)).toBe(0);
    expect(colors.getW(0)).toBeCloseTo(color==='transparent'?0:128/255*.5);
    expect(geometry.userData.hasVertexAlpha).toBe(true);geometry.dispose();
  });

  it('retains SVG paint alpha separately from RGB for 3D extrusion',()=>{
    const svg=parseSvg('<svg width="20" height="20"><rect width="20" height="20" fill="#ff000080" fill-opacity=".5"/><rect width="10" height="10" fill="transparent"/></svg>');
    expect(svg.paths[0]).toMatchObject({fill:'#FF0000',fillOpacity:128/255*50});
    expect(svg.paths[1]?.fillOpacity).toBe(0);
  });

  it('measures a path stroke by its visual half-width, without raster padding in tool bounds',()=>{
    const PM:any={mul:(a:number[],b:number[])=>[a[0]!*b[0]!+a[2]!*b[1]!,a[1]!*b[0]!+a[3]!*b[1]!,a[0]!*b[2]!+a[2]!*b[3]!,a[1]!*b[2]!+a[3]!*b[3]!,a[0]!*b[4]!+a[2]!*b[5]!+a[4]!,a[1]!*b[4]!+a[3]!*b[5]!+a[5]!]};
    const v={vertices:[{x:0,y:0,inX:0,inY:0,outX:0,outY:0},{x:100,y:0,inX:0,inY:0,outX:0,outY:0}],strokeWidth:10,strokeOpacity:100,fillEnabled:false,copies:1,repeatRotation:0,repeatX:0,repeatY:0,cap:'NONE',trimStart:0,trimEnd:100};
    const bounds=pathRasterGeometry(PM,{d:{}},0,1,[{v,matrix:[1,0,0,1,0,0]}]);
    expect(bounds.visual).toEqual({x0:0,y0:-5,x1:100,y1:5,w:100,h:10});
  });

  it('shares compatible textures across meshes and slots and replaces every binding at a new size',()=>{
    const source=new THREE.Group();
    for(let i=0;i<2;i++){
      const material=new THREE.MeshStandardMaterial();material.userData.sourceMaterialIndex=i;
      source.add(new THREE.Mesh(new THREE.PlaneGeometry(2,2),material));
    }
    const runtime=new SceneRuntime(),scene=parseScene({objects:[{id:'model',source:{assetId:'model'},useSourceMaterials:true,sourceMaterials:{0:{color:{compositionId:'paint'},emissive:{compositionId:'paint'}},1:{color:{compositionId:'paint'},emissive:{compositionId:'paint'}}}}]});
    let size=4;const capture=vi.fn(()=>({pixels:new Uint8Array(size*size*4),width:size,height:size,key:`frame-${size}`}));
    runtime.sync(scene,()=>({object3d:source}),p=>p.v,0,false,undefined,capture);
    const meshes=runtime.objects.get('model')!.children as THREE.Mesh<THREE.BufferGeometry,THREE.MeshStandardMaterial>[];
    const previous=meshes[0]!.material.map!;
    expect(meshes[1]!.material.map).toBe(previous);expect(meshes[0]!.material.emissiveMap).toBe(previous);
    // Future and imported scalar slots can share the same storage too.
    meshes[1]!.material.normalMap=previous;meshes[1]!.material.roughnessMap=previous;
    const disposed=vi.fn();previous.addEventListener('dispose',disposed);
    size=64;(runtime as any).prepareMaterialTextures(1024,1024,runtime.camera,4096);
    const next=meshes[0]!.material.map!;
    expect(next).not.toBe(previous);expect(meshes[1]!.material.map).toBe(next);
    expect(meshes[1]!.material.normalMap).toBe(next);expect(meshes[1]!.material.roughnessMap).toBe(next);
    expect(disposed).toHaveBeenCalledOnce();runtime.dispose();
  });
});
