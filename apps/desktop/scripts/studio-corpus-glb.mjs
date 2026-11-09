/** Lossless mesh/instance transport into the ordinary editor import pipeline. */
export function corpusGlb(scene,screenPng) {
 const chunks=[],views=[],accessors=[],meshes=[],nodes=[];let offset=0;
 const append=(bytes,extra={})=>{const pad=Buffer.alloc((4-bytes.length%4)%4);views.push({buffer:0,byteOffset:offset,byteLength:bytes.length,...extra});chunks.push(bytes,pad);offset+=bytes.length+pad.length;return views.length-1;};
 const accessor=(array,type,componentType,extra={})=>{accessors.push({bufferView:append(Buffer.from(array.buffer,array.byteOffset,array.byteLength)),componentType,count:array.length/({SCALAR:1,VEC2:2,VEC3:3,VEC4:4}[type]),type,...extra});return accessors.length-1;};
 const xyz=v=>Array.isArray(v)?v:[v.x,v.y,v.z];
 const materials=scene.materials.map(m=>({
  pbrMetallicRoughness:{baseColorFactor:[...xyz(m.base),1],metallicFactor:m.metallic,roughnessFactor:m.roughness},
  extras:{subsurface:m.subsurface,droplets:m.droplets,studioProcedural:m.procedural,studioDispersion:m.dispersion,anisotropy:m.anisotropy},
  emissiveFactor:xyz(m.emission).map(v=>v/Math.max(1,...xyz(m.emission))),doubleSided:true,
  ...(m.screen&&screenPng?{emissiveTexture:{index:0}}:{}),
  extensions:{KHR_materials_transmission:{transmissionFactor:m.transmission},KHR_materials_ior:{ior:m.ior},
   KHR_materials_clearcoat:{clearcoatFactor:m.coat,clearcoatRoughnessFactor:m.coat_roughness},
   KHR_materials_specular:{specularFactor:m.specular},KHR_materials_emissive_strength:{emissiveStrength:Math.max(1,...xyz(m.emission))},
   KHR_materials_sheen:{sheenColorFactor:[m.sheen,m.sheen,m.sheen],sheenRoughnessFactor:Math.max(.2,m.roughness)},
   KHR_materials_iridescence:{iridescenceFactor:m.film_nm>0?1:0,iridescenceThicknessMinimum:m.film_nm,iridescenceThicknessMaximum:m.film_nm},
   KHR_materials_anisotropy:{anisotropyStrength:Math.abs(m.anisotropy),anisotropyRotation:m.anisotropy<0?Math.PI/2:0},KHR_materials_dispersion:{dispersion:m.dispersion*5}}
 }));
 for(const [i,object] of scene.objects.entries()){
  const g=object.mesh,pos=new Float32Array(g.positions.flatMap(xyz)),normals=new Float32Array(g.normals.flatMap(xyz)),uvs=new Float32Array(g.uvs.flat()),indices=new Uint32Array(g.indices.flat());
  const min=[Infinity,Infinity,Infinity],max=[-Infinity,-Infinity,-Infinity];for(let k=0;k<pos.length;k++){const a=k%3;min[a]=Math.min(min[a],pos[k]);max[a]=Math.max(max[a],pos[k]);}
  const attributes={POSITION:accessor(pos,'VEC3',5126,{min,max})};
  if(normals.length)attributes.NORMAL=accessor(normals,'VEC3',5126);if(uvs.length)attributes.TEXCOORD_0=accessor(uvs,'VEC2',5126);
  if(g.colors?.length)attributes.COLOR_0=accessor(new Float32Array(g.colors.flatMap(xyz)),'VEC3',5126);
  if(g.triangle_materials?.length){
   const byMaterial=new Map();g.triangle_materials.forEach((m,t)=>{if(!byMaterial.has(m))byMaterial.set(m,[]);byMaterial.get(m).push(...g.indices[t]);});
   meshes.push({name:`corpus-${i}`,primitives:[...byMaterial].map(([material,triangles])=>({attributes,indices:accessor(new Uint32Array(triangles),'SCALAR',5125),material}))});
  }else meshes.push({name:`corpus-${i}`,primitives:[{attributes,indices:accessor(indices,'SCALAR',5125),material:object.material}]});
 }
 const rotation=r=>{const [x,y,z]=xyz(r).map(v=>v*Math.PI/360),a=Math.cos(x),b=Math.sin(x),c=Math.cos(y),d=Math.sin(y),e=Math.cos(z),f=Math.sin(z);return [b*c*e+a*d*f,a*d*e-b*c*f,a*c*f+b*d*e,a*c*e-b*d*f];};
 if(scene.instances?.length)for(const p of scene.instances)nodes.push({mesh:p.object,translation:xyz(p.position??{x:0,y:0,z:0}),rotation:rotation(p.rotation??{x:0,y:0,z:0}),scale:xyz(p.scale??{x:1,y:1,z:1})});
 else scene.objects.forEach((_,mesh)=>nodes.push({mesh}));
 const image=screenPng?{images:[{bufferView:append(screenPng),mimeType:'image/png'}],textures:[{source:0}]}:{};
 const doc={asset:{version:'2.0',generator:'Powermove synthetic acceptance corpus'},scene:0,scenes:[{nodes:nodes.map((_,i)=>i)}],nodes,meshes,materials,accessors,bufferViews:views,buffers:[{byteLength:offset}],extensionsUsed:Object.keys(materials[0]?.extensions??{}),...image};
 const json=Buffer.from(JSON.stringify(doc)),j=Buffer.concat([json,Buffer.alloc((4-json.length%4)%4,32)]),binary=Buffer.concat(chunks),header=Buffer.alloc(12),jh=Buffer.alloc(8),bh=Buffer.alloc(8);
 header.writeUInt32LE(0x46546c67);header.writeUInt32LE(2,4);header.writeUInt32LE(28+j.length+binary.length,8);jh.writeUInt32LE(j.length);jh.writeUInt32LE(0x4e4f534a,4);bh.writeUInt32LE(binary.length);bh.writeUInt32LE(0x004e4942,4);
 return Buffer.concat([header,jh,j,bh,binary]);
}
