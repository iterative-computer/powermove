// Studio path tracer. Shared by the Rust engine (pm-studio) and the desktop
// app's WebGPU host (apps/desktop/src/renderer/src/core/studio): one program,
// one look, everywhere.
//
// One sample per pixel per iteration of the sample loop. Live runs one frame
// of samples and denoises; Refine and Final keep accumulating.
//
// Units are the scene's own (millimetres in pm-studio's product scenes, scene
// units in the app). With the shadow catcher on, the plane y = 0 shows the
// seamless backdrop darkened only by the shadows and contact occlusion the
// objects cast, so the void keeps its exact display color. With it off,
// pixels that see nothing are transparent (or the backdrop color when its
// alpha is set).

struct Params {
  cam_pos: vec4f,      // w: lens radius
  cam_fwd: vec4f,      // w: focus distance
  cam_right: vec4f,    // w: tan(fov/2) * aspect
  cam_up: vec4f,       // w: tan(fov/2)
  backdrop: vec4f,     // display-referred rgb; w: signed far clip when clipping enabled (+ catcher, - no catcher)
  env_zenith: vec4f,   // w: exposure
  env_horizon: vec4f,  // w: floor albedo for secondary bounces
  dims: vec4u,         // width, height, frame seed, samples this dispatch
  counts: vec4u,       // lights, max bounces, reset (1 = overwrite), firefly clamp (luminance)
  opts: vec4u,         // x: Live; y: shadows off (1); z: instance count
  ground: vec4f,       // environment radiance below the horizon; w: floor height
  caustic: vec4f,      // x: grid cell size, y: photons emitted so far, z: caustics on (1), w: photons this pass
  bounds: vec4f,       // scene centre (xyz) and radius, for sun photons
}

// opts.w keeps the legacy projection bit; its high bit enables clipping.
// The remaining bits are the positive f32 near plane (one mantissa bit unused).
fn camera_clip_on() -> bool { return (P.opts.w & 0x80000000u) != 0u; }
fn camera_near() -> f32 { return bitcast<f32>(P.opts.w & 0x7ffffffeu); }
fn camera_limit(ro:vec3f, rd:vec3f) -> f32 {
  if (!camera_clip_on()) { return BIG; }
  return max(0.0, (abs(P.backdrop.w) - dot(ro - P.cam_pos.xyz, P.cam_fwd.xyz)) / max(dot(rd, P.cam_fwd.xyz), 1e-6));
}

// Per-triangle shading data in BVH order (positions live in `pos`).
struct Tri { n0: vec4f, n1: vec4f, n2: vec4f, uv01: vec4f, uv2m: vec4f, tangent: vec4f }
struct Node { lmin: vec3f, left: u32, lmax: vec3f, lcount: u32, rmin: vec3f, right: u32, rmax: vec3f, rcount: u32 }
// base: rgb, metallic · spec: roughness, coat, coat roughness, transmission
// emit: rgb, subsurface radius (0: none) · extra: ior, specular, opacity, anisotropy (-1..1)
// maps: texture-array layers for color, emissive, occlusion-roughness-metal, normal (-1: none)
// layers: sheen, thin-film thickness (nm), dispersion, water droplets
// policy: ambient response, flags (casts=1,receives=2,double-sided=4,unlit=8,vertex-color=16), alpha test
// maps2: independent roughness, metalness, AO indices, normal-map scale
struct Material { base: vec4f, spec: vec4f, emit: vec4f, extra: vec4f, maps: vec4f, layers: vec4f, policy: vec4f, maps2: vec4f }
// pos.w: kind (0 softbox, 1 sphere, 2 spot, 3 sun).
// softbox: pos = centre, u/v = half extents (u.w: area); emits along u × v.
// sphere/spot: pos = centre, u.x = radius; spot: v.xyz = axis, v.w = cos outer, radiance.w = cos inner.
// sun: pos = direction toward the sun, u.x = cos angular radius.
struct Light { pos: vec4f, u: vec4f, v: vec4f, radiance: vec4f }

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> tris: array<Tri>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> materials: array<Material>;
// Scene data: the lights (4 vec4 each), then the instances (9 vec4 each, in
// top-level BVH order): rows of world→object (xyz linear, w translation),
// rows of the normal matrix (n0.w: the instance's BVH root node), rows of the
// object→world linear part (for tangents).
@group(0) @binding(4) var<storage, read> sdata: array<vec4f>;
fn light_at(i: u32) -> Light { let o = 4u * i; return Light(sdata[o], sdata[o + 1u], sdata[o + 2u], sdata[o + 3u]); }
fn inst_base(i: u32) -> u32 { return 4u * P.counts.x + 9u * i; }
fn to_object(i: u32, p: vec3f, w: f32) -> vec3f {
  let o = inst_base(i);
  return vec3f(dot(sdata[o].xyz, p), dot(sdata[o + 1u].xyz, p), dot(sdata[o + 2u].xyz, p)) + w * vec3f(sdata[o].w, sdata[o + 1u].w, sdata[o + 2u].w);
}
fn normal_to_world(i: u32, n: vec3f) -> vec3f {
  let o = inst_base(i) + 3u;
  return normalize(vec3f(dot(sdata[o].xyz, n), dot(sdata[o + 1u].xyz, n), dot(sdata[o + 2u].xyz, n)));
}
fn vector_to_world(i: u32, v: vec3f) -> vec3f {
  let o = inst_base(i) + 6u;
  return vec3f(dot(sdata[o].xyz, v), dot(sdata[o + 1u].xyz, v), dot(sdata[o + 2u].xyz, v));
}
@group(0) @binding(5) var maps: texture_2d_array<f32>;
@group(0) @binding(6) var map_samp: sampler;
// rgb: sum of scene radiance (linear, without directly seen emission); w: samples taken
@group(0) @binding(7) var<storage, read_write> accum: array<vec4f>;
// rgb: sum of backdrop display color for samples that saw the void/floor; w: how many
@group(0) @binding(8) var<storage, read_write> bgsum: array<vec4f>;
// xy: first-hit normal (octahedral), z: first-hit object + 1 (0: the void),
// w: first-hit distance (-1 for the void)
@group(0) @binding(9) var<storage, read_write> gbuf: array<vec4f>;
// rgb: sum of emission seen directly by the camera (a product's screen):
// exact per pixel, so Live denoises only what is noisy. a: Σ luminance² of
// each sample's radiance + emission (their noise, for Final's denoiser).
@group(0) @binding(10) var<storage, read_write> ebuf: array<vec4f>;
// Corner positions, 3 per triangle in BVH order (intersection reads only these).
@group(0) @binding(11) var<storage, read> pos: array<vec4f>;
// Caustics: a hashed photon grid at two cell sizes, 8 words per slot: flux
// (rgb, f32 bits), the cell's key, the photon count.
@group(0) @binding(12) var<storage, read_write> grid: array<atomic<u32>>;
const GRID_CELLS: u32 = 524288u;

const PI: f32 = 3.14159265;
const LEGACY_EPS: f32 = 0.004;
const BIG: f32 = 1e30;

var<private> seed: u32;
// The shading tangent of the surface being lit (anisotropic highlights follow it).
var<private> shade_t: vec3f;
// Randomly rotated three-sample wavelength strata, set by the camera kernel.
var<private> spectral_band: i32;
fn pcg(v: u32) -> u32 {
  let s = v * 747796405u + 2891336453u;
  let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}
fn rnd() -> f32 { seed = pcg(seed); return f32(seed >> 8u) * (1.0 / 16777216.0); }
fn rnd2() -> vec2f { return vec2f(rnd(), rnd()); }

struct Hit { t: f32, tri: i32, b: vec2f, inst: u32 }

fn hit_tri(ro: vec3f, rd: vec3f, i: u32, best: f32) -> vec3f {
  let p0 = pos[3u * i].xyz;
  let e1 = pos[3u * i + 1u].xyz - p0;
  let e2 = pos[3u * i + 2u].xyz - p0;
  let pv = cross(rd, e2);
  let det = dot(e1, pv);
  if (abs(det) < 1e-12) { return vec3f(BIG, 0.0, 0.0); }
  let inv = 1.0 / det;
  let tv = ro - p0;
  let u = dot(tv, pv) * inv;
  if (u < 0.0 || u > 1.0) { return vec3f(BIG, 0.0, 0.0); }
  let qv = cross(tv, e1);
  let v = dot(rd, qv) * inv;
  if (v < 0.0 || u + v > 1.0) { return vec3f(BIG, 0.0, 0.0); }
  let d = dot(e2, qv) * inv;
  if (d <= ray_epsilon() || d >= best) { return vec3f(BIG, 0.0, 0.0); }
  return vec3f(d, u, v);
}

fn box_t(ro: vec3f, inv: vec3f, mn: vec3f, mx: vec3f, best: f32) -> f32 {
  let a = (mn - ro) * inv;
  let b = (mx - ro) * inv;
  let lo = max(max(min(a.x, b.x), min(a.y, b.y)), min(a.z, b.z));
  let hi = min(min(max(a.x, b.x), max(a.y, b.y)), max(a.z, b.z));
  if (hi >= max(lo, 0.0) && lo < best) { return max(lo, 0.0); }
  return BIG;
}

fn leaf(ro: vec3f, rd: vec3f, first: u32, count: u32, h: ptr<function, Hit>, any: bool) -> bool {
  for (var k = 0u; k < count; k++) {
    let r = hit_tri(ro, rd, first + k, (*h).t);
    if (r.x < (*h).t) {
      *h = Hit(r.x, i32(first + k), r.yz, (*h).inst);
      if (any) { return true; }
    }
  }
  return false;
}

// One model's BVH (its own space), from `root`.
fn intersect_blas(ro: vec3f, rd: vec3f, root: u32, h: ptr<function, Hit>, any: bool) -> bool {
  let inv = 1.0 / rd;
  var stack: array<u32, 40>;
  var sp = 1;
  stack[0] = root;
  while (sp > 0) {
    sp -= 1;
    let n = nodes[stack[sp]];
    var tl = BIG;
    var tr = BIG;
    if (n.lcount != 0xffffffffu) { tl = box_t(ro, inv, n.lmin, n.lmax, (*h).t); }
    if (n.rcount != 0xffffffffu) { tr = box_t(ro, inv, n.rmin, n.rmax, (*h).t); }
    // Leaves are intersected at once; interior children are pushed far-first.
    if (tl < BIG && n.lcount > 0u) {
      if (leaf(ro, rd, n.left, n.lcount, h, any)) { return true; }
      tl = BIG;
    }
    if (tr < BIG && n.rcount > 0u) {
      if (leaf(ro, rd, n.right, n.rcount, h, any)) { return true; }
      tr = BIG;
    }
    if (tl < BIG && tr < BIG) {
      if (tl < tr) { stack[sp] = n.right; stack[sp + 1] = n.left; } else { stack[sp] = n.left; stack[sp + 1] = n.right; }
      sp += 2;
    } else if (tl < BIG) { stack[sp] = n.left; sp += 1; } else if (tr < BIG) { stack[sp] = n.right; sp += 1; }
  }
  return false;
}

// Two levels: the top-level BVH (node 0) over the instances' world bounds;
// each instance's ray is moved into its model's space (t is unchanged
// because the direction is not renormalised) and walks that model's BVH.
fn intersect(ro: vec3f, rd: vec3f, tmax: f32, any: bool) -> Hit {
  var h = Hit(tmax, -1, vec2f(0.0), 0u);
  if (arrayLength(&pos) < 3u || P.opts.z == 0u) { return h; }
  let inv = 1.0 / rd;
  var stack: array<u32, 24>;
  var sp = 1;
  stack[0] = 0u;
  while (sp > 0) {
    sp -= 1;
    let n = nodes[stack[sp]];
    for (var side = 0u; side < 2u; side++) {
      let mn = select(n.rmin, n.lmin, side == 0u);
      let mx = select(n.rmax, n.lmax, side == 0u);
      let count = select(n.rcount, n.lcount, side == 0u);
      let index = select(n.right, n.left, side == 0u);
      if (count == 0xffffffffu || box_t(ro, inv, mn, mx, h.t) >= BIG) { continue; }
      if (count == 0u) { stack[sp] = index; sp += 1; continue; }
      for (var k = 0u; k < count; k++) {
        let i = index + k;
        let oro = to_object(i, ro, 1.0);
        let ord = to_object(i, rd, 0.0);
        var hi = Hit(h.t, -1, vec2f(0.0), i);
        let done = intersect_blas(oro, ord, u32(sdata[inst_base(i) + 3u].w), &hi, any);
        if (hi.tri >= 0) { h = hi; }
        if (done) { return h; }
      }
    }
  }
  return h;
}

fn tri_material(tri: i32) -> Material { return materials[u32(tris[u32(tri)].uv2m.z)]; }

// How much light gets from `ro` to a point `dist` along `rd`: opaque surfaces
// block it; glass and partly transparent surfaces tint and dim it.
// Separate layer effects/opacity/masks are composited by the host. Other
// such layers remain visible to shadow and reflection rays, while a layer's
// own primary image includes the geometry that will show behind those masks.
fn primary_intersect(ro:vec3f,rd:vec3f,tmax:f32) -> Hit {
  var origin=ro;var offset=0.0;var left=tmax;
  for(var i=0;i<1024;i++){
    var hit=intersect(origin,rd,left,false);
    if(hit.tri<0){return Hit(tmax,-1,vec2f(0.0),0u);}
    if(sdata[P.counts.x*4u+hit.inst*9u+5u].w<0.5){hit.t+=offset;return hit;}
    let step=hit.t+ray_epsilon();origin+=rd*step;offset+=step;left-=step;
    if(left<=0.0){return Hit(tmax,-1,vec2f(0.0),0u);}
  }
  return Hit(tmax,-1,vec2f(0.0),0u);
}

