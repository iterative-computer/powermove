import { bridge } from '../../kernel/bridge';

export const FBX_FILE = /\.fbx$/i;
/** ufbx runs off the UI thread in a disposable native worker. Store only its
 * portable GLB result, so saving/reopening uses the existing glTF path. */
export async function fbxToGlb(file: File): Promise<File> {
  if(file.size===0 || file.size>256*1024*1024)throw new Error('FBX files must be smaller than 256 MB');
  const native=bridge()?.studioNative;
  if(!native)throw new Error('FBX import requires the desktop Studio importer');
  const reply=await native.request(`fbx:${crypto.randomUUID()}`,{op:'importFbx'},new Uint8Array(await file.arrayBuffer()));
  const bytes=reply.binary;
  if(bytes.byteLength<20 || new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength).getUint32(0,true)!==0x46546c67)throw new Error('FBX importer returned an invalid GLB');
  for(const warning of reply.result?.warnings ?? [])console.warn(`[Studio FBX] ${warning}`);
  return new File([new Uint8Array(bytes)],file.name.replace(FBX_FILE,'.glb'),{type:'model/gltf-binary'});
}
