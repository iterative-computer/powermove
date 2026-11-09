// Native Live direct-light reservoirs. Two or four fresh candidates, one reprojected
// reservoir and one spatial neighbor; visibility is traced only for the winner.
// Final/Refine retain the ordinary unbiased MIS estimator.
struct Reservoir { key: vec4u, weights: vec4f, surface: vec4f, normal: vec4f }
struct Resampling { dims: vec4u, prev_pos: vec4f, prev_fwd: vec4f, prev_right: vec4f, prev_up: vec4f }
@group(2) @binding(0) var<uniform> R: Resampling;
@group(2) @binding(1) var<storage, read> reservoirs_in: array<Reservoir>;
@group(2) @binding(2) var<storage, read_write> reservoirs_out: array<Reservoir>;
var<private> reservoir_pixel: u32;
var<private> reservoir_used: bool;
struct DirectCandidate { sample: LightSample, f: vec3f, importance: f32 }
fn direct_candidate(li: u32, sample_seed: u32, p: vec3f, n: vec3f, v: vec3f, m: Material, w: Lobes, floor_hit: bool) -> DirectCandidate {
  let saved_seed = seed; seed = sample_seed;
  let s = sample_light(light_at(li), p); seed = saved_seed;
  var f = vec3f(0.0); var pb = 0.0;
  if (s.pdf > 0.0) {
    if (floor_hit) { let nl = max(dot(n, s.dir), 0.0); f = vec3f(P.env_horizon.w * nl / PI); pb = nl / PI; }
    else { f = eval_reflect(m, n, v, s.dir); pb = pdf_reflect(m, w, n, v, s.dir); }
    let pl = s.pdf * light_selection_pdf(li,p);
    f *= s.radiance * (pl * pl / max(pl * pl + pb * pb, 1e-20)) / max(s.pdf,1e-20);
  }
  let importance = max(dot(f, vec3f(0.2126,0.7152,0.0722)), 0.0);
  return DirectCandidate(s, f, importance);
}
fn sample_lights(p: vec3f, n: vec3f, v: vec3f, m: Material, w: Lobes, floor_hit: bool) -> vec3f {
  if (P.opts.x == 0u || P.counts.x == 0u || reservoir_used) { return sample_lights_uniform(p,n,v,m,w,floor_hit); }
  reservoir_used = true;
  var chosen_light = 0u; var chosen_seed = 1u; var chosen_target = 0.0;
  var total_weight = 0.0; var candidates = 0u;
  let fresh=select(2u,4u,P.counts.x>8u);
  for (var j = 0u; j < fresh; j++) {
    let li = choose_light(p);
    let rs = pcg(seed); seed = pcg(rs);
    let c = direct_candidate(li,rs,p,n,v,m,w,floor_hit);
    let weight = c.importance / max(light_selection_pdf(li,p),1e-20);
    total_weight += weight; candidates++;
    if (weight > 0.0 && rnd()*total_weight < weight) { chosen_light=li;chosen_seed=rs;chosen_target=c.importance; }
  }
  // Project this surface into the previous camera. Object/material/light edits
  // invalidate history in the host; camera drags retain matching world points.
  let to_p = p-R.prev_pos.xyz; let z = dot(to_p,R.prev_fwd.xyz);
  if (R.dims.z != 0u && z > 1e-4) {
    let scale=select(z,1.,R.dims.w!=0u);let ndc=vec2f(dot(to_p,R.prev_right.xyz)/(scale*R.prev_right.w),-dot(to_p,R.prev_up.xyz)/(scale*R.prev_up.w));
    let dims=vec2f(f32(R.dims.x),f32(R.dims.y));let center=vec2i(floor((ndc*.5+.5)*dims));
    for(var neighbor=0u;neighbor<2u;neighbor++) {
      var offset=vec2i(0);if(neighbor>0u){let angle=rnd()*2.*PI;offset=vec2i(round(vec2f(cos(angle),sin(angle))*2.));}
      let pixel=center+offset;
      if(any(pixel<vec2i(0))||any(pixel>=vec2i(R.dims.xy))){continue;}
      let h=reservoirs_in[u32(pixel.y)*R.dims.x+u32(pixel.x)];
      if(h.key.w==0u||h.key.x>=P.counts.x||dot(h.normal.xyz,n)<.95||distance(h.surface.xyz,p)>max(.02*length(p-P.cam_pos.xyz),EPS*4.)){continue;}
      let c=direct_candidate(h.key.x,h.key.y,p,n,v,m,w,floor_hit);
      let count=min(h.key.z,16u);let weight=c.importance*h.weights.x*f32(count);
      total_weight+=weight;candidates+=count;
      if(weight>0.&&rnd()*total_weight<weight){chosen_light=h.key.x;chosen_seed=h.key.y;chosen_target=c.importance;}
    }
  }
  let normalization=total_weight/max(f32(candidates)*chosen_target,1e-20);
  let valid=u32(chosen_target>0. && normalization>0. && normalization<1e15);
  reservoirs_out[reservoir_pixel]=Reservoir(vec4u(chosen_light,chosen_seed,candidates,valid),vec4f(normalization,chosen_target,0.,0.),vec4f(p,1.),vec4f(n,0.));
  if(valid==0u){return vec3f(0.);}
  let c=direct_candidate(chosen_light,chosen_seed,p,n,v,m,w,floor_hit);
  var visibility=vec3f(1.);let light=light_at(chosen_light);
  if(P.opts.y==0u&&light.u.w>=0.&&(floor_hit||(u32(m.policy.y)&2u)!=0u)){visibility=transmittance(p,c.sample.dir,c.sample.dist);}
  return c.f * visibility * normalization;
}