fn surface_transmittance(ro: vec3f, rd: vec3f, dist: f32) -> vec3f {
  var o = ro;
  var left = dist - 2.0 * ray_epsilon();
  var t = vec3f(1.0);
  var nearest = false;
  for (var k = 0; k < 6; k++) {
    let h = intersect(o, rd, left, !nearest);
    if (h.tri < 0) { return t; }
    let sf = surface(o, rd, h);
    let m = sf.m;
    if ((u32(m.policy.y) & 1u) == 0u) {
      o += rd * (h.t + ray_epsilon()); left -= h.t + ray_epsilon(); nearest = true;
      if (left <= 0.0) { return t; } continue;
    }
    // With caustics on, light through glass arrives as photons (bent and
    // focused); here glass then blocks it like any solid. Live has no
    // photons and lets it through tinted instead.
    let tr = select(m.spec.w * (1.0 - m.base.w), 0.0, P.caustic.z > 0.5 && P.opts.x == 0u);
    let through = tr * 0.92 + (1.0 - m.extra.z);
    if (through <= 0.001) { return vec3f(0.0); }
    t *= mix(vec3f(1.0), m.base.xyz, tr) * min(through, 1.0);
    // Continue past this surface; nearest-first from now on so none is skipped.
    nearest = true;
    o = o + rd * (h.t + ray_epsilon());
    left -= h.t + ray_epsilon();
    if (left <= 0.0) { return t; }
  }
  return t;
}

struct Medium { sigma:f32, albedo:vec3f, g:f32 }
// The global atmosphere is one finite volume, shared by every camera,
// continuation and shadow segment. Restarting a distance budget at each bounce
// changes the light's optical depth and makes overlapping paths disagree.
fn global_fog_density(p:vec3f)->f32 {
  let h=world_row(3u);let q=p-P.cam_pos.xyz;
  if(dot(q,q)>h.w*h.w){return 0.;}
  return world_row(2u).w*exp(-max(p.y-h.y,0.)*h.z);
}
fn fog_interval(ro:vec3f,rd:vec3f,end:f32)->vec2f {
  let radius=world_row(3u).w;
  return volume_interval(vec4f(P.cam_pos.xyz,1.),vec4f(vec3f(radius),0.),ro,rd,end);
}
fn medium_at(p:vec3f)->Medium {
  var sigma=0.;var albedo=vec3f(0.);var g=0.;
  if(!world_on()){return Medium(0.,vec3f(0.),0.);}
  let fog=world_row(2u);let h=world_row(3u);
  let global=global_fog_density(p);
  sigma=global;albedo=fog.xyz*global;g=h.x*global;
  let config=world_row(4u);
  for(var i=0u;i<u32(config.y);i++){
    let at=u32(config.x)+3u*i;let center=world_row(at);let size=world_row(at+1u);let color=world_row(at+2u);
    let q=p-center.xyz;
    var inside=all(abs(q)<=size.xyz);
    if(center.w>0.5){inside=dot(q,q)<=size.x*size.x;}
    if(inside){sigma+=size.w;albedo+=color.xyz*size.w;g+=color.w*size.w;}
  }
  if(sigma<=0.){return Medium(0.,vec3f(0.),0.);}
  return Medium(sigma,albedo/sigma,g/sigma);
}
// At a real collision, select one overlapping medium by extinction. Carrying
// its albedo and phase yields the exact sum of colored HG scattering kernels;
// averaging HG anisotropy would change the shape of that mixture.
fn sample_medium(p:vec3f)->Medium {
  let combined=medium_at(p);if(combined.sigma<=0.){return combined;}
  var choice=rnd()*combined.sigma;let fog=world_row(2u);let h=world_row(3u);
  let global=global_fog_density(p);
  if(choice<global){return Medium(combined.sigma,fog.xyz,h.x);}choice-=global;
  let config=world_row(4u);
  for(var i=0u;i<u32(config.y);i++){
    let at=u32(config.x)+3u*i;let center=world_row(at);let size=world_row(at+1u);let color=world_row(at+2u);let q=p-center.xyz;
    var inside=all(abs(q)<=size.xyz);if(center.w>0.5){inside=dot(q,q)<=size.x*size.x;}
    if(inside){if(choice<size.w){return Medium(combined.sigma,color.xyz,color.w);}choice-=size.w;}
  }
  return combined;
}
fn medium_majorant()->f32 {
  if(!world_on()){return 0.;}
  var m=world_row(2u).w;let c=world_row(4u);
  for(var i=0u;i<u32(c.y);i++){m+=world_row(u32(c.x)+3u*i+1u).w;}
  return m;
}
fn volume_interval(center:vec4f,size:vec4f,ro:vec3f,rd:vec3f,end:f32)->vec2f {
  let q=ro-center.xyz;var near=0.;var far=0.;
  if(center.w>0.5){let b=dot(q,rd);let disc=b*b-dot(q,q)+size.x*size.x;if(disc<=0.){return vec2f(0.);}near=-b-sqrt(disc);far=-b+sqrt(disc);}
  else {let inv=select(1./rd,vec3f(1e30),abs(rd)<vec3f(1e-30));let a=(-size.xyz-q)*inv;let b=(size.xyz-q)*inv;near=max(max(min(a.x,b.x),min(a.y,b.y)),min(a.z,b.z));far=min(min(max(a.x,b.x),max(a.y,b.y)),max(a.z,b.z));}
  return vec2f(max(0.,near),min(end,far));
}
// Clip null-collision tracking to potentially occupied space and use a
// height-aware majorant. A small dense steam volume must not make a kilometre
// of empty ray march at the steam density.
fn tracking_segment(ro:vec3f,rd:vec3f,dist:f32)->vec3f {
  let end=dist;let fog=world_row(2u);let h=world_row(3u);
  var near=end;var far=0.;var majorant=0.;
  let global=fog_interval(ro,rd,end);
  if(fog.w>0. && global.y>global.x){
    near=global.x;far=global.y;
    majorant=fog.w*exp(-max(min(ro.y+rd.y*near,ro.y+rd.y*far)-h.y,0.)*h.z);
  }
  let config=world_row(4u);
  for(var i=0u;i<u32(config.y);i++){
    let at=u32(config.x)+3u*i;let interval=volume_interval(world_row(at),world_row(at+1u),ro,rd,end);
    if(interval.y>interval.x){near=min(near,interval.x);far=max(far,interval.y);majorant+=world_row(at+1u).w;}
  }
  return vec3f(near,far,majorant);
}
// Exact optical depth for exponential height fog plus homogeneous local shapes.
// All current media have analytic integrals. Use these for shadow rays in every
// tier: ratio tracking is unbiased but adds avoidable Bernoulli noise, especially
// when a homogeneous medium equals its majorant. Free-flight collision sampling
// still uses delta tracking, so multiple volume scattering is unchanged.
fn live_optical_depth(ro:vec3f,rd:vec3f,end:f32)->f32 {
  let fog=world_row(2u);let h=world_row(3u);var optical=0.;
  let global=fog_interval(ro,rd,end);let length=max(0.,global.y-global.x);
  let y0=ro.y+rd.y*global.x-h.y;let y1=y0+rd.y*length;let k=h.z;
  if(k==0.){optical=fog.w*length;}
  else if(abs(k*rd.y)*length<1e-4){optical=fog.w*exp(-k*max((y0+y1)*0.5,0.))*length;}
  else {
    var below=0.;
    if(rd.y>0.){below=clamp(-y0/rd.y,0.,length);}else{below=length-clamp(y0/(-rd.y),0.,length);}
    let upper=max(y0,y1);let lower=max(min(y0,y1),0.);
    optical=fog.w*(below+(exp(-k*lower)-exp(-k*max(upper,0.)))/(k*abs(rd.y)));
  }
  let config=world_row(4u);
  for(var i=0u;i<u32(config.y);i++){
    let at=u32(config.x)+3u*i;let size=world_row(at+1u);let interval=volume_interval(world_row(at),size,ro,rd,end);
    optical+=size.w*max(0.,interval.y-interval.x);
  }
  return optical;
}
fn medium_transmittance(ro:vec3f,rd:vec3f,dist:f32)->f32 {
  if(!world_on()){return 1.;}
  return exp(-live_optical_depth(ro,rd,dist));
}
fn transmittance(ro:vec3f,rd:vec3f,dist:f32)->vec3f {
  return surface_transmittance(ro,rd,dist)*medium_transmittance(ro,rd,dist);
}
fn phase_hg(mu:f32,g:f32)->f32 {let q=max(1.+g*g-2.*g*mu,1e-8);return (1.-g*g)/(4.*PI*q*sqrt(q));}
fn sample_hg(axis:vec3f,g:f32)->vec3f {
  let u=rnd();var mu=1.-2.*u;
  if(abs(g)>0.001){let q=(1.-g*g)/(1.-g+2.*g*u);mu=clamp((1.+g*g-q*q)/(2.*g),-1.,1.);}
  let phi=2.*PI*rnd();let st=sqrt(max(0.,1.-mu*mu));return basis(axis)*vec3f(st*cos(phi),st*sin(phi),mu);
}
fn medium_direct(p:vec3f,rd:vec3f,g:f32)->vec3f {
  var e=vec3f(0.);
  let samples=light_sample_count();
  for(var i=0u;i<samples;i++){
    var li=i;if(samples==1u){li=choose_light(p);}
    let l=sample_light(light_at(li),p);
    if(l.pdf>0.){e+=l.radiance*transmittance(p,l.dir,l.dist)*phase_hg(dot(rd,l.dir),g)/(l.pdf*light_selection_pdf(li,p));}
  }
  if(map_count()>0u){let d=sample_environment_direction();let pdf=environment_pdf(d);
    if(pdf>0.){e+=environment(d)*transmittance(p,d,BIG)*phase_hg(dot(rd,d),g)/pdf;}}
  return e;
}
fn medium_direct_live(p:vec3f,rd:vec3f,g:f32)->vec3f {
  // Smooth ambient fill, plus one visible practical/sun sample. The expensive
  // environment shadow integral is reserved for Refine/Final.
  var e=(environment(rd)+environment(vec3f(0.,1.,0.)))*0.5;
  if(P.counts.x>0u){let li=choose_light(p);let l=sample_light(light_at(li),p);
    if(l.pdf>0.){e+=l.radiance*transmittance(p,l.dir,l.dist)*phase_hg(dot(rd,l.dir),g)/(l.pdf*light_selection_pdf(li,p));}}
  return e;
}
// Delta-tracked real event; w=distance, negative means no event before the surface.
fn medium_event(ro:vec3f,rd:vec3f,dist:f32)->f32 {
  let segment=tracking_segment(ro,rd,dist);let majorant=segment.z;if(majorant<=0.){return -1.;}
  let end=segment.y;var t=segment.x;
  loop{t+=-log(max(1.-rnd(),1e-7))/majorant;if(t>=end){return -1.;}
    if(rnd()*majorant<medium_at(ro+rd*t).sigma){return t;}}
  return -1.;
}

// Contact occlusion measures coverage, while the direct-light estimate above
// follows transmission. Glass still occludes the dome: focused transmitted
// light is supplied by photons, rather than an unbent ambient shortcut.
fn ambient_visibility(ro:vec3f,rd:vec3f,dist:f32) -> f32 {
  var origin=ro;var left=dist;var visible=1.0;
  for(var k=0;k<8;k++){
    let hit=intersect(origin,rd,left,false);if(hit.tri<0){return visible;}
    let sf=surface(origin,rd,hit);
    if((u32(sf.m.policy.y)&1u)!=0u){visible*=1.0-sf.m.extra.z;if(visible<0.001){return 0.0;}}
    origin+=rd*(hit.t+ray_epsilon());left-=hit.t+ray_epsilon();if(left<=0.0){return visible;}
  }
  return visible;
}

fn floor_t(ro: vec3f, rd: vec3f) -> f32 {
  let y = ro.y - P.ground.w;
  if (P.backdrop.w <= 0.0 || rd.y >= -1e-6 || y <= 0.0) { return BIG; }
  return -y / rd.y;
}

