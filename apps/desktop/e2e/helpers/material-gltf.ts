/** Original tiny glTF panel. Geometry, UV sets and all images are embedded. */
export function materialGltf(image:Uint8Array,options:{basis?:boolean;secondary?:boolean;transform?:boolean;extensions?:Record<string,unknown>;unlit?:boolean}={}):Uint8Array {
  const parts:Buffer[]=[],views:any[]=[],accessors:any[]=[];
  const accessor=(array:Float32Array|Uint16Array,type:string,min?:number[],max?:number[])=>{
    const offset=parts.reduce((n,b)=>n+b.length,0),bytes=Buffer.from(array.buffer,array.byteOffset,array.byteLength);
    views.push({buffer:0,byteOffset:offset,byteLength:bytes.length});parts.push(bytes);
    const width=type==='VEC3'?3:type==='VEC2'?2:1;
    accessors.push({bufferView:views.length-1,componentType:array instanceof Float32Array?5126:5123,count:array.length/width,type,...(min?{min,max}:{})});return accessors.length-1;
  };
  const position=accessor(new Float32Array([-1,-1,0,1,-1,0,1,1,0,-1,1,0]),'VEC3',[-1,-1,0],[1,1,0]);
  const normal=accessor(new Float32Array([0,0,1,0,0,1,0,0,1,0,0,1]),'VEC3');
  const uv=accessor(new Float32Array([0,1,1,1,1,0,0,0]),'VEC2');
  const uv1=accessor(new Float32Array([.25,.25,.25,.25,.25,.25,.25,.25]),'VEC2');
  const indices=accessor(new Uint16Array([0,1,2,0,2,3]),'SCALAR');
  const bin=Buffer.concat(parts),extensions={...(options.unlit===false?{}:{KHR_materials_unlit:{}}),...options.extensions};
  const texInfo={index:0,texCoord:options.secondary?1:0,...(options.transform?{extensions:{KHR_texture_transform:{offset:[.5,.5],scale:[.5,.5],rotation:Math.PI/2,texCoord:1}}}:{})};
  const used=[...Object.keys(extensions),...(options.transform?['KHR_texture_transform']:[]),...(options.basis?['KHR_texture_basisu']:[])];
  return new TextEncoder().encode(JSON.stringify({asset:{version:'2.0'},scene:0,scenes:[{nodes:[0]}],nodes:[{name:'Imported panel',mesh:0}],
    meshes:[{primitives:[{attributes:{POSITION:position,NORMAL:normal,TEXCOORD_0:uv,TEXCOORD_1:uv1},indices,material:0}]}],
    accessors,bufferViews:views,buffers:[{byteLength:bin.length,uri:`data:application/octet-stream;base64,${bin.toString('base64')}`}],
    images:[{uri:`data:image/${options.basis?'ktx2':'png'};base64,${Buffer.from(image).toString('base64')}`}],
    textures:[options.basis?{extensions:{KHR_texture_basisu:{source:0}}}:{source:0}],samplers:[{wrapS:10497,wrapT:10497}],
    materials:[{name:'Imported physical panel',doubleSided:true,pbrMetallicRoughness:{baseColorFactor:[1,1,1,1],baseColorTexture:texInfo,metallicFactor:0,roughnessFactor:.6},extensions}],extensionsUsed:used,...(options.basis?{extensionsRequired:['KHR_texture_basisu']}:{})}));
}
