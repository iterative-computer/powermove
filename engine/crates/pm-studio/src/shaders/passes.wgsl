// Data AOVs share the beauty tracer's camera, opacity, mesh and material lookup.
// P.counts.w selects depth, world normal, linear albedo, object ID, material ID,
// or the fraction of direct illumination removed by shadows.
@compute @workgroup_size(8,8)
fn data_pass(@builtin(global_invocation_id) gid:vec3u){
  if(gid.x>=P.dims.x || gid.y>=P.dims.y){return;}
  let idx=gid.y*P.dims.x+gid.x;seed=pcg(idx ^ pcg(P.dims.z));
  let ray=camera_ray(vec2f(gid.xy)+vec2f(0.5));
  var ro=ray[0];let rd=ray[1];
  var value=vec4f(0.);
  for(var cut=0;cut<64;cut++){
    let hit=primary_intersect(ro,rd,camera_limit(ro,rd));
    if(hit.tri<0){break;}
    let sf=surface(ro,rd,hit);
    if(sf.m.extra.z<0.5){let step=hit.t+ray_epsilon();ro+=rd*step;continue;}
    let kind=P.counts.w;
    if(kind==0u){let distance=select(length(sf.p-P.cam_pos.xyz),dot(sf.p-P.cam_pos.xyz,P.cam_fwd.xyz),(P.opts.w&1u)!=0u);let depth=clamp((distance-camera_near())/max(abs(P.backdrop.w)-camera_near(),1e-5),0.,1.);value=vec4f(vec3f(depth),1.);}
    if(kind==1u){value=vec4f(sf.n*0.5+vec3f(0.5),1.);}
    if(kind==2u){value=vec4f(sf.m.base.xyz,1.);}
    if(kind==3u){value=vec4f(f32(sf.object+1u)/65535.,0.,0.,1.);}
    if(kind==4u){value=vec4f((tris[u32(hit.tri)].uv2m.z+1.)/65535.,0.,0.,1.);}
    if(kind==5u){
      var total=0.;var visible=0.;
      if(P.opts.y==0u && (u32(sf.m.policy.y)&2u)!=0u){
        for(var k=0;k<32;k++){
          if(P.counts.x==0u){break;}
          let i=choose_light(sf.p);let l=light_at(i);let sample=sample_light(l,sf.p);
          let weight=max(dot(sf.n,sample.dir),0.)*dot(sample.radiance,vec3f(0.2126,0.7152,0.0722))/max(sample.pdf*light_selection_pdf(i,sf.p),1e-20);
          total+=weight;
          var tr=vec3f(1.);if(l.u.w>=0.){tr=transmittance(sf.p+sf.ng*ray_epsilon(),sample.dir,sample.dist);}
          visible+=weight*dot(tr,vec3f(0.2126,0.7152,0.0722));
        }
      }
      value=vec4f(vec3f(select(0.,1.-visible/max(total,1e-20),total>0.)),1.);
    }
    break;
  }
  aov[idx]=value;
}
