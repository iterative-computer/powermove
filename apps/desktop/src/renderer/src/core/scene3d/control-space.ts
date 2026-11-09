import {isModelGroup,isModelNull,layer3DRole} from './layers';

/** Existing model channels use scene units; native artwork uses composition
 * pixels. Shared bounds controllers convert their own transform at evaluation
 * time, so both kinds of children retain their original channels and keys. */
export function usesSceneUnits(PM:any,layer:any):boolean {
  return !!layer3DRole(layer)||isModelGroup(PM,layer)||isModelNull(PM,layer);
}
export function boundsControlSpace(layer:any):'scene'|'composition'|null {
  return layer?.type==='null'&&['scene','composition'].includes(layer.d?.boundsNullSpace)?layer.d.boundsNullSpace:null;
}
export function controlMatrix(PM:any,layer:any,m:number[],scene:boolean):number[] {
  const basis=boundsControlSpace(layer);
  if(!basis||(basis==='scene')===scene)return m;
  const comp=PM.curComp?.()||PM.proj,u=6/Math.max(1,comp.h),s=[u,-u,-u],t=[-comp.w*u/2,3,0];
  // B maps composition pixels to scene units. Conjugate by B (or its
  // inverse) rather than treating a scene-unit translation as pixel motion.
  const affine=m.length===6,dimension=affine?2:3,stride=affine?2:4,offset=affine?4:12,out=m.slice();
  for(let row=0;row<dimension;row++){
    let translation=scene?t[row]!+s[row]!*m[offset+row]!:m[offset+row]!-t[row]!;
    for(let column=0;column<dimension;column++){
      const index=column*stride+row;
      out[index]=m[index]!*(scene?s[row]!/s[column]!:s[column]!/s[row]!);
      translation+=scene?-out[index]!*t[column]!:m[index]!*t[column]!;
    }
    out[offset+row]=scene?translation:translation/s[row]!;
  }
  return out;
}
