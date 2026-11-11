// Immutable first-batch guides avoid cross-pixel races inside the tracing pass.
@group(0) @binding(0) var<uniform> dims: vec4u;
@group(0) @binding(1) var<storage,read> guides: array<vec4f>;
@group(0) @binding(2) var<storage,read> accum: array<vec4f>;
@group(0) @binding(3) var<storage,read_write> emission: array<vec4f>;
fn normal(e:vec2f)->vec3f {
  var n=vec3f(e,1.-abs(e.x)-abs(e.y));
  if(n.z<0.){n=vec3f((1.-abs(n.yx))*select(vec2f(-1.),vec2f(1.),n.xy>=vec2f(0.)),n.z);}
  return n/max(length(n),1e-8);
}
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) p:vec3u){
  if(any(p.xy>=dims.xy)){return;}
  let count=dims.x*dims.y;let i=p.y*dims.x+p.x;let center=guides[i];
  let c=(accum[i].xyz+emission[i].xyz)/max(accum[i].w,1.);
  var edge=false;
  for(var dy=-1;dy<=1;dy++) {for(var dx=-1;dx<=1;dx++) {
    let at=clamp(vec2i(p.xy)+vec2i(dx,dy),vec2i(0),vec2i(dims.xy)-vec2i(1));
    let j=u32(at.y)*dims.x+u32(at.x);let other=guides[j];
    edge=edge || center.z!=other.z;
    if(center.z>0. && other.z>0.){
      edge=edge || dot(normal(center.xy),normal(other.xy))<0.98
        || abs(center.w-other.w)>0.01*max(min(center.w,other.w),1e-4);
    }
    let q=(accum[j].xyz+emission[j].xyz)/max(accum[j].w,1.);
    // First-batch Monte Carlo noise is not a persistent image edge. Keep the
    // original contrast threshold, but require significance above both means'
    // sampling variance. Geometry/coverage discontinuities stay unconditional.
    let cn=max(accum[i].w,1.);let qn=max(accum[j].w,1.);
    let cv=max(vec3f(0.),(emission[count+i].xyz-cn*c*c)/max(cn-1.,1.))/cn;
    let qv=max(vec3f(0.),(emission[count+j].xyz-qn*q*q)/max(qn-1.,1.))/qn;
    let contrast=0.15*max(max(length(c),length(q)),0.1);
    edge=edge || dot(c-q,c-q)>contrast*contrast+9.*dot(cv+qv,vec3f(1.));
  }}
  if(edge){emission[count+i].w=1.;}
}