// Optional environment trailer. Legacy app uploads end before this header.
fn world_base() -> u32 { return 4u*P.counts.x+9u*P.opts.z; }
fn world_on() -> bool {
  let o=world_base();
  if(arrayLength(&sdata)<o+6u){return false;}
  return sdata[o].x==735031. && sdata[o].y==1.;
}
// Product scenes retain their established tolerance. Environment scenes use
// the world extent/coordinate magnitude, so millimetre glass and trim in a
// metre-sized room are not skipped by the old 0.004-unit near threshold.
var<private> cached_epsilon:f32;
fn ray_epsilon()->f32 {
  if(cached_epsilon>0.){return cached_epsilon;}
  cached_epsilon=LEGACY_EPS;
  if(world_on()){
    let h=world_row(0u);let fog=world_row(2u);let volumes=world_row(4u);
    if(h.z*h.w>0. || fog.w>0. || volumes.y>0.){
      let magnitude=max(P.bounds.w,max(max(abs(P.bounds.x),abs(P.bounds.y)),abs(P.bounds.z)));
      cached_epsilon=clamp(magnitude*1e-6,1e-6,LEGACY_EPS);
    }
  }
  return cached_epsilon;
}
fn world_row(i:u32)->vec4f {return sdata[world_base()+i];}
fn map_count()->u32 {if(!world_on()){return 0u;}return u32(world_row(0u).z)*u32(world_row(0u).w);}
fn environment_uv(d:vec3f)->vec2f {
  let rotation=world_row(1u).x;
  var longitude=0.;if(abs(d.x)+abs(d.z)>1e-8){longitude=atan2(d.x,d.z);}
  return vec2f(fract((longitude-rotation)/(2.*PI)+1.),acos(clamp(d.y,-1.,1.))/PI);
}
fn environment_index(d:vec3f)->u32 {
  let wh=vec2u(world_row(0u).zw);let xy=min(vec2u(environment_uv(d)*vec2f(wh)),wh-1u);
  return xy.y*wh.x+xy.x;
}
fn environment_pdf(d:vec3f)->f32 {
  let count=map_count();if(count==0u){return 0.;}
  return world_row(u32(world_row(1u).w)+count+environment_index(d)).y;
}
fn environment(d: vec3f) -> vec3f {
  if(map_count()>0u){
    let wh=vec2i(world_row(0u).zw);let q=environment_uv(d)*vec2f(wh)-0.5;let xy=vec2i(floor(q));let f=fract(q);
    let x0=u32((xy.x%wh.x+wh.x)%wh.x);let x1=(x0+1u)%u32(wh.x);let y0=u32(clamp(xy.y,0,wh.y-1));let y1=u32(clamp(xy.y+1,0,wh.y-1));let base=u32(world_row(1u).w);
    let a=mix(world_row(base+y0*u32(wh.x)+x0).xyz,world_row(base+y0*u32(wh.x)+x1).xyz,f.x);
    let b=mix(world_row(base+y1*u32(wh.x)+x0).xyz,world_row(base+y1*u32(wh.x)+x1).xyz,f.x);
    return mix(a,b,f.y)*world_row(1u).y;
  }
  if (d.y < 0.0) { return mix(P.env_horizon.xyz, P.ground.xyz, clamp(-d.y * 4.0, 0.0, 1.0)); }
  return mix(P.env_horizon.xyz, P.env_zenith.xyz, clamp(d.y,0.,1.));
}
fn cdf_search(base:u32,count:u32,u:f32)->u32 {
  var lo=0u;var hi=count-1u;
  while(lo<hi){let mid=(lo+hi)/2u;if(world_row(base+mid).x<u){lo=mid+1u;}else{hi=mid;}}
  return lo;
}
fn light_tree_importance(index:u32,p:vec3f)->f32 {
  let base=u32(world_row(4u).w)+3u*index;let a=world_row(base);let b=world_row(base+1u);let power=world_row(base+2u);
  let d=max(max(a.xyz-p,p-b.xyz),vec3f(0.));let extent=b.xyz-a.xyz;
  return max(1e-20,power.x/max(dot(d,d)+0.25*dot(extent,extent),1e-6)+power.y);
}
fn light_tree_left_probability(index:u32,p:vec3f)->f32 {
  let base=u32(world_row(4u).w)+3u*index;let left=u32(world_row(base).w);let right=u32(world_row(base+1u).w);
  let a=light_tree_importance(left,p);let b=light_tree_importance(right,p);
  // Uniform mixture avoids starvation due to a loose bound or rounding.
  return clamp(a/(a+b),0.001,0.999);
}
// Small Final rigs stratify over emitters rather than randomly choosing one.
// Every emitter gets one NEE sample, so its MIS density has no selection PMF.
// Live and large light hierarchies retain the single-light estimator.
fn light_sample_count()->u32 {
  if(P.opts.x==0u && P.counts.x<=8u){return P.counts.x;}
  return min(P.counts.x,1u);
}
fn light_selection_pdf(i:u32,p:vec3f)->f32 {
  if(light_sample_count()>1u){return 1.;}
  if(!world_on()){return 1./max(f32(P.counts.x),1.);}
  let c=world_row(4u);let leaf=world_row(u32(c.z)+i);
  if(c.w<=0.){return leaf.y;}
  var node=u32(leaf.z);var pdf=1.;
  while(node!=0u){let parent=u32(world_row(u32(c.w)+3u*node+2u).z);let probability=light_tree_left_probability(parent,p);
    let left=u32(world_row(u32(c.w)+3u*parent).w);pdf*=select(1.-probability,probability,node==left);node=parent;}
  return pdf;
}
fn choose_light(p:vec3f)->u32 {
  if(!world_on()){return min(u32(rnd()*f32(P.counts.x)),P.counts.x-1u);}
  let c=world_row(4u);if(c.w<=0.){return cdf_search(u32(c.z),P.counts.x,rnd());}
  var node=0u;
  loop {let base=u32(c.w)+3u*node;let a=world_row(base);let b=world_row(base+1u);
    if(b.w<0.){return u32(a.w);}
    node=select(u32(b.w),u32(a.w),rnd()<light_tree_left_probability(node,p));}
  return 0u;
}
fn sample_environment_direction()->vec3f {
  let wh=vec2u(world_row(0u).zw);let count=wh.x*wh.y;
  let i=cdf_search(u32(world_row(1u).w)+count,count,rnd());
  let y=i/wh.x;let x=i%wh.x;
  let mu=mix(cos(PI*f32(y)/f32(wh.y)),cos(PI*f32(y+1u)/f32(wh.y)),rnd());
  let phi=2.*PI*(f32(x)+rnd())/f32(wh.x)+world_row(1u).x;
  let st=sqrt(max(0.,1.-mu*mu));return vec3f(st*sin(phi),mu,st*cos(phi));
}

// ── Lights ──────────────────────────────────────────────
fn light_kind(l: Light) -> u32 { return u32(l.pos.w); }
fn rect_normal(l: Light) -> vec3f { return normalize(cross(l.u.xyz, l.v.xyz)); }

// 1 − cos θ of a cone whose half-angle has sin² = s2, accurate when tiny.
fn one_minus_cos(s2: f32) -> f32 {
  if (s2 < 1e-3) { return 0.5 * s2 + 0.125 * s2 * s2; }
  return 1.0 - sqrt(max(0.0, 1.0 - s2));
}

fn sphere_t(ro: vec3f, rd: vec3f, c: vec3f, r: f32) -> f32 {
  let oc = ro - c;
  let b = dot(oc, rd);
  let q = dot(oc, oc) - r * r;
  let disc = b * b - q;
  if (disc < 0.0) { return BIG; }
  let s = sqrt(disc);
  var t = -b - s;
  if (t <= ray_epsilon()) { t = -b + s; }
  if (t <= ray_epsilon()) { return BIG; }
  return t;
}

// Distance to a light along the ray (softboxes, spheres and spots), or BIG.
fn hit_light(ro: vec3f, rd: vec3f, l: Light) -> f32 {
  let kind = light_kind(l);
  if (kind == 0u) {
    let n = rect_normal(l);
    let dn = dot(rd, n);
    if (dn >= 0.0) { return BIG; } // one-sided, emits along n
    let t = dot(l.pos.xyz - ro, n) / dn;
    if (t <= ray_epsilon()) { return BIG; }
    let q = ro + rd * t - l.pos.xyz;
    let uu = dot(q, l.u.xyz) / dot(l.u.xyz, l.u.xyz);
    let vv = dot(q, l.v.xyz) / dot(l.v.xyz, l.v.xyz);
    if (abs(uu) > 1.0 || abs(vv) > 1.0) { return BIG; }
    return t;
  }
  if (kind == 1u || kind == 2u) { return sphere_t(ro, rd, l.pos.xyz, l.u.x); }
  return BIG;
}

struct EmittingHit {t:f32,light:u32}
fn closest_light(ro:vec3f,rd:vec3f,limit:f32)->EmittingHit {
  var hit=EmittingHit(limit,0u);
  if(!world_on() || world_row(4u).w<=0.){
    for(var i=0u;i<P.counts.x;i++){let t=hit_light(ro,rd,light_at(i));if(t<hit.t){hit=EmittingHit(t,i);}}return hit;
  }
  let base=u32(world_row(4u).w);var stack:array<u32,32>;var size=1u;stack[0]=0u;let inv=1./rd;
  while(size>0u){size-=1u;let index=stack[size];let a=world_row(base+3u*index);let b=world_row(base+3u*index+1u);let power=world_row(base+3u*index+2u);
    if(power.x<=0. || box_t(ro,inv,a.xyz,b.xyz,hit.t)>=BIG){continue;}
    if(b.w<0.){let light=u32(a.w);let t=hit_light(ro,rd,light_at(light));if(t<hit.t){hit=EmittingHit(t,light);}}
    else{stack[size]=u32(a.w);stack[size+1u]=u32(b.w);size+=2u;}
  }
  return hit;
}

// Radiance leaving a light toward `-rd` at point q.
fn light_emit(l: Light, q: vec3f, rd: vec3f) -> vec3f {
  let kind = light_kind(l);
  if (kind == 0u) {
    // Soft edges: falloff toward the softbox rim, like a diffusion panel.
    let d = q - l.pos.xyz;
    let uu = abs(dot(d, l.u.xyz) / dot(l.u.xyz, l.u.xyz));
    let vv = abs(dot(d, l.v.xyz) / dot(l.v.xyz, l.v.xyz));
    return l.radiance.xyz * smoothstep(1.0, 0.82, max(uu, vv));
  }
  if (kind == 2u) {
    return l.radiance.xyz * smoothstep(l.v.w, l.radiance.w, dot(rd, l.v.xyz));
  }
  return l.radiance.xyz;
}

// Solid-angle pdf of sampling the direction `rd` (which reaches the light
// at distance t) with light sampling, before dividing by the light count.
fn light_pdf(l: Light, ro: vec3f, rd: vec3f, t: f32) -> f32 {
  let kind = light_kind(l);
  if (kind == 0u) {
    let cl = max(dot(-rd, rect_normal(l)), 1e-4);
    return t * t / (cl * abs(l.u.w));
  }
  if (kind == 3u) { return 1.0 / (2.0 * PI * one_minus_cos(1.0 - l.u.x * l.u.x)); }
  let d = l.pos.xyz - ro;
  let s2 = min(l.u.x * l.u.x / max(dot(d, d), 1e-12), 1.0);
  return 1.0 / (2.0 * PI * one_minus_cos(s2));
}

struct LightSample { dir: vec3f, dist: f32, radiance: vec3f, pdf: f32 }

fn cone_dir(axis: vec3f, omc: f32, u: vec2f) -> vec3f {
  let c = 1.0 - u.x * omc;
  let s = sqrt(max(0.0, 1.0 - c * c));
  let phi = 2.0 * PI * u.y;
  return basis(axis) * vec3f(s * cos(phi), s * sin(phi), c);
}

fn sample_light(l: Light, p: vec3f) -> LightSample {
  let kind = light_kind(l);
  let u = rnd2();
  if (kind == 0u) {
    let q = l.pos.xyz + l.u.xyz * (u.x * 2.0 - 1.0) + l.v.xyz * (u.y * 2.0 - 1.0);
    let d = q - p;
    let dist2 = dot(d, d);
    let dist = sqrt(dist2);
    let dir = d / dist;
    let cl = dot(-dir, rect_normal(l));
    if (cl <= 0.0) { return LightSample(dir, dist, vec3f(0.0), 0.0); }
    return LightSample(dir, dist, light_emit(l, q, dir), dist2 / (cl * abs(l.u.w)));
  }
  if (kind == 3u) {
    let omc = one_minus_cos(1.0 - l.u.x * l.u.x);
    let dir = cone_dir(normalize(l.pos.xyz), omc, u);
    return LightSample(dir, BIG, l.radiance.xyz, 1.0 / (2.0 * PI * omc));
  }
  let d = l.pos.xyz - p;
  let dist2 = dot(d, d);
  let s2 = min(l.u.x * l.u.x / max(dist2, 1e-12), 1.0);
  let omc = one_minus_cos(s2);
  let dir = cone_dir(d * inverseSqrt(dist2), omc, u);
  let t = sphere_t(p, dir, l.pos.xyz, l.u.x);
  let dist = select(sqrt(dist2), t, t < BIG);
  // Preserve editable point/spot falloff and finite range. Sampling a sphere
  // already contributes inverse-square attenuation; correct only the exponent.
  var attenuation = pow(max(sqrt(dist2), 0.01), 2.0 - l.u.z);
  if (l.u.y > 0.0) { attenuation *= pow(clamp(1.0 - pow(sqrt(dist2) / l.u.y, 4.0), 0.0, 1.0), 2.0); }
  return LightSample(dir, dist, light_emit(l, p + dir * dist, dir) * attenuation, 1.0 / (2.0 * PI * omc));
}

// ── Microfacet BSDF ─────────────────────────────────────
// Written as cos²·α² + sin² so a near-mirror never rounds the denominator to 0.
fn ggx_d(nh: f32, a: f32) -> f32 {
  let a2 = max(a * a, 1e-8);
  let c2 = nh * nh;
  let d = c2 * a2 + max(1.0 - c2, 0.0);
  return a2 / (PI * d * d);
}
fn smith_g1(nx: f32, a: f32) -> f32 {
  let a2 = a * a;
  return 2.0 * nx / (nx + sqrt(a2 + (1.0 - a2) * nx * nx));
}
fn smith_g2(nv: f32, nl: f32, a: f32) -> f32 {
  let a2 = a * a;
  let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
  let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
  return 2.0 * nv * nl / (gv + gl);
}
fn schlick(f0: vec3f, c: f32) -> vec3f { return f0 + (1.0 - f0) * pow(1.0 - clamp(c, 0.0, 1.0), 5.0); }
fn schlick1(f0: f32, c: f32) -> f32 { return f0 + (1.0 - f0) * pow(1.0 - clamp(c, 0.0, 1.0), 5.0); }

fn fresnel_dielectric(cosi: f32, eta: f32) -> f32 {
  if (abs(eta - 1.0) < 1e-6) { return 0.0; }
  let s2 = (1.0 - cosi * cosi) / (eta * eta);
  if (s2 >= 1.0) { return 1.0; }
  let ct = sqrt(1.0 - s2);
  let rs = (cosi - eta * ct) / (cosi + eta * ct);
  let rp = (eta * cosi - ct) / (eta * cosi + ct);
  return 0.5 * (rs * rs + rp * rp);
}

fn basis(n: vec3f) -> mat3x3f {
  let s = select(-1.0, 1.0, n.z >= 0.0);
  let a = -1.0 / (s + n.z);
  let b = n.x * n.y * a;
  return mat3x3f(vec3f(1.0 + s * n.x * n.x * a, s * b, -s * n.x), vec3f(b, s + n.y * n.y * a, -n.y), n);
}

// Visible-normal sampling (spherical caps), local frame z = normal.
fn sample_vndf(v: vec3f, a: f32, u: vec2f) -> vec3f {
  let vs = normalize(vec3f(v.x * a, v.y * a, v.z));
  let phi = 2.0 * PI * u.x;
  let z = (1.0 - u.y) * (1.0 + vs.z) - vs.z;
  let st = sqrt(clamp(1.0 - z * z, 0.0, 1.0));
  let h = vec3f(st * cos(phi), st * sin(phi), z) + vs;
  return normalize(vec3f(h.x * a, h.y * a, max(h.z, 1e-6)));
}

