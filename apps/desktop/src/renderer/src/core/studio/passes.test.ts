import { expect, it, vi } from 'vitest';
import * as THREE from 'three';
const mocks=vi.hoisted(()=>({maps:[] as any[],size:512,uploads:[] as any[],mapCanvas:vi.fn((source:any,_flip:any,size=1024)=>({width:size,height:size,source}))}));
vi.mock('../scene3d/service',()=>({compositionRuntime:()=>({scene:{},camera:{},resizeCamera(){},raw:{environment:{samples:4}}})}));
vi.mock('./extract',()=>({STUDIO_MATERIAL_FLOATS:272,MAP_SIZE:1024,extractScene:()=>({maps:mocks.maps,mapSize:mocks.size,materials:new Float32Array(272),objects:['box'],bounds:new THREE.Box3()})}));
vi.mock('../scene3d/environment-render',()=>({attachStudioWorld(){}}));
vi.mock('../scene3d/material-maps',()=>({compositionById:()=>({id:'screen'})}));
vi.mock('./index',()=>({viewOf:()=>({}),cameraOf:()=>({near:.01,far:100}),mapCanvas:mocks.mapCanvas}));
vi.mock('./gpu',()=>({initStudioGpu:async()=>true,StudioTarget:class{setScene(_scene:any,maps:any){mocks.uploads=maps;}setView(){}dataPass(){return new Float32Array(4*4*4);}destroy(){} }}));
vi.mock('./png16',()=>({png16:async()=>new Blob()}));
import { renderStudioPasses } from './passes';

it.each([512,4096])('exports render-pass textures at the planned %i size',async size=>{
  mocks.size=size;mocks.mapCanvas.mockClear();
  const source={width:8192,height:8192};mocks.maps=[{source,flipY:true},{screen:'screen'}];
  const capture=vi.fn((_comp,_time,width,height)=>({width,height}));
  const output=renderStudioPasses({proj:{},GL:{renderMaterialComposition:capture}},0,4,4,['albedo']);
  await output.next();await output.return(undefined);
  expect(mocks.mapCanvas).toHaveBeenCalledWith(source,true,size);
  expect(capture).toHaveBeenCalledWith(expect.anything(),0,size,size,0);
  expect(mocks.uploads.map(source=>[source.width,source.height])).toEqual([[size,size],[size,size]]);
});
