/** A plain opaque depth set is already one resolved Studio image. Extracting
 * and recompositing an image for each object repeats full-frame readbacks.
 * Styled/transparent layers retain their individual compositing boundary. */
export function combineOpaqueDepthSet(PM:any,ids:Set<string>,separate:Set<string>,time:number,runtime?:any):boolean {
  if(ids.size<2 || [...ids].some(id=>separate.has(id)))return false;
  if(runtime)for(const id of ids){
    let transmitting=false;
    runtime.objects.get(id)?.traverseVisible((o:any)=>{
      if(o.isMesh && (Array.isArray(o.material)?o.material:[o.material]).some((m:any)=>(m?.transmission || 0)>0))transmitting=true;
    });
    if(transmitting)return false;
  }
  const comp=PM._renderComposition3D || PM.curComp?.() || PM.proj;
  return [...ids].every(id=>{
    const layer=comp?.layers?.find((l:any)=>l.id===id);
    if(!layer || layer.matteSource || layer.matteSources?.length || layer.transitionIn || layer.transitionOut)return false;
    const blend=typeof layer.blend==='object'?PM.evP?.(layer,layer.blend,time,'l.blend'):layer.blend;
    return !blend || blend==='normal';
  });
}