fn spec_pdf(nv: f32, nh: f32, a: f32) -> f32 { return smith_g1(nv, a) * ggx_d(nh, a) / (4.0 * nv); }

// Base-layer Fresnel: metal tints it (Schlick from the base color); a
// dielectric's specular weight scales all of it, so specular 0 means no
// reflection at any angle (OpenPBR).
fn fresnel_base(m: Material, c: f32) -> vec3f {
  return mix(vec3f(fresnel_dielectric_base(m, c)), schlick(m.base.xyz, c), m.base.w);
}
fn fresnel_dielectric_base(m: Material, c: f32) -> f32 { return m.extra.y * fresnel_dielectric(c, max(m.extra.x, 1e-4)); }

struct Lobes { coat: f32, spec: f32, diff: f32, trans: f32 }

fn lobe_probs(m: Material, nv: f32) -> Lobes {
  let metallic = m.base.w;
  let tr = m.spec.w * (1.0 - metallic);
  let fc = m.spec.y * fresnel_dielectric(nv, 1.5);
  let fs = fresnel_base(m, nv);
  let under = 1.0 - fc;
  // A narrow coat above a rough base sees a much spikier incident signal.
  // Give it more continuation samples; evaluation and the full mixture PDF
  // retain the actual Fresnel weights, so this changes variance, not energy.
  let sharp_coat=smoothstep(0.05,0.3,m.spec.x-m.spec.z);
  let coat_sample=mix(fc,sqrt(max(fc,0.)),sharp_coat);
  var w = Lobes(coat_sample, under * (1.0 - tr) * max(dot(fs, vec3f(0.333)), 0.04), under * (1.0 - metallic) * (1.0 - tr) * dot(m.base.xyz, vec3f(0.333)) * (1.0 - fs.x), under * tr);
  let s = w.coat + w.spec + w.diff + w.trans;
  if (s <= 0.0) { return Lobes(0.0, 1.0, 0.0, 0.0); }
  return Lobes(w.coat / s, w.spec / s, w.diff / s, w.trans / s);
}

// BEGIN GGX ENERGY LUT (generate_energy_lut.py, 16x16, 65536 samples/cell)
const GGX_ENERGY = array<f32,256>(
  1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000, 1.0000000,
  0.9995149, 0.9975306, 0.9994009, 0.9997385, 0.9998432, 0.9998991, 0.9999433, 0.9999668, 0.9999723, 0.9999759, 0.9999785, 0.9999805, 0.9999819, 0.9999831, 0.9999839, 0.9999847,
  0.9998668, 0.9602765, 0.9891992, 0.9953025, 0.9973423, 0.9982762, 0.9987854, 0.9990823, 0.9992651, 0.9993704, 0.9994726, 0.9995192, 0.9995876, 0.9996322, 0.9996607, 0.9996846,
  0.9999307, 0.9009376, 0.9511613, 0.9751463, 0.9854942, 0.9905095, 0.9932714, 0.9949052, 0.9959142, 0.9966162, 0.9971459, 0.9975276, 0.9978160, 0.9979972, 0.9981541, 0.9983052,
  0.9999513, 0.8887611, 0.9046925, 0.9353447, 0.9565657, 0.9696044, 0.9776516, 0.9828146, 0.9862361, 0.9886365, 0.9903357, 0.9916038, 0.9925431, 0.9933056, 0.9938858, 0.9943482,
  0.9999605, 0.9002687, 0.8809377, 0.8955202, 0.9162890, 0.9342815, 0.9480350, 0.9581357, 0.9655465, 0.9710064, 0.9751239, 0.9782422, 0.9806721, 0.9825794, 0.9841018, 0.9853382,
  0.9999657, 0.9117047, 0.8751582, 0.8698036, 0.8791993, 0.8934711, 0.9079385, 0.9207911, 0.9315839, 0.9404143, 0.9475749, 0.9533744, 0.9580838, 0.9619243, 0.9650813, 0.9677061,
  0.9999659, 0.9170885, 0.8737513, 0.8545163, 0.8507792, 0.8558218, 0.8651345, 0.8759994, 0.8869069, 0.8971015, 0.9062713, 0.9143502, 0.9213727, 0.9274357, 0.9326716, 0.9371737,
  0.9999630, 0.9163482, 0.8693440, 0.8415781, 0.8273902, 0.8225856, 0.8239331, 0.8290640, 0.8363204, 0.8445880, 0.8531400, 0.8615352, 0.8695126, 0.8769363, 0.8837555, 0.8899574,
  0.9999577, 0.9105324, 0.8595980, 0.8257710, 0.8038134, 0.7906102, 0.7838689, 0.7818285, 0.7831229, 0.7867149, 0.7918238, 0.7978647, 0.8044184, 0.8111799, 0.8179316, 0.8245343,
  0.9999506, 0.9006769, 0.8444079, 0.8050377, 0.7767564, 0.7566075, 0.7426788, 0.7335784, 0.7282349, 0.7258066, 0.7256197, 0.7271434, 0.7299465, 0.7336889, 0.7381008, 0.7429697,
  0.9999418, 0.8876192, 0.8244663, 0.7791896, 0.7450528, 0.7188441, 0.6986497, 0.6831833, 0.6715074, 0.6629068, 0.6568176, 0.6527865, 0.6504459, 0.6494893, 0.6496649, 0.6507633,
  0.9999316, 0.8720363, 0.8006711, 0.7489093, 0.7089376, 0.6771177, 0.6513547, 0.6302958, 0.6130032, 0.5987861, 0.5871230, 0.5776020, 0.5698929, 0.5637273, 0.5588843, 0.5551757,
  0.9999200, 0.8544762, 0.7739133, 0.7152010, 0.6693497, 0.6322149, 0.6014422, 0.5755360, 0.5534786, 0.5345401, 0.5181758, 0.5039692, 0.4915937, 0.4807881, 0.4713397, 0.4630724,
  0.9999073, 0.8353927, 0.7450075, 0.6791234, 0.6274775, 0.5853720, 0.5501559, 0.5201568, 0.4942426, 0.4716084, 0.4516599, 0.4339467, 0.4181179, 0.4038962, 0.3910585, 0.3794223,
  0.9998933, 0.8151684, 0.7146655, 0.6416557, 0.5845024, 0.5379095, 0.4989024, 0.4656126, 0.4367795, 0.4115101, 0.3891471, 0.3691934, 0.3512635, 0.3350532, 0.3203184, 0.3068604,
);
const GGX_ENERGY_AVG = array<f32,16>(1.0000000, 0.9999307, 0.9986904, 0.9941112, 0.9841617, 0.9667984, 0.9404353, 0.9041977, 0.8580536, 0.8028883, 0.7404596, 0.6731958, 0.6038664, 0.5351917, 0.4694983, 0.4085059);
const GGX_FRESNEL_B = array<f32,256>(
  1.0000000, 0.7082456, 0.4889455, 0.3276800, 0.2120836, 0.1316872, 0.0777600, 0.0431513, 0.0221327, 0.0102400, 0.0041152, 0.0013485, 0.0003200, 0.0000421, 0.0000013, 0.0000000,
  0.9105206, 0.7027009, 0.4873964, 0.3271246, 0.2118959, 0.1316543, 0.0777897, 0.0431957, 0.0221705, 0.0102664, 0.0041308, 0.0013561, 0.0003229, 0.0000429, 0.0000014, 0.0000000,
  0.7622395, 0.6408029, 0.4695333, 0.3205270, 0.2094958, 0.1310201, 0.0778671, 0.0435003, 0.0224861, 0.0105065, 0.0042846, 0.0014365, 0.0003576, 0.0000539, 0.0000036, 0.0000003,
  0.6117905, 0.5153683, 0.4131461, 0.2969507, 0.1999563, 0.1277173, 0.0772915, 0.0439672, 0.0231967, 0.0111252, 0.0047067, 0.0016741, 0.0004654, 0.0000907, 0.0000121, 0.0000013,
  0.4791196, 0.4009488, 0.3306023, 0.2520753, 0.1783985, 0.1185729, 0.0742778, 0.0437013, 0.0239175, 0.0119941, 0.0053818, 0.0020895, 0.0006692, 0.0001677, 0.0000322, 0.0000037,
  0.3701949, 0.3086247, 0.2532639, 0.1989162, 0.1472440, 0.1025750, 0.0672830, 0.0414565, 0.0238433, 0.0126570, 0.0060974, 0.0026013, 0.0009509, 0.0002854, 0.0000661, 0.0000082,
  0.2846041, 0.2352011, 0.1913172, 0.1513156, 0.1148771, 0.0830584, 0.0569274, 0.0368262, 0.0223508, 0.0126123, 0.0065299, 0.0030433, 0.0012419, 0.0004251, 0.0001117, 0.0000147,
  0.2192356, 0.1786037, 0.1439017, 0.1135760, 0.0870861, 0.0643722, 0.0455707, 0.0307091, 0.0195657, 0.0116841, 0.0064631, 0.0032559, 0.0014555, 0.0005523, 0.0001609, 0.0000230,
  0.1702178, 0.1360627, 0.1083894, 0.0850407, 0.0653045, 0.0487822, 0.0352275, 0.0244359, 0.0161662, 0.0101125, 0.0059132, 0.0031802, 0.0015338, 0.0006340, 0.0002023, 0.0000317,
  0.1338598, 0.1045334, 0.0821429, 0.0638938, 0.0489068, 0.0366484, 0.0267401, 0.0188865, 0.0128248, 0.0083028, 0.0050686, 0.0028721, 0.0014737, 0.0006547, 0.0002268, 0.0000396,
  0.1070342, 0.0812862, 0.0628516, 0.0483515, 0.0367707, 0.0275083, 0.0201473, 0.0143701, 0.0099192, 0.0065747, 0.0041410, 0.0024415, 0.0013156, 0.0006202, 0.0002312, 0.0000451,
  0.0872610, 0.0641189, 0.0486614, 0.0369399, 0.0278370, 0.0207200, 0.0151635, 0.0108579, 0.0075638, 0.0050890, 0.0032748, 0.0019872, 0.0011116, 0.0005498, 0.0002184, 0.0000475,
  0.0726525, 0.0513653, 0.0381644, 0.0285269, 0.0212544, 0.0156987, 0.0114422, 0.0081912, 0.0057286, 0.0038876, 0.0025369, 0.0015709, 0.0009034, 0.0004640, 0.0001944, 0.0000468,
  0.0618082, 0.0418000, 0.0303271, 0.0222762, 0.0163787, 0.0119797, 0.0086745, 0.0061897, 0.0043297, 0.0029504, 0.0019417, 0.0012189, 0.0007153, 0.0003782, 0.0001656, 0.0000437,
  0.0537057, 0.0345385, 0.0244087, 0.0175861, 0.0127391, 0.0092116, 0.0066146, 0.0046944, 0.0032761, 0.0022344, 0.0014772, 0.0009356, 0.0005570, 0.0003011, 0.0001366, 0.0000390,
  0.0476048, 0.0289511, 0.0198819, 0.0140276, 0.0099975, 0.0071379, 0.0050760, 0.0035776, 0.0024863, 0.0016935, 0.0011217, 0.0007144, 0.0004297, 0.0002362, 0.0001102, 0.0000336,
);
const GGX_FRESNEL_B_AVG = array<f32,16>(0.0476190, 0.0467935, 0.0457278, 0.0426717, 0.0378951, 0.0321600, 0.0263033, 0.0209327, 0.0163528, 0.0126337, 0.0097099, 0.0074579, 0.0057439, 0.0044465, 0.0034654, 0.0027218);
// END GGX ENERGY LUT

// Kulla-Conty isotropic conductor compensation. E is the directional albedo
// of this exact single-scatter correlated-Smith GGX with Fresnel=1. The extra
// reciprocal lobe restores missing inter-microfacet reflections. Colored Favg
// accounts for absorption between those reflections. Film/anisotropy require
// their own energy integrals and deliberately retain the original model.
fn ggx_energy(roughness:f32, cosine:f32)->f32 {
  let q=clamp(vec2f(cosine,roughness),vec2f(0.),vec2f(1.))*15.;
  let p=vec2u(floor(q));let hi=min(p+vec2u(1u),vec2u(15u));let f=fract(q);
  return mix(mix(GGX_ENERGY[p.y*16u+p.x],GGX_ENERGY[p.y*16u+hi.x],f.x),
             mix(GGX_ENERGY[hi.y*16u+p.x],GGX_ENERGY[hi.y*16u+hi.x],f.x),f.y);
}
// Split-sum Schlick integral estimates the directional energy of the base
// specular layer. The evaluated specular BSDF itself uses exact dielectric
// Fresnel. This removes smooth-surface Fresnel attenuation from rough diffuse
// layers, where it incorrectly absorbs energy at every wall bounce.
fn dielectric_layer_albedo(m:Material,cosine:f32)->f32 {
  let q=clamp(vec2f(cosine,m.spec.x),vec2f(0.),vec2f(1.))*15.;
  let p=vec2u(floor(q));let hi=min(p+vec2u(1u),vec2u(15u));let f=fract(q);
  let b=mix(mix(GGX_FRESNEL_B[p.y*16u+p.x],GGX_FRESNEL_B[p.y*16u+hi.x],f.x),
            mix(GGX_FRESNEL_B[hi.y*16u+p.x],GGX_FRESNEL_B[hi.y*16u+hi.x],f.x),f.y);
  let f0=fresnel_dielectric_base(m,1.);
  return clamp(f0*ggx_energy(m.spec.x,cosine)+(m.extra.y-f0)*b,0.,0.999);
}
fn diffuse_layer_weight(m:Material,nv:f32,nl:f32)->f32 {
  if(m.extra.w!=0. || m.layers.y>0.){return (1.-fresnel_dielectric_base(m,nv))*(1.-fresnel_dielectric_base(m,nl));}
  let q=clamp(m.spec.x,0.,1.)*15.;let index=u32(floor(q));let hi=min(index+1u,15u);
  let average=mix(GGX_ENERGY_AVG[index],GGX_ENERGY_AVG[hi],fract(q));
  let b=mix(GGX_FRESNEL_B_AVG[index],GGX_FRESNEL_B_AVG[hi],fract(q));
  let f0=fresnel_dielectric_base(m,1.);
  let reflected=clamp(f0*average+(m.extra.y-f0)*b,0.,0.999);
  // Reciprocal layered-diffuse normalization returns internally reflected
  // energy instead of losing both entry and exit Fresnel factors forever.
  return (1.-dielectric_layer_albedo(m,nv))*(1.-dielectric_layer_albedo(m,nl))/(1.-reflected);
}
fn ggx_multiscatter(m:Material,nv:f32,nl:f32)->vec3f {
  if(m.base.w<=0. || m.extra.w!=0. || m.layers.y>0.){return vec3f(0.);}
  let q=clamp(m.spec.x,0.,1.)*15.;let index=u32(floor(q));
  let average=mix(GGX_ENERGY_AVG[index],GGX_ENERGY_AVG[min(index+1u,15u)],fract(q));
  let favg=m.base.xyz+(vec3f(1.)-m.base.xyz)/21.;
  let color=favg*favg*average/max(vec3f(1.)-favg*(1.-average),vec3f(1e-6));
  let shape=(1.-ggx_energy(m.spec.x,nv))*(1.-ggx_energy(m.spec.x,nl))/max(1.-average,1e-6);
  return color*(m.base.w*shape*nl/PI);
}

