import { is3DLayer, world3D, parent3D, poseValues3D, CHANNELS_3D } from './space-3d';
/** Reparent using ordinary editable channels, keeping key identities and easing. */
export function preserveParentPose(PM: any, layer: any, parent: any, time: number): void {
  preserveWorldPose(PM, layer, is3DLayer(PM,layer) ? world3D(PM,layer,time) : PM.worldMatrix(layer, time), time, parent);
}

/** Rebase editable channels into a new grouping/parent coordinate space. */
export function preserveWorldPose(PM: any, layer: any, world: number[], time: number, parent = layer.parent ? PM.L(layer.parent) : null): void {
  if (world.length === 6 && is3DLayer(PM,layer)) {
    const [a,b,c,d,x,y]=world;
    world=[a!,b!,0,0,c!,d!,0,0,0,0,1,0,x!,y!,0,1];
  }
  if (world.length === 16) {
    for (const [key,fallback] of Object.entries(CHANNELS_3D)) layer.p[key] ||= PM.P(fallback);
    layer.threeD = true;
    const next = poseValues3D(PM,layer,world,time,parent3D(PM,layer,time,parent));
    applyPoseValues(PM,layer,next,time);
    return;
  }
  const p = PM.transformParentMatrix ? PM.transformParentMatrix(layer, time, parent) : parent ? PM.worldMatrix(parent, time) : [1,0,0,1,0,0];
  const currentWorld = PM.mul(p, PM.localMatrix(layer, time));
  if (world.every((value, index) => Math.abs(value - currentWorld[index]) < 1e-8)) return;
  const next = worldPoseValues(PM, layer, world, time, p);
  applyPoseValues(PM,layer,next,time);
}

function applyPoseValues(PM:any,layer:any,next:Record<string,number>,time:number):void {
  for (const [key,value] of Object.entries(next)) {
    const prop = layer.p[key], current = Number(PM.ev(layer,key,time));
    const scale = key.startsWith('scale.') && Math.abs(current)>1e-10 ? value/current : 1;
    const offset = key.startsWith('scale.') && Math.abs(current)>1e-10 ? 0 : value-current;
    if(prop.expr){if(Math.abs(scale-1)>1e-10||Math.abs(offset)>1e-10)prop.expr=`(${prop.expr}) * ${scale} + ${offset}`;continue;}
    prop.v = Number(prop.v)*scale+offset;
    for (const k of prop.kf) {k.v = Number(k.v)*scale+offset;if(k.inEase)k.inEase.speed*=scale;if(k.outEase)k.outEase.speed*=scale;}

  }
}

/** Convert a sampled world pose to editable local transform values. */
export function worldPoseValues(PM: any, layer: any, world: number[], time: number, p = PM.transformParentMatrix(layer,time), allowZero = false): Record<string,number> {
  if (world.length === 16) return poseValues3D(PM,layer,world,time,p?.length===16 ? p as any : parent3D(PM,layer,time));
  const determinant = p[0]*p[3]-p[1]*p[2];
  if (Math.abs(determinant) < 1e-10) throw new Error('This parent has zero scale. Increase its scale before parenting.');
  const inverse = [p[3]/determinant, -p[1]/determinant, -p[2]/determinant, p[0]/determinant,
    (p[2]*p[5]-p[3]*p[4])/determinant, (p[1]*p[4]-p[0]*p[5])/determinant];
  const [a,b,c,d,x,y] = PM.mul(inverse,world);
  const sx = Math.hypot(a,b), det = a*d-b*c;
  if (sx < 1e-10 || Math.abs(det) < 1e-10) {
    if (!allowZero) throw new Error('Increase the child’s zero scale before changing its parent.');
    const columnY = Math.hypot(c,d);
    if (sx >= 1e-10 && columnY >= 1e-10) throw new Error('Increase the group’s zero scale before ungrouping');
    const ax=PM.ev(layer,'anchor.x',time), ay=PM.ev(layer,'anchor.y',time);
    return {
      'position.x':x+a*ax+c*ay, 'position.y':y+b*ax+d*ay,
      'scale.x':sx*100, 'scale.y':columnY*100,
      rotation:sx>=1e-10 ? Math.atan2(b,a)*180/Math.PI : columnY>=1e-10 ? Math.atan2(-c,d)*180/Math.PI : PM.ev(layer,'rotation',time),
      skew:0,
    };
  }
  const sy = det/sx;
  const ax = PM.ev(layer,'anchor.x',time), ay = PM.ev(layer,'anchor.y',time);
  return {
    'position.x': x+a*ax+c*ay, 'position.y': y+b*ax+d*ay,
    'scale.x': sx*100, 'scale.y': sy*100,
    rotation: Math.atan2(b,a)*180/Math.PI,
    skew: Math.atan((a*c+b*d)/(sx*sy))*180/Math.PI,
  };
}
