const MiB=1024*1024;
export interface TextureBudget { memoryGB?:number; maxDimension?:number; maxLayers?:number; textureBytes?:number; sceneBytes?:number }
/** WebGPU has no free-VRAM query. Bound the shared array conservatively using
 * device memory, resident geometry and guaranteed adapter limits. */
export function planStudioTextures(sources:{width:number;height:number}[],geometryBytes:number,limits:TextureBudget={}) {
  if(sources.length>(limits.maxLayers ?? 256))throw new Error('Studio supports at most 256 unique texture images per scene');
  if(!sources.length)return {size:1,bytes:4,budgetBytes:0};
  const memory=Number.isFinite(limits.memoryGB)?limits.memoryGB!:8;
  const budgetBytes=Math.max(4*sources.length,Math.min(limits.textureBytes ?? 256*MiB,Math.max(8*MiB,memory*32*MiB),Math.max(8*MiB,(limits.sceneBytes ?? 512*MiB)-Math.max(0,geometryBytes))));
  const desired=Math.max(...sources.map(s=>Math.max(s.width,s.height)),1);
  const maximum=Math.min(4096,limits.maxDimension ?? 4096);
  let size=2**Math.floor(Math.log2(Math.max(1,Math.min(maximum,2**Math.ceil(Math.log2(desired))))));
  while(size>1 && size*size*4*sources.length>budgetBytes)size/=2;
  return {size,bytes:size*size*4*sources.length,budgetBytes};
}