// BSDF × cos for reflection (coat over specular + diffuse).
fn eval_reflect(m: Material, n: vec3f, v: vec3f, l: vec3f) -> vec3f {
  let nl = dot(n, l);
  let nv = dot(n, v);
  if (nl <= 0.0 || nv <= 0.0) { return vec3f(0.0); }
  let h = normalize(v + l);
  let nh = max(dot(n, h), 0.0);
  let vh = max(dot(v, h), 0.0);
  let metallic = m.base.w;
  let a = m.spec.x * m.spec.x;
  let F = thin_film(m, vh);
  var spec = F * ggx_d(nh, a) * smith_g2(nv, nl, a) / (4.0 * nv);
  if (m.extra.w != 0.0) {
    let fr = aniso_frame(n);
    spec = F * aniso_dg(fr, v, l, h, a, m.extra.w) / (4.0 * nv);
  }
  let tr = m.spec.w * (1.0 - metallic);
  // Glass reflects through its transmission lobe (Fresnel-weighted with the
  // refraction); here only the opaque part's specular remains.
  spec = spec * (1.0 - tr) + ggx_multiscatter(m, nv, nl);
  let diff = m.base.xyz * (1.0 - metallic) * (1.0 - tr) * diffuse_layer_weight(m, nv, nl) * nl / PI;
  let ca = m.spec.z * m.spec.z;
  let fc = m.spec.y * fresnel_dielectric(vh, 1.5);
  let coat = fc * ggx_d(nh, ca) * smith_g2(nv, nl, ca) / (4.0 * nv);
  let atten = (1.0 - m.spec.y * fresnel_dielectric(nv, 1.5)) * (1.0 - m.spec.y * fresnel_dielectric(nl, 1.5));
  return vec3f(coat) + (spec + diff + sheen(m, nh, nl, nv)) * atten;
}

// Thin-film interference integrates polarized Airy reflectance over the
// visible spectrum. RGB metals infer a complex IOR
// from F0 and the existing Schlick grazing reflectance. This is not measured
// optical data, but preserves interface energy, phase and spectral bandwidth
// instead of multiplying RGB by a cosine.
// Rec.709 quadrature weights: Wyman et al., JCGT 2(2), 2013 analytic CIE fits.
const FILM_RESPONSE = array<vec3f,20>(
  vec3f(-0.000785372, 0.000024754, 0.004430319),
  vec3f(0.008359184, -0.006727168, 0.044917621),
  vec3f(0.027950585, -0.036934153, 0.303418314),
  vec3f(0.027306534, -0.038623185, 0.388818358),
  vec3f(-0.025350407, 0.007193522, 0.279132034),
  vec3f(-0.071348804, 0.076281823, 0.092398008),
  vec3f(-0.124250028, 0.182756270, 0.013058868),
  vec3f(-0.131489187, 0.290440893, -0.025214430),
  vec3f(-0.019964978, 0.284182402, -0.034767330),
  vec3f(0.163316562, 0.203133470, -0.030655314),
  vec3f(0.334370610, 0.086687405, -0.020228530),
  vec3f(0.386906991, -0.006059439, -0.009520296),
  vec3f(0.269381002, -0.027796940, -0.003544440),
  vec3f(0.117376535, -0.013459821, -0.001368065),
  vec3f(0.032273448, -0.002069502, -0.000589651),
  vec3f(0.005448043, 0.000541457, -0.000216115),
  vec3f(0.000498633, 0.000334740, -0.000057101),
  vec3f(0.000006038, 0.000080239, -0.000010662),
  vec3f(-0.000004679, 0.000011974, -0.000001442),
  vec3f(-0.000000710, 0.000001258, -0.000000146),
);

fn conductor_fresnel(c:f32,n:vec3f,k:vec3f)->vec3f {
  let c2=c*c;let s2=1.-c2;
  let t=n*n-k*k-s2;
  let ab=sqrt(t*t+4.*n*n*k*k);
  let ar=sqrt(max(vec3f(0.),0.5*(ab+t)));
  let t1=ab+c2;let t2=2.*c*ar;
  let rs=(t1-t2)/max(t1+t2,vec3f(1e-8));
  let t3=c2*ab+s2*s2;let t4=t2*s2;
  return 0.5*rs*(vec3f(1.)+(t3-t4)/max(t3+t4,vec3f(1e-8)));
}
fn film_airy(r1:f32,re:vec3f,im:vec3f,phase:f32)->vec3f {
  let x=re*cos(phase)-im*sin(phase);
  let y=re*sin(phase)+im*cos(phase);
  return ((r1+x)*(r1+x)+y*y)/max((1.+r1*x)*(1.+r1*x)+r1*r1*y*y,vec3f(1e-8));
}
fn thin_film(m:Material,cosv:f32)->vec3f {
  let F=fresnel_base(m,cosv);let nm=m.layers.y;
  if(nm<=0.){return F;}
  let f0=clamp(fresnel_base(m,1.),vec3f(1e-5),vec3f(0.9999));
  let root=sqrt(f0);
  var eta=(vec3f(1.)+root)/(vec3f(1.)-root);
  var extinction=vec3f(0.);
  if(m.base.w>0.5){
    // RGB F0 alone has no unique complex IOR. Preserve its Schlick reflectance
    // at 82 degrees as the second constraint, solving exact conductor Fresnel.
    // This defines phase shifts without introducing a new material parameter.
    var lo=(vec3f(1.)-root)/(vec3f(1.)+root);var hi=eta;
    let grazing=schlick(f0,1./7.);
    for(var j=0u;j<12u;j++){
      let n=0.5*(lo+hi);
      let k=sqrt(max(vec3f(0.),(f0*(n+1.)*(n+1.)-(n-1.)*(n-1.))/(1.-f0)));
      let value=conductor_fresnel(1./7.,n,k);
      lo=select(lo,n,value>grazing);hi=select(n,hi,value>grazing);
    }
    eta=0.5*(lo+hi);
    extinction=sqrt(max(vec3f(0.),(f0*(eta+1.)*(eta+1.)-(eta-1.)*(eta-1.))/(1.-f0)));
  }
  let c1=max(cosv,1e-5);let sin2=1.-c1*c1;let nf=1.38;
  let c2=sqrt(max(0.,1.-sin2/(nf*nf)));
  // Complex Snell law: q = sqrt((n+ik)^2 - sin(theta)^2).
  let nr=eta*eta-extinction*extinction;let ni=2.*eta*extinction;
  let t=nr-sin2;let mag=sqrt(t*t+ni*ni);
  let qr=sqrt(max(vec3f(0.),0.5*(mag+t)));
  let qi=sqrt(max(vec3f(0.),0.5*(mag-t)));
  // s-polarized reflection at film/substrate.
  let ar=vec3f(nf*c2)-qr;let ai=-qi;let br=vec3f(nf*c2)+qr;let bi=qi;
  let sd=max(br*br+bi*bi,vec3f(1e-8));
  let sr=(ar*br+ai*bi)/sd;let si=(ai*br-ar*bi)/sd;
  // p-polarized reflection, multiplied through by the complex bulk IOR.
  let pr=nr*c2-nf*qr;let pi=ni*c2-nf*qi;
  let dr=nr*c2+nf*qr;let di=ni*c2+nf*qi;
  let pd=max(dr*dr+di*di,vec3f(1e-8));
  let tr=(pr*dr+pi*di)/pd;let ti=(pi*dr-pr*di)/pd;
  let rs=(c1-nf*c2)/(c1+nf*c2);let rp=(nf*c1-c2)/(nf*c1+c2);
  let bare=0.5*(film_airy(rs,sr,si,0.)+film_airy(rp,tr,ti,0.));
  var result=vec3f(0.);
  for(var i=0u;i<20u;i++){
    let wavelength=390.+20.*f32(i);let phase=4.*PI*nf*nm*c2/wavelength;
    let reflectance=0.5*(film_airy(rs,sr,si,phase)+film_airy(rp,tr,ti,phase));
    result+=FILM_RESPONSE[i]*reflectance;
  }
  return clamp(result*F/max(bare,vec3f(1e-6)),vec3f(0.),vec3f(1.));
}

// Sheen (fabric, soft-touch): the "Charlie" distribution with a soft
// visibility term, a white rim that grows at grazing angles.
fn sheen(m: Material, nh: f32, nl: f32, nv: f32) -> vec3f {
  if (m.layers.x <= 0.0) { return vec3f(0.0); }
  let r = max(m.spec.x, 0.07);
  let inv = 1.0 / r;
  let s2 = max(1.0 - nh * nh, 0.0);
  let d = (2.0 + inv) * pow(s2, inv * 0.5) / (2.0 * PI);
  let vis = 1.0 / (4.0 * (nl + nv - nl * nv));
  return vec3f(m.layers.x * d * vis * nl);
}

fn pdf_reflect(m: Material, w: Lobes, n: vec3f, v: vec3f, l: vec3f) -> f32 {
  let nl = dot(n, l);
  let nv = dot(n, v);
  if (nl <= 0.0 || nv <= 0.0) { return 0.0; }
  let h = normalize(v + l);
  let nh = max(dot(n, h), 0.0);
  let a = m.spec.x * m.spec.x;
  let ca = m.spec.z * m.spec.z;
  var sp = spec_pdf(nv, nh, a);
  if (m.extra.w != 0.0) {
    let fr = aniso_frame(n);
    let ax = aniso_alpha(a, m.extra.w);
    let vl = transpose(fr) * v;
    let hl = transpose(fr) * h;
    sp = aniso_g1(vl, ax) * aniso_d(hl, ax) / (4.0 * max(vl.z, 1e-4));
  }
  return w.coat * spec_pdf(nv, nh, ca) + w.spec * sp + w.diff * nl / PI;
}

// ── Subsurface random walk ──
// Inside the object, light travels an exponential distance (mean = the
// material's radius) between isotropic scatters, each tinted by the base
// color, until it reaches the surface again. Returns the exit point (w = 1)
// and leaves its outward normal in `exit_n`; w = 0 when it never leaves.
var<private> exit_n: vec3f;
fn random_walk(start: vec3f, dir0: vec3f, m: Material) -> vec4f {
  var p = start;
  var d = dir0;
  for (var k = 0; k < 48; k++) {
    let dist = -log(max(1.0 - rnd(), 1e-6)) * m.emit.w;
    let h = intersect(p, d, dist, false);
    if (h.tri >= 0) {
      let i = u32(h.tri);
      let p0 = pos[3u * i].xyz;
      var ng = normal_to_world(h.inst, cross(pos[3u * i + 1u].xyz - p0, pos[3u * i + 2u].xyz - p0));
      if (dot(ng, d) < 0.0) { ng = -ng; }
      exit_n = ng;
      return vec4f(p + d * h.t, 1.0);
    }
    p = p + d * dist;
    // Uniform direction on the sphere.
    let u = rnd2();
    let z = 1.0 - 2.0 * u.x;
    let r = sqrt(max(0.0, 1.0 - z * z));
    d = vec3f(r * cos(2.0 * PI * u.y), r * sin(2.0 * PI * u.y), z);
  }
  return vec4f(0.0);
}

// ── Anisotropic GGX (brushed metal): roughness stretched along the tangent ──
fn aniso_frame(n: vec3f) -> mat3x3f {
  var t = shade_t - n * dot(n, shade_t);
  if (dot(t, t) < 1e-8) { return basis(n); }
  t = normalize(t);
  return mat3x3f(t, cross(n, t), n);
}
fn aniso_alpha(a: f32, k: f32) -> vec2f {
  return vec2f(max(a * (1.0 + k), 0.002), max(a * (1.0 - k), 0.002));
}
fn aniso_d(h: vec3f, ax: vec2f) -> f32 {
  let q = h.x * h.x / (ax.x * ax.x) + h.y * h.y / (ax.y * ax.y) + h.z * h.z;
  return 1.0 / (PI * ax.x * ax.y * q * q);
}
fn aniso_lambda(v: vec3f, ax: vec2f) -> f32 {
  let z2 = max(v.z * v.z, 1e-8);
  return 0.5 * (-1.0 + sqrt(1.0 + (ax.x * ax.x * v.x * v.x + ax.y * ax.y * v.y * v.y) / z2));
}
fn aniso_g1(v: vec3f, ax: vec2f) -> f32 { return 1.0 / (1.0 + aniso_lambda(v, ax)); }
fn aniso_dg(fr: mat3x3f, v: vec3f, l: vec3f, h: vec3f, a: f32, k: f32) -> f32 {
  let ax = aniso_alpha(a, k);
  let vl = transpose(fr) * v;
  let ll = transpose(fr) * l;
  let hl = transpose(fr) * h;
  return aniso_d(hl, ax) / (1.0 + aniso_lambda(vl, ax) + aniso_lambda(ll, ax));
}
fn sample_vndf2(v: vec3f, ax: vec2f, u: vec2f) -> vec3f {
  let vs = normalize(vec3f(v.x * ax.x, v.y * ax.y, v.z));
  let phi = 2.0 * PI * u.x;
  let z = (1.0 - u.y) * (1.0 + vs.z) - vs.z;
  let st = sqrt(clamp(1.0 - z * z, 0.0, 1.0));
  let h = vec3f(st * cos(phi), st * sin(phi), z) + vs;
  return normalize(vec3f(h.x * ax.x, h.y * ax.y, max(h.z, 1e-6)));
}

