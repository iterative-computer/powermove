/** Reparent using ordinary editable channels, keeping key identities and easing. */
export function preserveParentPose(PM: any, layer: any, parent: any, time: number): void {
  const world = PM.worldMatrix(layer, time);
  const p = parent ? PM.worldMatrix(parent, time) : [1,0,0,1,0,0];
  const determinant = p[0]*p[3]-p[1]*p[2];
  if (Math.abs(determinant) < 1e-10) throw new Error('This parent has zero scale. Increase its scale before parenting.');
  const inverse = [p[3]/determinant, -p[1]/determinant, -p[2]/determinant, p[0]/determinant,
    (p[2]*p[5]-p[3]*p[4])/determinant, (p[1]*p[4]-p[0]*p[5])/determinant];
  const [a,b,c,d,x,y] = PM.mul(inverse,world);
  const sx = Math.hypot(a,b), det = a*d-b*c;
  if (sx < 1e-10 || Math.abs(det) < 1e-10) throw new Error('Increase the child’s zero scale before changing its parent.');
  const sy = det/sx;
  const ax = PM.ev(layer,'anchor.x',time), ay = PM.ev(layer,'anchor.y',time);
  const next: Record<string,number> = {
    'position.x': x+a*ax+c*ay, 'position.y': y+b*ax+d*ay,
    'scale.x': sx*100, 'scale.y': sy*100,
    rotation: Math.atan2(b,a)*180/Math.PI,
    skew: Math.atan((a*c+b*d)/(sx*sy))*180/Math.PI,
  };
  for (const [key,value] of Object.entries(next)) {
    const prop = layer.p[key], current = Number(PM.ev(layer,key,time));
    const scale = key.startsWith('scale.') && Math.abs(current)>1e-10 ? value/current : 1;
    const offset = key.startsWith('scale.') && Math.abs(current)>1e-10 ? 0 : value-current;
    if(prop.expr){if(Math.abs(scale-1)>1e-10||Math.abs(offset)>1e-10)prop.expr=`(${prop.expr}) * ${scale} + ${offset}`;continue;}
    prop.v = Number(prop.v)*scale+offset;
    for (const k of prop.kf) {k.v = Number(k.v)*scale+offset;if(k.inEase)k.inEase.speed*=scale;if(k.outEase)k.outEase.speed*=scale;}

  }
}
