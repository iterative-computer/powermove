// Native-only inputs to MTLFXTemporalDenoisedScaler. Appended to the common
// tracer so geometry, material maps and first-hit normals use the same code.
struct FX { previous_pos:vec4f, previous_fwd:vec4f, previous_right:vec4f, previous_up:vec4f, misc:vec4f }
@group(2) @binding(0) var fx_color:texture_storage_2d<rgba16float,write>;
@group(2) @binding(1) var fx_depth:texture_storage_2d<r32float,write>;
@group(2) @binding(2) var fx_motion:texture_storage_2d<rg32float,write>;
@group(2) @binding(3) var fx_diffuse:texture_storage_2d<rgba16float,write>;
@group(2) @binding(4) var fx_specular:texture_storage_2d<rgba16float,write>;
@group(2) @binding(5) var fx_normal:texture_storage_2d<rgba16float,write>;
@group(2) @binding(6) var fx_roughness:texture_storage_2d<r32float,write>;
@group(2) @binding(7) var fx_mask:texture_storage_2d<rgba32float,write>;
@group(2) @binding(8) var<uniform> F:FX;
@compute @workgroup_size(8,8)
fn fx_guides(@builtin(global_invocation_id) g:vec3u) {
  if(g.x>=P.dims.x||g.y>=P.dims.y){return;}
  let i=g.y*P.dims.x+g.x;let pixel=vec2i(g.xy);let px=vec2f(g.xy)+F.misc.zw+.5;
  seed=pcg(i^P.dims.z);let ray=camera_ray(px);let hit=primary_intersect(ray[0],ray[1],camera_limit(ray[0],ray[1]));let tf=floor_t(ray[0],ray[1]);
  var normal=vec3f(0.,1.,0.);var diffuse=vec3f(.82);var specular=vec3f(.04);var roughness=.8;var depth=1.;var point=ray[0]+ray[1]*1e5;
  if(hit.tri>=0&&hit.t<tf){let sf=surface(ray[0],ray[1],hit);point=sf.p;normal=sf.n;diffuse=sf.m.base.xyz*(1.-sf.m.base.w)*(1.-sf.m.spec.w);specular=mix(vec3f(.04*sf.m.extra.y),sf.m.base.xyz,sf.m.base.w);roughness=sf.m.spec.x;}
  else if(tf<BIG){point=ray[0]+ray[1]*tf;}
  if(dot(normal,ray[1])>0.){normal=-normal;}
  let viewz=dot(point-P.cam_pos.xyz,P.cam_fwd.xyz);let near=F.misc.x;let far=F.misc.y;
  let ortho=(P.opts.w&1u)!=0u;
  depth=clamp(select(far/(far-near)-near*far/((far-near)*max(viewz,near)),(viewz-near)/(far-near),ortho),0.,1.);
  var motion=vec2f(0.);let v=point-F.previous_pos.xyz;let z=dot(v,F.previous_fwd.xyz);
  if(z>1e-4){let scale=select(z,1.,ortho);let ndc=vec2f(dot(v,F.previous_right.xyz)/(scale*F.previous_right.w),-dot(v,F.previous_up.xyz)/(scale*F.previous_up.w));motion=(ndc*.5+.5)*vec2f(P.dims.xy)-px;}
  let a=accum[i];let b=bgsum[i];let n=max(a.w,1.);let coverage=max(a.w-b.w,0.)/n;
  var shade=1.;if(b.w>0.){shade=dot(b.xyz/b.w,vec3f(1.))/max(dot(P.backdrop.xyz,vec3f(1.)),1e-6);}
  // A complete linear image, with display-referred backdrop converted once.
  let color=(a.xyz+ebuf[i].xyz)/n*P.env_zenith.w+decode_srgb(P.backdrop.xyz*shade)*(1.-coverage);
  textureStore(fx_color,pixel,vec4f(min(color,vec3f(60000.)),1.));textureStore(fx_depth,pixel,vec4f(depth));textureStore(fx_motion,pixel,vec4f(motion,0.,0.));
  textureStore(fx_diffuse,pixel,vec4f(diffuse,1.));textureStore(fx_specular,pixel,vec4f(specular,1.));
  textureStore(fx_normal,pixel,vec4f(dot(normal,P.cam_right.xyz),dot(normal,P.cam_up.xyz),dot(normal,P.cam_fwd.xyz),0.));textureStore(fx_roughness,pixel,vec4f(roughness));
  textureStore(fx_mask,pixel,vec4f(coverage,1.-coverage,gbuf[i].z,shade));
}