fn cosine_dir(n: vec3f, u: vec2f) -> vec3f {
  let r = sqrt(u.x);
  let phi = 2.0 * PI * u.y;
  return basis(n) * vec3f(r * cos(phi), r * sin(phi), sqrt(max(0.0, 1.0 - u.x)));
}

// One light sample with MIS against the BSDF.
fn sample_lights(p: vec3f, n: vec3f, v: vec3f, m: Material, w: Lobes, floor_hit: bool) -> vec3f {
  let samples=light_sample_count();
  var result=vec3f(0.);
  for(var i=0u;i<samples;i++){
    var li=i;if(samples==1u){li=choose_light(p);}
    result+=sample_one_light(li,p,n,v,m,w,floor_hit);
  }
  return result;
}
fn sample_one_light(li:u32,p: vec3f, n: vec3f, v: vec3f, m: Material, w: Lobes, floor_hit: bool) -> vec3f {
  let l = light_at(li);
  let s = sample_light(l, p);
  if (s.pdf <= 0.0 || all(s.radiance <= vec3f(0.0))) { return vec3f(0.0); }
  var f = vec3f(0.0);
  var pb = 0.0;
  if (floor_hit) {
    let nl = dot(n, s.dir);
    if (nl <= 0.0) { return vec3f(0.0); }
    f = vec3f(P.env_horizon.w * nl / PI);
    pb = nl / PI;
  } else {
    f = eval_reflect(m, n, v, s.dir);
    pb = pdf_reflect(m, w, n, v, s.dir);
  }
  if (all(f <= vec3f(0.0))) { return vec3f(0.0); }
  // With shadows off (the environment's switch), every light reaches every surface.
  var t = vec3f(1.0);
  if (P.opts.y == 0u && l.u.w >= 0.0 && (floor_hit || (u32(m.policy.y) & 2u) != 0u)) { t = transmittance(p, s.dir, s.dist); }
  if (all(t <= vec3f(0.0))) { return vec3f(0.0); }
  let pl = s.pdf * light_selection_pdf(li,p);
  let mis = pl * pl / (pl * pl + pb * pb);
  return f * t * s.radiance * mis / pl;
}

fn sample_environment(p:vec3f,n:vec3f,v:vec3f,m:Material,w:Lobes)->vec3f {
  if(map_count()==0u){return vec3f(0.);}
  let d=sample_environment_direction();let pe=environment_pdf(d);
  let f=eval_reflect(m,n,v,d);let pb=pdf_reflect(m,w,n,v,d);
  if(pe<=0. || all(f<=vec3f(0.))){return vec3f(0.);}
  var tr=vec3f(1.);
  if(P.opts.y==0u && (u32(m.policy.y)&2u)!=0u){tr=transmittance(p,d,BIG);}
  return f*tr*environment(d)*(pe/(pe*pe+pb*pb));
}

// Shadow catcher: how much of the floor's light at `p` the objects leave.
fn catcher_ratio(p: vec3f) -> f32 {
  var free = 0.0;
  var lit = 0.0;
  let n = vec3f(0.0, 1.0, 0.0);
  let live = P.opts.x == 1u;
  // Live: one light, picked at random, stands in for all of them.
  var first = 0u;
  var last = P.counts.x;
  if (live && P.counts.x > 0u) { first = min(u32(rnd() * f32(P.counts.x)), P.counts.x - 1u); last = first + 1u; }
  for (var i = first; i < last; i++) {
    let light = light_at(i);
    let s = sample_light(light, p);
    if (s.pdf <= 0.0) { continue; }
    let e = max(dot(n, s.dir), 0.0) * dot(s.radiance, vec3f(0.333)) / s.pdf;
    free += e;
    if (e > 0.0) { lit += e * select(1.0, dot(transmittance(p, s.dir, s.dist), vec3f(0.333)), P.opts.y == 0u && light.u.w >= 0.0); }
  }
  if (live) { free *= f32(P.counts.x); lit *= f32(P.counts.x); }
  // Cosine-weighted dome samples: soft contact occlusion.
  let dome = select(2, 1, live);
  for (var k = 0; k < dome; k++) {
    let dir = cosine_dir(n, rnd2());
    var e = 2.4 / f32(dome);
    if(map_count()>0u){e=PI*dot(environment(dir),vec3f(0.2126,0.7152,0.0722))/f32(dome);}
    free += e;
    lit += e * select(1.0, ambient_visibility(p, dir, 4000.0), P.opts.y == 0u);
  }
  if (free <= 0.0) { return 1.0; }
  // Focused light on the studio floor brightens it past the backdrop.
  if (P.caustic.z > 0.5 && P.opts.x == 0u) { lit += dot(caustic_at(p, vec3f(0.,1.,0.)), vec3f(0.333)); }
  return lit / free;
}

