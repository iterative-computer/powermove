import * as THREE from 'three';
import { IMPORT_MAP_SLOTS, IMPORT_MAP_FLOATS, IMPORT_SCALAR_FLOATS } from './layout';

const wrap=(value:number)=>value===THREE.ClampToEdgeWrapping?1:value===THREE.MirroredRepeatWrapping?2:0;
const unwrap=(value:number)=>value===1?THREE.ClampToEdgeWrapping:value===2?THREE.MirroredRepeatWrapping:THREE.RepeatWrapping;
const rgb=(value:THREE.Color|undefined,fallback:number[])=>value?.toArray() ?? fallback;

/** Keep source sampling per slot: one image can use different UV sets and
 * transforms in different materials, while its pixels occupy one GPU layer. */
export function packImportedMaterial(m:any,layer:(texture:THREE.Texture|null)=>number,basic=false,screenLayer?:number):number[] {
  const textures=IMPORT_MAP_SLOTS.map(slot=>(basic && slot==='emissiveMap'?m.map:m[slot]) as THREE.Texture|undefined);
  const enabled=m.userData?.sourceMaterialIndex!==undefined || m.userData?.studioImported || textures.some(t=>t && (t.channel!==0 || t.rotation!==0 || t.offset.lengthSq()!==0 || !t.repeat.equals(new THREE.Vector2(1,1)) || t.wrapS!==THREE.RepeatWrapping || t.wrapT!==THREE.RepeatWrapping));
  if(!enabled)return new Array(IMPORT_SCALAR_FLOATS+IMPORT_MAP_SLOTS.length*IMPORT_MAP_FLOATS).fill(0);
  const values=[
    ...rgb(m.specularColor,[1,1,1]),m.sheenRoughness ?? m.roughness ?? 1,
    ...rgb(m.sheenColor,[1,1,1]),m.iridescence ?? (m.userData?.film_nm>0?1:0),
    ...rgb(m.attenuationColor,[1,1,1]),Number.isFinite(m.attenuationDistance)?m.attenuationDistance:0,
    m.thickness ?? 0,m.anisotropyRotation ?? 0,m.iridescenceIOR ?? 1.38,m.iridescenceThicknessRange?.[0] ?? 100,
    m.iridescenceThicknessRange?.[1] ?? 400,m.clearcoatNormalScale?.x ?? 1,m.clearcoatNormalScale?.y ?? 1,m.aoMapIntensity ?? 1,
    m.normalScale?.y ?? 1,m.normalScale?.x ?? 1,0,1,
  ];
  for(const [i,texture] of textures.entries()){
    if(i===1 && screenLayer!==undefined){values.push(screenLayer,0,0,0,1,0,0,0,0,1,0,0);continue;}
    if(!texture){values.push(-1,0,0,0,1,0,0,0,0,1,0,0);continue;}
    if(texture.channel!==0 && texture.channel!==1)throw new Error('Studio supports texture coordinates TEXCOORD_0 and TEXCOORD_1');
    if(texture.matrixAutoUpdate)texture.updateMatrix();const e=texture.matrix.elements;
    values.push(layer(texture),texture.channel,wrap(texture.wrapS),wrap(texture.wrapT),e[0]!,e[3]!,e[6]!,0,e[1]!,e[4]!,e[7]!,0);
  }
  return values;
}

/** Restore the portable scene contract through ordinary Three materials. */
export function restoreImportedMaterial(m:any,values:number[]|undefined,textures:THREE.Texture[]):void {
  if(!values || values.length!==IMPORT_SCALAR_FLOATS+IMPORT_MAP_SLOTS.length*IMPORT_MAP_FLOATS || values[23]!==1)return;
  if(!values.every(Number.isFinite))throw new Error('Invalid imported material data');
  m.userData.studioImported=true;
  m.specularColor=new THREE.Color().fromArray(values,0);m.sheenRoughness=values[3];
  m.sheenColor=new THREE.Color().fromArray(values,4);m.iridescence=values[7];
  m.attenuationColor=new THREE.Color().fromArray(values,8);m.attenuationDistance=values[11] || Infinity;
  m.thickness=values[12];m.anisotropyRotation=values[13];m.iridescenceIOR=values[14];m.iridescenceThicknessRange=[values[15],values[16]];
  m.clearcoatNormalScale=new THREE.Vector2(values[17],values[18]);m.aoMapIntensity=values[19];
  m.normalScale ??= new THREE.Vector2(1,1);m.normalScale.set(values[21]!,values[20]!);
  IMPORT_MAP_SLOTS.forEach((slot,i)=>{
    const o=IMPORT_SCALAR_FLOATS+i*IMPORT_MAP_FLOATS,index=values[o]!;
    if(index<0){m[slot]=null;return;}
    if(!Number.isInteger(index)||!textures[index])throw new Error('Imported material references a missing texture');
    const t=textures[index]!.clone();t.channel=values[o+1]!;t.wrapS=unwrap(values[o+2]!);t.wrapT=unwrap(values[o+3]!);
    t.matrixAutoUpdate=false;t.matrix.set(values[o+4]!,values[o+5]!,values[o+6]!,values[o+8]!,values[o+9]!,values[o+10]!,0,0,1);
    m[slot]=t;
  });
}
