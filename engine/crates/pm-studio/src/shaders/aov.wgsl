// OIDN features follow near-delta reflection/refraction to the first rough
// surface. Front-glass normals describe the pane, not the detail seen through
// it. Coverage and depth still describe the primary hit for compositing.
struct Guide { albedo:vec3f, normal:vec3f, coverage:f32, depth:f32, specular:f32 }
fn denoising_guide(origin:vec3f, direction:vec3f) -> Guide {
  var ro=origin;var rd=direction;var tint=vec3f(1.);
  var result=Guide(vec3f(0.),vec3f(0.),0.,0.,0.);
  var primary=true;let band=floor(rnd()*3.);
  for(var bounce=0u;bounce<24u;bounce++) {
    var h:Hit;
    if(primary){h=primary_intersect(ro,rd,camera_limit(ro,rd));}
    else{h=secondary_intersect(ro,rd,BIG);}
    let tf=floor_t(ro,rd);
    if(tf<h.t){result.albedo=tint*P.env_horizon.w;result.normal=vec3f(0.,1.,0.);return result;}
    if(h.tri<0){result.albedo=select(tint,vec3f(0.),primary);return result;}
    let sf=surface(ro,rd,h);let m=sf.m;
    if(m.extra.z<.999 && rnd()>=m.extra.z){ro=sf.p+rd*ray_epsilon();continue;}
    let front=dot(rd,sf.ng)<0.;
    if(!front && (u32(m.policy.y)&4u)==0u){ro=sf.p+rd*ray_epsilon();continue;}
    let n=select(-sf.n,sf.n,front);let ng=select(-sf.ng,sf.ng,front);
    if(primary){
      result.coverage=1.;result.depth=h.t;primary=false;
      result.specular=select(0.,1.,m.spec.x<.075 && (m.spec.w*(1.-m.base.w)>.5 || m.base.w>.5));
    }
    if(any(m.emit.xyz>vec3f(0.))){result.albedo=tint;result.normal=n;return result;}
    let transmission=m.spec.w*(1.-m.base.w);
    let near_delta=m.spec.x<.075;
    if(!near_delta || (transmission<.5 && m.base.w<.5)){
      result.albedo=tint*m.base.xyz;result.normal=n;return result;
    }
    let v=-rd;let tb=basis(n);
    let hm=tb*sample_vndf(transpose(tb)*v,max(m.spec.x*m.spec.x,.0005),rnd2());
    if(transmission>.5){
      let ior=max(1.0001,m.extra.x+m.layers.z*.04*(band-1.));
      let eta=select(1./ior,ior,front);
      let F=fresnel_dielectric(abs(dot(v,hm)),eta);
      var next=reflect(rd,hm);
      if(rnd()>=F){
        let refracted=select(refract(rd,hm,1./eta),rd,(u32(m.policy.y)&64u)!=0u);
        if(dot(refracted,refracted)>.5){next=refracted;tint*=m.base.xyz;}
      }
      rd=next;
    }else{rd=reflect(rd,hm);tint*=m.base.xyz;}
    ro=sf.p+ng*select(-ray_epsilon(),ray_epsilon(),dot(rd,ng)>0.);
  }
  // Trapped specular paths have no rough feature; never invent a front normal.
  result.albedo=tint;return result;
}
@compute @workgroup_size(8,8)
fn aovs(@builtin(global_invocation_id) gid:vec3u){
  let w=P.dims.x;let h=P.dims.y;if(gid.x>=w || gid.y>=h){return;}
  let idx=gid.y*w+gid.x;
  // Deterministic strata keep the lens guides stable across output frames.
  seed=pcg(idx^0x5bd1e995u);
  var albedo=vec3f(0.);var normal=vec3f(0.);var hits=0.;var dist=0.;var specular=0.;
  let count=u32(select(64.,clamp(-P.caustic.w,1.,64.),P.caustic.w<0.));
  var columns=u32(ceil(sqrt(f32(count))));
  loop {if(count%columns==0u){break;}columns-=1u;}
  let rows=count/columns;
  for(var k=0u;k<count;k++){
    let jitter=(vec2f(f32(k%columns),f32(k/columns))+rnd2())/vec2f(f32(columns),f32(rows));
    let ray=camera_ray(vec2f(gid.xy)+jitter);
    let guide=denoising_guide(ray[0],ray[1]);
    // Negative depth marks any near-specular coverage: temporal history
    // must not reproject refracted detail using the front surface.
    // Primary coverage is independent of the surface reached through glass.
    albedo+=guide.albedo*guide.coverage;normal+=guide.normal;
    hits+=guide.coverage;dist+=guide.depth;specular+=guide.specular;
  }
  var a=vec4f(albedo/max(hits,1.),hits/f32(count));
  var n=vec4f(normal/f32(count),select(1.,-1.,specular>0.)*dist/max(hits,1.));
  if(P.counts.z==2u && P.dims.z>0u){
    let weight=f32(P.dims.z);let old=aov[2u*idx];let previous=aov[2u*idx+1u];
    let coverage=old.w*weight+a.w;
    a=vec4f((old.xyz*old.w*weight+a.xyz*a.w)/max(coverage,1e-6),coverage/(weight+1.));
    n=vec4f((previous.xyz*weight+n.xyz)/(weight+1.),select(1.,-1.,previous.w<0. || n.w<0.)*(abs(previous.w)*weight+abs(n.w))/(weight+1.));
  }
  aov[2u*idx]=a;aov[2u*idx+1u]=n;
}