fn decode_srgb(c: vec3f) -> vec3f {
  return select(pow((c + 0.055) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}

fn map_texel(layer: f32, uv: vec2f) -> vec4f {
  return textureSampleLevel(maps, map_samp, fract(uv), i32(layer), 0.0);
}

struct Surface { p: vec3f, n: vec3f, ng: vec3f, uv: vec2f, m: Material, object: u32 }

// The hit's position, normals and material with its texture maps applied.
fn surface(ro: vec3f, rd: vec3f, h: Hit) -> Surface {
  let i = u32(h.tri);
  let t = tris[i];
  let w = 1.0 - h.b.x - h.b.y;
  let p = ro + rd * h.t;
  let p0 = pos[3u * i].xyz;
  // Geometry is stored in its model's space; normals go to world space.
  let ng = normal_to_world(h.inst, cross(pos[3u * i + 1u].xyz - p0, pos[3u * i + 2u].xyz - p0));
  var n = normal_to_world(h.inst, t.n0.xyz * w + t.n1.xyz * h.b.x + t.n2.xyz * h.b.y);
  if (dot(n, ng) < 0.0) { n = -n; } // normals and winding may disagree
  let uv = t.uv01.xy * w + t.uv01.zw * h.b.x + t.uv2m.xy * h.b.y;
  var m = materials[u32(t.uv2m.z)];
  if (m.maps.x >= 0.0) {
    let c = map_texel(m.maps.x, uv);
    m.base = vec4f(m.base.xyz * decode_srgb(c.rgb), m.base.w);
    m.extra.z *= c.a;
  }
  if (m.maps.y >= 0.0) { m.emit = vec4f(m.emit.xyz * decode_srgb(map_texel(m.maps.y, uv).rgb), m.emit.w); }
  if (m.maps.z >= 0.0 && m.maps2.x < 0.0 && m.maps2.y < 0.0) {
    let orm = map_texel(m.maps.z, uv);
    m.spec.x *= orm.g;
    m.base.w *= orm.b;
  }
  if (m.maps2.x >= 0.0) { m.spec.x *= map_texel(m.maps2.x, uv).g; }
  if (m.maps2.y >= 0.0) { m.base.w *= map_texel(m.maps2.y, uv).b; }
  if (m.maps2.z >= 0.0) { m.policy.x *= map_texel(m.maps2.z, uv).r; }
  if ((u32(m.policy.y) & 16u) != 0u) {
    let c = unpack4x8unorm(bitcast<u32>(t.n0.w)) * w + unpack4x8unorm(bitcast<u32>(t.n1.w)) * h.b.x + unpack4x8unorm(bitcast<u32>(t.n2.w)) * h.b.y;
    m.base = vec4f(m.base.xyz * c.rgb, m.base.w); m.extra.z *= c.a;
    if ((u32(m.policy.y) & 8u) != 0u) { m.emit = vec4f(m.emit.xyz*c.rgb,m.emit.w); }
  }
  let instance_base=P.counts.x*4u+h.inst*9u;
  if((u32(max(sdata[instance_base+4u].w,0.))&1u)!=0u){
    let color=vec3f(sdata[instance_base+6u].w,sdata[instance_base+7u].w,sdata[instance_base+8u].w);
    m.base=vec4f(m.base.xyz*color,m.base.w);
    if((u32(m.policy.y)&8u)!=0u){m.emit=vec4f(m.emit.xyz*color,m.emit.w);}
  }
  if (m.extra.z < m.policy.z) { m.extra.z = 0.0; }
  let tangent = vector_to_world(h.inst, t.tangent.xyz);
  if (m.maps.w >= 0.0 && dot(tangent, tangent) > 0.0) {
    let texnormal = map_texel(m.maps.w, uv).xyz * 2.0 - 1.0;
    let tn = vec3f(texnormal.xy * m.maps2.w, texnormal.z);
    let tg = normalize(tangent - n * dot(n, tangent));
    let bt = cross(n, tg) * t.tangent.w;
    let mapped = normalize(tg * tn.x + bt * tn.y + n * tn.z);
    if (dot(mapped, ng) > 0.0) { n = mapped; }
  }
  if (m.layers.w > 0.0) { n = droplets(n, uv, m.layers.w); }
  m.spec.x = max(m.spec.x, 0.002);
  let layer_override=floor(max(sdata[instance_base+4u].w,0.)*0.5);
  return Surface(p, n, ng, uv, m, select(u32(t.uv2m.w),u32(max(layer_override-1.,0.)),layer_override>0.));
}

fn hash2(c: vec2f) -> vec2f {
  let h = vec2u(pcg((bitcast<u32>(c.x) * 1597334677u) ^ pcg(bitcast<u32>(c.y))), pcg((bitcast<u32>(c.y) * 3812015801u) ^ bitcast<u32>(c.x)));
  return vec2f(h) * (1.0 / 4294967296.0);
}

// Water droplets on a surface: beads in a jittered grid over UV space. Inside
// a bead the normal tilts like a lens, so it bends reflections and the content
// behind it. `amount` sets how many cells carry a bead and how large they are.
fn droplets(n: vec3f, uv: vec2f, amount: f32) -> vec3f {
  let scale = 36.0;
  let q = uv * scale;
  let cell = floor(q);
  var best = vec3f(0.0, 0.0, 1e9); // offset xy, distance²
  var radius = 0.0;
  for (var j = -1; j <= 1; j++) {
    for (var i = -1; i <= 1; i++) {
      let c = cell + vec2f(f32(i), f32(j));
      let h = hash2(c);
      if (h.x > amount) { continue; }
      let centre = c + 0.2 + 0.6 * hash2(c + 17.0);
      let r = 0.18 + 0.32 * h.y * amount;
      let d = q - centre;
      let d2 = dot(d, d);
      if (d2 < r * r && d2 < best.z) { best = vec3f(d / r, d2); radius = r; }
    }
  }
  if (radius <= 0.0) { return n; }
  // A spherical cap: the slope grows toward the bead's rim.
  let off = best.xy;
  let h = sqrt(max(1.0 - dot(off, off), 0.02));
  let tb = basis(n);
  return normalize(tb * vec3f(off.x * 0.9, off.y * 0.9, h));
}

// Radiance arriving along a mirror direction: a light, the dome, the floor
// or another object (its emission plus one light sample).
fn mirror_radiance(p: vec3f, d: vec3f) -> vec3f {
  let h = intersect(p, d, BIG, false);
  let tf = floor_t(p, d);
  let light_hit=closest_light(p,d,min(h.t,tf));
  if(light_hit.t<h.t && light_hit.t<tf){return light_emit(light_at(light_hit.light),p+d*light_hit.t,d);}
  if (h.tri < 0 && tf >= BIG) { return environment(d) + sun_seen(d); }
  if (tf < h.t) {
    let q = p + d * tf;
    let n = vec3f(0.0, 1.0, 0.0);
    let dummy = Material(vec4f(0.0), vec4f(1.0, 0.0, 1.0, 0.0), vec4f(0.0), vec4f(1.5, 1.0, 1.0, 0.0), vec4f(-1.0), vec4f(0.0), vec4f(1.0, 7.0, 0.0, 0.0), vec4f(-1.0, -1.0, -1.0, 1.0));
    return sample_lights(q + n * ray_epsilon(), n, -d, dummy, Lobes(0.0, 0.0, 1.0, 0.0), true);
  }
  let sf = surface(p, d, h);
  var n = sf.n;
  var ng = sf.ng;
  if (dot(d, ng) > 0.0) { n = -n; ng = -ng; }
  let nv = max(dot(n, -d), 1e-4);
  shade_t = vector_to_world(h.inst, tris[u32(h.tri)].tangent.xyz);
  return sf.m.emit.xyz + sample_lights(sf.p + ng * ray_epsilon(), n, -d, sf.m, lobe_probs(sf.m, nv), false);
}

// The sun disc as seen along an escaping ray (no MIS: used where light
// sampling cannot reach, such as mirror reflections).
fn sun_contribution(i:u32,p:vec3f,d:vec3f,bsdf_pdf:f32)->vec3f {
  let l=light_at(i);
  if(light_kind(l)!=3u || dot(d,normalize(l.pos.xyz))<l.u.x){return vec3f(0.);}
  var mis=1.;if(bsdf_pdf>0.){let pl=light_pdf(l,p,d,BIG)*light_selection_pdf(i,p);mis=bsdf_pdf*bsdf_pdf/(bsdf_pdf*bsdf_pdf+pl*pl);}
  return l.radiance.xyz*mis;
}
fn sun_radiance(p:vec3f,d:vec3f,bsdf_pdf:f32)->vec3f {
  var e=vec3f(0.);
  if(!world_on() || world_row(4u).w<=0.){for(var i=0u;i<P.counts.x;i++){e+=sun_contribution(i,p,d,bsdf_pdf);}return e;}
  let base=u32(world_row(4u).w);var stack:array<u32,32>;var size=1u;stack[0]=0u;
  while(size>0u){size-=1u;let index=stack[size];if(world_row(base+3u*index+2u).y<=0.){continue;}
    let a=world_row(base+3u*index);let b=world_row(base+3u*index+1u);
    if(b.w<0.){e+=sun_contribution(u32(a.w),p,d,bsdf_pdf);}else{stack[size]=u32(a.w);stack[size+1u]=u32(b.w);size+=2u;}}
  return e;
}
fn sun_seen(d:vec3f)->vec3f {return sun_radiance(vec3f(0.),d,0.);}

struct Sample { radiance: vec3f, bg: vec3f, normal: vec3f, depth: f32, void_hit: f32, emit: vec3f, object: u32 }

fn camera_ray(px: vec2f) -> array<vec3f, 2> {
  let dims = vec2f(f32(P.dims.x), f32(P.dims.y));
  let ndc = (px / dims) * 2.0 - 1.0;
  let fwd = P.cam_fwd.xyz;
  let d = normalize(fwd + P.cam_right.xyz * ndc.x * P.cam_right.w - P.cam_up.xyz * ndc.y * P.cam_up.w);
  var ro = P.cam_pos.xyz;
  var rd = d;
  if ((P.opts.w & 1u) != 0u) {
    ro += P.cam_right.xyz * ndc.x * P.cam_right.w - P.cam_up.xyz * ndc.y * P.cam_up.w;
    rd = fwd;
  }
  // Live uses a pinhole: depth of field appears as the view refines.
  if (P.cam_pos.w > 0.0 && P.opts.x == 0u) {
    let focus = ro + d * (P.cam_fwd.w / dot(d, fwd));
    let u = rnd2();
    let r = sqrt(u.x) * P.cam_pos.w;
    let a = 2.0 * PI * u.y;
    ro = ro + P.cam_right.xyz * r * cos(a) + P.cam_up.xyz * r * sin(a);
    rd = normalize(focus - ro);
  }
  if (camera_clip_on()) { ro += rd * max(0.0, camera_near() - dot(ro - P.cam_pos.xyz, fwd)) / max(dot(rd, fwd), 1e-6); }
  return array<vec3f, 2>(ro, rd);
}

fn path(px: vec2f) -> Sample {
  var s = Sample(vec3f(0.0), vec3f(0.0), vec3f(0.0), -1.0, 0.0, vec3f(0.0), 0u);
  let ray = camera_ray(px);
  var ro = ray[0];
  var rd = ray[1];
  var tp = vec3f(1.0);
  var ambient_response = 1.0;
  var last_pdf = 0.0;   // BSDF pdf of the last bounce (0: camera or delta)
  var primary = true;   // still on the camera ray (passes through cut-outs keep it)
  var band = -1;        // dispersion: the RGB channel this path carries (-1: all)
  // Caustics: 1 after a matte bounce, 2 once glass or a mirror follows it.
  // Photons already deliver that light, so the path must not count it twice.
  var chain = 0;
  let photons_on = P.caustic.z > 0.5 && P.opts.x == 0u;
  let max_b = P.counts.y;
  for (var b = 0u; b <= max_b + 2u; b++) {
    if (b > max_b && !primary) { break; }
    var h:Hit;
    if(primary){h=primary_intersect(ro,rd,camera_limit(ro,rd));}else{h=intersect(ro,rd,BIG,false);}
    let tf = floor_t(ro, rd);
    // Finite emitters terminate a secondary segment at the closest light.
    var light_distance=BIG;var seen_light=0u;
    if(!primary){let light_hit=closest_light(ro,rd,min(h.t,tf));light_distance=light_hit.t;seen_light=light_hit.light;}
    let segment=min(min(h.t,tf),light_distance);
    if(medium_majorant()>0.){
      if(P.opts.x!=0u){
        let end=tracking_segment(ro,rd,segment).y;let dt=end/2.;
        if(primary){
          for(var step=0u;step<2u;step++){
            let point=ro+rd*((f32(step)+rnd())*dt);let med=medium_at(point);
            let tr=exp(-live_optical_depth(ro+rd*(f32(step)*dt),rd,dt));
            s.radiance+=tp*med.albedo*(1.-tr)*medium_direct_live(point,rd,med.g);tp*=tr;
          }
        }else{tp*=exp(-live_optical_depth(ro,rd,end));}
      }else{
        let event=medium_event(ro,rd,segment);
        if(event>=0.){
          let point=ro+rd*event;let med=sample_medium(point);tp*=med.albedo;
          s.radiance+=tp*medium_direct(point,rd,med.g);
          // Phase-sampled direct emitters are not added again: NEE above handles
          // those, while following segments still carry indirect surface light.
          ro=point;rd=sample_hg(rd,med.g);primary=false;last_pdf=0.;chain=3;
          if(b>=3u){let q=clamp(max(tp.x,max(tp.y,tp.z)),0.05,0.95);if(rnd()>q){return s;}tp/=q;}
          continue;
        }
      }
    }
    if(light_distance<h.t && light_distance<tf){
      if(chain==2 || chain==3){return s;}
      let light=light_at(seen_light);var mis=1.;
      if(last_pdf>0.){let pl=light_pdf(light,ro,rd,light_distance)*light_selection_pdf(seen_light,ro);mis=last_pdf*last_pdf/(last_pdf*last_pdf+pl*pl);}
      s.radiance+=tp*light_emit(light,ro+rd*light_distance,rd)*mis;return s;
    }
    if (h.tri < 0 && tf >= BIG) {
      if (primary) {
        if(map_count()>0u && world_row(1u).z>0.5){s.radiance+=tp*(environment(rd)+sun_seen(rd));}
        else{let transmission=clamp(max(tp.x,max(tp.y,tp.z)),0.,1.);s.bg = P.backdrop.xyz*transmission;s.void_hit = transmission;}
      } else {
        let pe=environment_pdf(rd);
        var mis=1.;if(last_pdf>0. && pe>0.){mis=last_pdf*last_pdf/(last_pdf*last_pdf+pe*pe);}
        if(chain!=3){s.radiance += tp * environment(rd) * ambient_response * mis;}
        // The infinite-light branches avoid scanning every practical on misses.
        if(chain!=2 && chain!=3){s.radiance+=tp*sun_radiance(ro,rd,last_pdf);}
      }
      return s;
    }
    if (tf < h.t) {
      let p = ro + rd * tf;
      if (primary) {
        let transmission=clamp(max(tp.x,max(tp.y,tp.z)),0.,1.);
        s.bg = P.backdrop.xyz * catcher_ratio(p)*transmission;
        s.void_hit = transmission;
        return s;
      }
      // A matte floor in reflections: one diffuse bounce, lit by the lights.
      let n = vec3f(0.0, 1.0, 0.0);
      let dummy = Material(vec4f(0.0), vec4f(1.0, 0.0, 1.0, 0.0), vec4f(0.0), vec4f(1.5, 1.0, 1.0, 0.0), vec4f(-1.0), vec4f(0.0), vec4f(1.0, 7.0, 0.0, 0.0), vec4f(-1.0, -1.0, -1.0, 1.0));
      s.radiance += tp * sample_lights(p + n * ray_epsilon(), n, -rd, dummy, Lobes(0.0, 0.0, 1.0, 0.0), true);
      if (photons_on) { s.radiance += tp * P.env_horizon.w / PI * caustic_at(p, vec3f(0.,1.,0.)); }
      tp *= P.env_horizon.w;
      ro = p + n * ray_epsilon();
      rd = cosine_dir(n, rnd2());
      chain = select(0, 1, photons_on);
      last_pdf = dot(n, rd) / PI;
      continue;
    }
    let sf = surface(ro, rd, h);
    var m = sf.m;
    shade_t = vector_to_world(h.inst, tris[u32(h.tri)].tangent.xyz);
    // Partly transparent surfaces: pass straight through with 1 − opacity.
    if (m.extra.z < 0.999 && rnd() >= m.extra.z) {
      ro = sf.p + rd * ray_epsilon();
      continue;
    }
    let front = dot(rd, sf.ng) < 0.0;
    if (!front && (u32(m.policy.y) & 4u) == 0u) { ro = sf.p + rd * ray_epsilon(); continue; }
    ambient_response = m.policy.x;
    var n = sf.n;
    var ng = sf.ng;
    if (!front) { n = -n; ng = -ng; }
    if (primary) { s.normal = n; s.depth = select(distance(sf.p,P.cam_pos.xyz), dot(sf.p-P.cam_pos.xyz,P.cam_fwd.xyz), (P.opts.w & 1u)!=0u); s.object = sf.object + 1u; }
    let v = -rd;
    var nv = max(dot(n, v), 1e-4);
    // Emission originates below the coat and loses its reflected share on exit.
    if (any(m.emit.xyz > vec3f(0.0)) && (front || (u32(m.policy.y) & 4u) != 0u)) {
      let emitted = m.emit.xyz * (1.0 - m.spec.y * fresnel_dielectric(nv, 1.5));
      if (primary) { s.emit += tp * emitted; } else { s.radiance += tp * emitted; }
    }
    if ((u32(m.policy.y) & 8u) != 0u) { return s; }
    let p = sf.p + ng * ray_epsilon();
    // Live splits near-mirror coats to keep highlights stable at one sample.
    // Final/Refine follow the full coat BSDF: the shortcut only follows one
    // reflection bounce and omits the incoming coat attenuation of the base.
    if (m.spec.y > 0.0 && m.spec.z < 0.05 && front && primary && P.opts.x != 0u) {
      let fc = m.spec.y * fresnel_dielectric(nv, 1.5);
      s.radiance += tp * fc * mirror_radiance(p, reflect(-v, n));
      tp *= 1.0 - fc;
      m.spec.y = 0.0;
    }
    primary = false;
    let w = lobe_probs(m, nv);
    // Direct light.
    if (w.trans < 0.999) { s.radiance += tp * (sample_lights(p, n, v, m, w, false)+sample_environment(p,n,v,m,w)); }
    // Caustics: light focused onto this surface by glass or mirrors.
    if (photons_on && w.diff > 0.0) {
      s.radiance += tp * m.base.xyz * (1.0 - m.base.w) * (1.0 - m.spec.w) / PI * caustic_at(sf.p, n);
    }
    // Russian roulette after the third bounce.
    if (b >= 3u) {
      let q = clamp(max(tp.x, max(tp.y, tp.z)), 0.05, 0.95);
      if (rnd() > q) { return s; }
      tp /= q;
    }
    let pick = rnd();
    if (pick < w.trans) {
      // This lobe is the glass share of the surface (under any coat), chosen with probability w.trans.
      tp *= (1.0 - m.spec.y * fresnel_dielectric(nv, 1.5)) * m.spec.w * (1.0 - m.base.w) / w.trans;
      // Rough dielectric transmission/reflection through a sampled microfacet.
      let a = max(m.spec.x * m.spec.x, 0.0005);
      let tb = basis(n);
      let hm = tb * sample_vndf(transpose(tb) * v, a, rnd2());
      // Dispersion: blue bends more than red. The first refraction picks one
      // channel for the rest of the path (weighted ×3 so the average holds).
      var ior = m.extra.x;
      if (m.layers.z > 0.0) {
        if (band < 0) {
          band = spectral_band;
          var pick_w = vec3f(0.0);
          pick_w[band] = 3.0;
          tp *= pick_w;
        }
        ior += m.layers.z * 0.04 * (f32(band) - 1.0);
      }
      let eta = select(1.0 / ior, ior, front);
      let F = fresnel_dielectric(abs(dot(v, hm)), eta);
      if (rnd() < F) {
        rd = reflect(-v, hm);
        ro = sf.p + ng * ray_epsilon();
      } else {
        let r = refract(-v, hm, 1.0 / eta);
        if (dot(r, r) < 0.5) { rd = reflect(-v, hm); ro = sf.p + ng * ray_epsilon(); }
        else { rd = r; ro = sf.p - ng * ray_epsilon(); tp *= m.base.xyz; }
      }
      if (chain > 0) { chain = 2; }
      last_pdf = 0.0;
      continue;
    }
    var l = vec3f(0.0);
    let r = (pick - w.trans) / max(1.0 - w.trans, 1e-6);
    let wr = 1.0 - w.trans;
    if (r < w.coat / wr) {
      let tb = basis(n);
      l = reflect(-v, tb * sample_vndf(transpose(tb) * v, m.spec.z * m.spec.z, rnd2()));
    } else if (r < (w.coat + w.spec) / wr) {
      if (m.extra.w != 0.0) {
        let fr = aniso_frame(n);
        l = reflect(-v, fr * sample_vndf2(transpose(fr) * v, aniso_alpha(m.spec.x * m.spec.x, m.extra.w), rnd2()));
      } else {
        let tb = basis(n);
        l = reflect(-v, tb * sample_vndf(transpose(tb) * v, m.spec.x * m.spec.x, rnd2()));
      }
      // Polished metal continues a caustic chain; anything else ends it.
      if (chain > 0 && m.base.w > 0.5 && m.spec.x < 0.25) { chain = 2; } else { chain = 0; }
    } else if (m.emit.w > 0.0) {
      // Subsurface: a random walk inside the object; the path leaves where it exits.
      let walk = random_walk(sf.p - ng * ray_epsilon(), cosine_dir(-n, rnd2()), m);
      if (walk.w <= 0.0) { return s; }
      chain = 0;
      tp *= m.base.xyz;
      ro = walk.xyz;
      rd = cosine_dir(exit_n, rnd2());
      ro = ro + exit_n * ray_epsilon();
      last_pdf = max(dot(exit_n, rd), 1e-4) / PI;
      continue;
    } else {
      l = cosine_dir(n, rnd2());
      chain = select(0, 1, photons_on);
    }
    if (r < w.coat / wr) { chain = 0; }
    if (dot(l, ng) <= 0.0) { return s; }
    let f = eval_reflect(m, n, v, l);
    let pdf = pdf_reflect(m, w, n, v, l);
    if (pdf <= 1e-8) { return s; }
    tp *= f / pdf;
    last_pdf = pdf;
    ro = p;
    rd = l;
  }
  return s;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3u) {
  let w = P.dims.x;
  let h = P.dims.y;
  if (gid.x >= w || gid.y >= h) { return; }
  let idx = gid.y * w + gid.x;
  if(P.opts.x==0u && P.counts.z==0u && world_on()){
    let settings=world_row(5u);let a=accum[idx];
    if(settings.x>0.5 && a.w>=settings.y){
      let mean=dot(a.xyz+ebuf[idx].xyz,vec3f(0.2126,0.7152,0.0722))/a.w;
      let variance=max(0.,(ebuf[idx].w-a.w*mean*mean)/max(a.w-1.,1.));
      let error=sqrt(variance/a.w);let limit=max(settings.w,abs(mean)*settings.z);
      if(error<limit){return;}
    }
  }
  seed = pcg(idx ^ pcg(P.dims.z * 9781u + 1u));
  var rad = vec3f(0.0);
  var bg = vec3f(0.0);
  var nbg = 0.0;
  var em = vec3f(0.0);
  // Σ luminance² (radiance + emission) of the samples: each pixel's noise, for Final's denoiser.
  var l2 = 0.0;
  var nd = vec4f(0.0, 0.0, 0.0, -1.0);
  var object = 0u;
  for (var i = 0u; i < P.dims.w; i++) {
    var jitter = rnd2();
    // Four-sample batches cover all pixel quadrants. Thin artwork and live
    // texture edges must not disappear because every ray landed on one side.
    if (P.dims.w == 4u) { jitter = (vec2f(f32(i & 1u), f32((i >> 1u) & 1u)) + jitter) * 0.5; }
    // Exactly one of each RGB wavelength per three paths. Each stratum gets
    // an independent random cyclic rotation, preserving each path's uniform
    // marginal while removing channel-count noise in dispersive highlights.
    let path_index=(P.dims.z-1u)*P.dims.w+i;
    let rotation=pcg(idx ^ pcg(path_index/3u+0xa511e9b3u))%3u;
    spectral_band=i32((path_index%3u+rotation)%3u);
    var smp = path(vec2f(f32(gid.x), f32(gid.y)) + jitter);
    // A non-finite sample would spread through the denoiser; drop it.
    if (any(smp.radiance != smp.radiance) || any(abs(smp.radiance) > vec3f(1e20))) { smp.radiance = vec3f(0.0); }
    // Clamp fireflies from tiny bright paths (Live clamps harder).
    let lum = dot(smp.radiance, vec3f(0.2126, 0.7152, 0.0722));
    rad += smp.radiance * min(1.0, f32(P.counts.w) / max(lum, 1e-6));
    let x = min(lum, f32(P.counts.w)) + dot(smp.emit, vec3f(0.2126, 0.7152, 0.0722));
    l2 += x * x;
    em += smp.emit;
    bg += smp.bg;
    nbg += smp.void_hit;
    if (i == 0u && object == 0u) { nd = vec4f(smp.normal, smp.depth); }
    if (smp.object != 0u && (object == 0u || smp.depth < nd.w)) { nd = vec4f(smp.normal, smp.depth); object = smp.object; }
  }
  let n = f32(P.dims.w);
  if (P.counts.z == 1u) {
    accum[idx] = vec4f(rad, n);
    bgsum[idx] = vec4f(bg, nbg);
    gbuf[idx] = vec4f(oct_encode(nd.xyz), f32(object), nd.w);
    ebuf[idx] = vec4f(em, l2);
  } else {
    accum[idx] += vec4f(rad, n);
    bgsum[idx] += vec4f(bg, nbg);
    ebuf[idx] += vec4f(em, l2);
    if (object != 0u && (gbuf[idx].z == 0.0 || nd.w < gbuf[idx].w)) { gbuf[idx] = vec4f(oct_encode(nd.xyz), f32(object), nd.w); }
  }
}

// ── Normals packed in two floats (octahedral) ──
fn oct_encode(n: vec3f) -> vec2f {
  if (dot(n, n) < 1e-12) { return vec2f(0.0); }
  let a = n / (abs(n.x) + abs(n.y) + abs(n.z));
  if (a.z >= 0.0) { return a.xy; }
  return (1.0 - abs(a.yx)) * select(vec2f(-1.0), vec2f(1.0), a.xy >= vec2f(0.0));
}

// ── Caustics: photons from the lights through glass and mirrors ──
// Level 0: fine cells for sharp caustics; level 1: 4× larger, for sparse light.
fn cell_size(level: i32) -> f32 { return P.caustic.x * select(1.0, 4.0, level == 1); }
fn cell_of(p: vec3f, level: i32) -> vec3i { return vec3i(floor(p / cell_size(level))); }
fn cell_hash(c: vec3i, level: i32) -> u32 {
  return pcg((bitcast<u32>(c.x) * 73856093u) ^ pcg((bitcast<u32>(c.y) * 19349663u) ^ pcg(bitcast<u32>(c.z) * 83492791u + u32(level))));
}
// The cell's slot: cells that hash to the same slot probe onward instead of
// sharing light. `claim` takes a free slot.
fn cell_slot(c: vec3i, level: i32, claim: bool) -> i32 {
  let h = cell_hash(c, level);
  let key = pcg(h ^ 0x9e3779b9u) | 1u;
  for (var k = 0u; k < 12u; k++) {
    let i = ((h + k * k) % GRID_CELLS) * 8u;
    let cur = atomicLoad(&grid[i + 3u]);
    if (cur == key) { return i32(i); }
    if (cur == 0u) {
      if (!claim) { return -1; }
      let r = atomicCompareExchangeWeak(&grid[i + 3u], 0u, key);
      if (r.exchanged || r.old_value == key) { return i32(i); }
    }
  }
  return -1;
}
fn add_f32(i: u32, v: f32) {
  var old = atomicLoad(&grid[i]);
  loop {
    let next = bitcast<u32>(bitcast<f32>(old) + v);
    let r = atomicCompareExchangeWeak(&grid[i], old, next);
    if (r.exchanged) { break; }
    old = r.old_value;
  }
}
fn deposit(p: vec3f, flux: vec3f) {
  for (var level = 0; level < 2; level++) {
    let s = cell_slot(cell_of(p, level), level, true);
    if (s < 0) { continue; }
    let i = u32(s);
    add_f32(i, flux.x); add_f32(i + 1u, flux.y); add_f32(i + 2u, flux.z);
    atomicAdd(&grid[i + 4u], 1u);
    let local = p / cell_size(level) - (vec3f(cell_of(p,level)) + 0.5);
    add_f32(i + 5u, local.x); add_f32(i + 6u, local.y); add_f32(i + 7u, local.z);
  }
}
// Surface density uses a bilinear tent in the receiver's dominant-axis
// projection. Each projected bin represents size² / |n_major| surface area.
// Unlike a normalized 3D stencil, this partitions unity on a flat receiver:
// translating the grid or tilting the surface cannot absorb photon energy.
// The stored centroid rejects deposits on a nearby, separate parallel surface.
fn gather(p: vec3f, n: vec3f, level: i32) -> vec4f {
  let size = cell_size(level);
  let c = cell_of(p, level);
  let axis = select(select(2u, 1u, abs(n.y) > abs(n.z)), 0u, abs(n.x) > max(abs(n.y),abs(n.z)));
  let major = max(abs(n[axis]), 1e-6);
  var sum = vec3f(0.0);
  var count = 0.0;
  for (var z = -1; z <= 1; z++) { for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
    let cc = c + vec3i(x,y,z);
    let centre = vec3f(cc) + 0.5;
    var kernel = max(vec3f(0.), vec3f(1.) - abs(centre - p/size));
    kernel[axis] = 1.;
    let w = kernel.x * kernel.y * kernel.z;
    if (w <= 0.) { continue; }
    let slot = cell_slot(cc,level,false);
    if (slot < 0) { continue; }
    let i = u32(slot);
    let photons = f32(atomicLoad(&grid[i+4u]));
    if (photons <= 0.) { continue; }
    let local = vec3f(bitcast<f32>(atomicLoad(&grid[i+5u])),bitcast<f32>(atomicLoad(&grid[i+6u])),bitcast<f32>(atomicLoad(&grid[i+7u]))) / photons;
    if (abs(dot((centre + local)*size-p,n)) > 0.5*size) { continue; }
    sum += w * vec3f(bitcast<f32>(atomicLoad(&grid[i])),bitcast<f32>(atomicLoad(&grid[i+1u])),bitcast<f32>(atomicLoad(&grid[i+2u])));
    count += w * photons;
  } } }
  return vec4f(sum * major / (P.caustic.y * size * size), count);
}
// Caustic irradiance at `p`: the fine level where photons are dense enough
// to be smooth, blending to the coarse level where they are sparse.
fn caustic_at(p: vec3f, n: vec3f) -> vec3f {
  if (P.caustic.y <= 0.0) { return vec3f(0.0); }
  let fine = gather(p, n, 0);
  if (fine.w >= 96.0) { return fine.xyz; }
  let coarse = gather(p, n, 1);
  if (coarse.w < 8.0) { return vec3f(0.0); }
  return mix(coarse.xyz, fine.xyz, fine.w / 96.0);
}

// One photon per invocation. Only paths that pass through glass or bounce off
// polished metal and then land on a matte surface (or the studio floor) are
// kept: those are the caustics a camera path rarely finds.
@compute @workgroup_size(64)
fn photons(@builtin(global_invocation_id) gid: vec3u) {
  if (gid.x >= u32(P.caustic.w) || P.counts.x == 0u) { return; }
  seed = pcg(gid.x ^ pcg(u32(P.caustic.y) * 7919u + P.dims.z * 104729u + 17u));
  let count = P.counts.x;
  let li = min(u32(rnd() * f32(count)), count - 1u);
  let l = light_at(li);
  let kind = light_kind(l);
  var ro = vec3f(0.0);
  var rd = vec3f(0.0, -1.0, 0.0);
  var flux = vec3f(0.0);
  let u = rnd2();
  if (kind == 0u) {
    ro = l.pos.xyz + l.u.xyz * (u.x * 2.0 - 1.0) + l.v.xyz * (u.y * 2.0 - 1.0);
    rd = cosine_dir(rect_normal(l), rnd2());
    flux = light_emit(l, ro, -rd) * abs(l.u.w) * PI;
  } else if (kind == 3u) {
    let dir = normalize(l.pos.xyz);
    let tb = basis(dir);
    let r = sqrt(u.x) * P.bounds.w;
    let a = 2.0 * PI * u.y;
    ro = P.bounds.xyz + dir * P.bounds.w * 2.0 + tb * vec3f(r * cos(a), r * sin(a), 0.0);
    rd = -dir;
    flux = l.radiance.xyz * 2.0 * PI * one_minus_cos(1.0 - l.u.x * l.u.x) * PI * P.bounds.w * P.bounds.w;
  } else {
    let z = 1.0 - 2.0 * u.x;
    let rr = sqrt(max(0.0, 1.0 - z * z));
    rd = vec3f(rr * cos(2.0 * PI * u.y), rr * sin(2.0 * PI * u.y), z);
    ro = l.pos.xyz + rd * (l.u.x * 1.01);
    let intensity = l.radiance.xyz * PI * l.u.x * l.u.x;
    flux = intensity * 4.0 * PI * light_emit(l, ro, -rd) / max(l.radiance.xyz, vec3f(1e-12));
  }
  flux *= f32(count);
  var specular = 0;
  var band = -1;
  for (var b = 0; b < 8; b++) {
    let h = intersect(ro, rd, BIG, false);
    let tf = floor_t(ro, rd);
    if (tf < h.t) {
      if (specular > 0) { deposit(ro + rd * tf, flux); }
      return;
    }
    if (h.tri < 0) { return; }
    let sf = surface(ro, rd, h);
    let m = sf.m;
    let front = dot(rd, sf.ng) < 0.0;
    if (!front && (u32(m.policy.y) & 4u) == 0u) { ro = sf.p + rd * ray_epsilon(); continue; }
    var n = sf.n;
    var ng = sf.ng;
    if (!front) { n = -n; ng = -ng; }
    let v = -rd;
    let tr = m.spec.w * (1.0 - m.base.w);
    if (tr > 0.5) {
      // Glass: refract or reflect through a sampled microfacet, as camera paths do.
      let a = max(m.spec.x * m.spec.x, 0.0005);
      let tb = basis(n);
      let hm = tb * sample_vndf(transpose(tb) * v, a, rnd2());
      var ior = m.extra.x;
      if (m.layers.z > 0.0) {
        if (band < 0) { band = min(i32(rnd() * 3.0), 2); var pw = vec3f(0.0); pw[band] = 3.0; flux *= pw; }
        ior += m.layers.z * 0.04 * (f32(band) - 1.0);
      }
      let eta = select(1.0 / ior, ior, front);
      let F = fresnel_dielectric(abs(dot(v, hm)), eta);
      if (rnd() < F) {
        rd = reflect(-v, hm); ro = sf.p + ng * ray_epsilon();
      } else {
        let r = refract(-v, hm, 1.0 / eta);
        if (dot(r, r) < 0.5) { rd = reflect(-v, hm); ro = sf.p + ng * ray_epsilon(); }
        else { rd = r; ro = sf.p - ng * ray_epsilon(); flux *= m.base.xyz; }
      }
      specular++;
    } else if (m.base.w > 0.5 && m.spec.x < 0.25) {
      // Polished metal: a near-mirror bounce tinted by the metal.
      let tb = basis(n);
      rd = reflect(-v, tb * sample_vndf(transpose(tb) * v, max(m.spec.x * m.spec.x, 0.0005), rnd2()));
      ro = sf.p + ng * ray_epsilon();
      flux *= m.base.xyz;
      specular++;
    } else {
      if (specular > 0) { deposit(sf.p, flux); }
      return;
    }
  }
}
