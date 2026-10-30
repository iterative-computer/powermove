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
  backdrop: vec4f,     // display-referred rgb; w: shadow catcher on (1)
  env_zenith: vec4f,   // w: exposure
  env_horizon: vec4f,  // w: floor albedo for secondary bounces
  dims: vec4u,         // width, height, frame seed, samples this dispatch
  counts: vec4u,       // lights, max bounces, reset (1 = overwrite), firefly clamp (luminance)
  opts: vec4u,         // x: Live; y: environment ground color follows zenith (0) or horizon (1)
  ground: vec4f,       // environment radiance below the horizon; w: floor height
}

// Per-triangle shading data in BVH order (positions live in `pos`).
struct Tri { n0: vec4f, n1: vec4f, n2: vec4f, uv01: vec4f, uv2m: vec4f, tangent: vec4f }
struct Node { lmin: vec3f, left: u32, lmax: vec3f, lcount: u32, rmin: vec3f, right: u32, rmax: vec3f, rcount: u32 }
// base: rgb, metallic · spec: roughness, coat, coat roughness, transmission
// emit: rgb, subsurface radius (0: none) · extra: ior, specular, opacity, anisotropy (-1..1)
// maps: texture-array layers for color, emissive, occlusion-roughness-metal, normal (-1: none)
// layers: sheen, thin-film thickness (nm), dispersion, water droplets
struct Material { base: vec4f, spec: vec4f, emit: vec4f, extra: vec4f, maps: vec4f, layers: vec4f }
// pos.w: kind (0 softbox, 1 sphere, 2 spot, 3 sun).
// softbox: pos = centre, u/v = half extents (u.w: area); emits along u × v.
// sphere/spot: pos = centre, u.x = radius; spot: v.xyz = axis, v.w = cos outer, radiance.w = cos inner.
// sun: pos = direction toward the sun, u.x = cos angular radius.
struct Light { pos: vec4f, u: vec4f, v: vec4f, radiance: vec4f }

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> tris: array<Tri>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> materials: array<Material>;
@group(0) @binding(4) var<storage, read> lights: array<Light>;
@group(0) @binding(5) var maps: texture_2d_array<f32>;
@group(0) @binding(6) var map_samp: sampler;
// rgb: sum of scene radiance (linear, without directly seen emission); w: samples taken
@group(0) @binding(7) var<storage, read_write> accum: array<vec4f>;
// rgb: sum of backdrop display color for samples that saw the void/floor; w: how many
@group(0) @binding(8) var<storage, read_write> bgsum: array<vec4f>;
// xyz: first-hit normal; w: first-hit distance (-1 for the void)
@group(0) @binding(9) var<storage, read_write> gbuf: array<vec4f>;
// rgb: sum of emission seen directly by the camera (a product's screen):
// exact per pixel, so Live denoises only what is noisy.
@group(0) @binding(10) var<storage, read_write> ebuf: array<vec4f>;
// Corner positions, 3 per triangle in BVH order (intersection reads only these).
@group(0) @binding(11) var<storage, read> pos: array<vec4f>;
// First hit's object + 1 (0: the void) — splits one trace into per-layer outputs.
@group(0) @binding(12) var<storage, read_write> idbuf: array<u32>;

const PI: f32 = 3.14159265;
const EPS: f32 = 0.004;
const BIG: f32 = 1e30;

var<private> seed: u32;
// The shading tangent of the surface being lit (anisotropic highlights follow it).
var<private> shade_t: vec3f;
fn pcg(v: u32) -> u32 {
  let s = v * 747796405u + 2891336453u;
  let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}
fn rnd() -> f32 { seed = pcg(seed); return f32(seed) * (1.0 / 4294967296.0); }
fn rnd2() -> vec2f { return vec2f(rnd(), rnd()); }

struct Hit { t: f32, tri: i32, b: vec2f }

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
  if (d <= EPS || d >= best) { return vec3f(BIG, 0.0, 0.0); }
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
      *h = Hit(r.x, i32(first + k), r.yz);
      if (any) { return true; }
    }
  }
  return false;
}

fn intersect(ro: vec3f, rd: vec3f, tmax: f32, any: bool) -> Hit {
  var h = Hit(tmax, -1, vec2f(0.0));
  if (arrayLength(&pos) < 3u) { return h; }
  let inv = 1.0 / rd;
  var stack: array<u32, 40>;
  var sp = 1;
  stack[0] = 0u;
  while (sp > 0) {
    sp -= 1;
    let n = nodes[stack[sp]];
    var tl = BIG;
    var tr = BIG;
    if (n.lcount != 0xffffffffu) { tl = box_t(ro, inv, n.lmin, n.lmax, h.t); }
    if (n.rcount != 0xffffffffu) { tr = box_t(ro, inv, n.rmin, n.rmax, h.t); }
    // Leaves are intersected at once; interior children are pushed far-first.
    if (tl < BIG && n.lcount > 0u) {
      if (leaf(ro, rd, n.left, n.lcount, &h, any)) { return h; }
      tl = BIG;
    }
    if (tr < BIG && n.rcount > 0u) {
      if (leaf(ro, rd, n.right, n.rcount, &h, any)) { return h; }
      tr = BIG;
    }
    if (tl < BIG && tr < BIG) {
      if (tl < tr) {
        stack[sp] = n.right; stack[sp + 1] = n.left;
      } else {
        stack[sp] = n.left; stack[sp + 1] = n.right;
      }
      sp += 2;
    } else if (tl < BIG) {
      stack[sp] = n.left; sp += 1;
    } else if (tr < BIG) {
      stack[sp] = n.right; sp += 1;
    }
  }
  return h;
}

fn tri_material(tri: i32) -> Material { return materials[u32(tris[u32(tri)].uv2m.z)]; }

// How much light gets from `ro` to a point `dist` along `rd`: opaque surfaces
// block it; glass and partly transparent surfaces tint and dim it.
fn transmittance(ro: vec3f, rd: vec3f, dist: f32) -> vec3f {
  var o = ro;
  var left = dist - 2.0 * EPS;
  var t = vec3f(1.0);
  var nearest = false;
  for (var k = 0; k < 6; k++) {
    let h = intersect(o, rd, left, !nearest);
    if (h.tri < 0) { return t; }
    let m = tri_material(h.tri);
    let tr = m.spec.w * (1.0 - m.base.w);
    let through = tr * 0.92 + (1.0 - m.extra.z);
    if (through <= 0.001) { return vec3f(0.0); }
    t *= mix(vec3f(1.0), m.base.xyz, tr) * min(through, 1.0);
    // Continue past this surface; nearest-first from now on so none is skipped.
    nearest = true;
    o = o + rd * (h.t + EPS);
    left -= h.t + EPS;
    if (left <= 0.0) { return t; }
  }
  return t;
}

fn floor_t(ro: vec3f, rd: vec3f) -> f32 {
  let y = ro.y - P.ground.w;
  if (P.backdrop.w < 0.5 || rd.y >= -1e-6 || y <= 0.0) { return BIG; }
  return -y / rd.y;
}

fn environment(d: vec3f) -> vec3f {
  if (d.y < 0.0) { return mix(P.env_horizon.xyz, P.ground.xyz, clamp(-d.y * 4.0, 0.0, 1.0)); }
  let k = clamp(d.y, 0.0, 1.0);
  return mix(P.env_horizon.xyz, P.env_zenith.xyz, k);
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
  if (t <= EPS) { t = -b + s; }
  if (t <= EPS) { return BIG; }
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
    if (t <= EPS) { return BIG; }
    let q = ro + rd * t - l.pos.xyz;
    let uu = dot(q, l.u.xyz) / dot(l.u.xyz, l.u.xyz);
    let vv = dot(q, l.v.xyz) / dot(l.v.xyz, l.v.xyz);
    if (abs(uu) > 1.0 || abs(vv) > 1.0) { return BIG; }
    return t;
  }
  if (kind == 1u || kind == 2u) { return sphere_t(ro, rd, l.pos.xyz, l.u.x); }
  return BIG;
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
    return t * t / (cl * l.u.w);
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
    return LightSample(dir, dist, light_emit(l, q, dir), dist2 / (cl * l.u.w));
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
  return LightSample(dir, dist, light_emit(l, p + dir * dist, dir), 1.0 / (2.0 * PI * omc));
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

fn f0_of(m: Material) -> vec3f { return mix(vec3f(0.04 * m.extra.y), m.base.xyz, m.base.w); }

struct Lobes { coat: f32, spec: f32, diff: f32, trans: f32 }

fn lobe_probs(m: Material, nv: f32) -> Lobes {
  let metallic = m.base.w;
  let tr = m.spec.w * (1.0 - metallic);
  let fc = m.spec.y * schlick1(0.04, nv);
  let fs = schlick(f0_of(m), nv);
  let under = 1.0 - fc;
  var w = Lobes(fc, under * max(dot(fs, vec3f(0.333)), 0.04), under * (1.0 - metallic) * (1.0 - tr) * dot(m.base.xyz, vec3f(0.333)) * (1.0 - fs.x), under * tr);
  let s = w.coat + w.spec + w.diff + w.trans;
  if (s <= 0.0) { return Lobes(0.0, 1.0, 0.0, 0.0); }
  return Lobes(w.coat / s, w.spec / s, w.diff / s, w.trans / s);
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
  let f0 = f0_of(m);
  let F = thin_film(schlick(f0, vh), vh, m.layers.y);
  var spec = F * ggx_d(nh, a) * smith_g2(nv, nl, a) / (4.0 * nv);
  if (m.extra.w != 0.0) {
    let fr = aniso_frame(n);
    spec = F * aniso_dg(fr, v, l, h, a, m.extra.w) / (4.0 * nv);
  }
  let tr = m.spec.w * (1.0 - metallic);
  let diff = m.base.xyz * (1.0 - metallic) * (1.0 - tr) * (1.0 - schlick(f0, nl)) * (1.0 - schlick(f0, nv)) * nl / PI;
  let ca = m.spec.z * m.spec.z;
  let fc = m.spec.y * schlick1(0.04, vh);
  let coat = fc * ggx_d(nh, ca) * smith_g2(nv, nl, ca) / (4.0 * nv);
  let atten = (1.0 - m.spec.y * schlick1(0.04, nv)) * (1.0 - m.spec.y * schlick1(0.04, nl));
  return vec3f(coat) + (spec + diff + sheen(m, nh, nl, nv)) * atten;
}

// Thin-film interference (iridescent coatings): the reflected color cycles
// with the film's optical path, per RGB wavelength.
fn thin_film(F: vec3f, cosv: f32, nm: f32) -> vec3f {
  if (nm <= 0.0) { return F; }
  let n_film = 1.38;
  let s2 = (1.0 - cosv * cosv) / (n_film * n_film);
  let cost = sqrt(max(0.0, 1.0 - s2));
  let path = 2.0 * n_film * nm * cost;
  let phase = 2.0 * PI * path / vec3f(650.0, 532.0, 460.0);
  let interference = 0.5 + 0.5 * cos(phase);
  return clamp(F * (0.35 + 1.3 * interference), vec3f(0.0), vec3f(1.0));
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
      var ng = normalize(cross(pos[3u * i + 1u].xyz - p0, pos[3u * i + 2u].xyz - p0));
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
  let count = P.counts.x;
  if (count == 0u) { return vec3f(0.0); }
  let li = min(u32(rnd() * f32(count)), count - 1u);
  let l = lights[li];
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
    pb = pdf_reflect(m, w, n, v, s.dir) * (1.0 - w.trans);
  }
  if (all(f <= vec3f(0.0))) { return vec3f(0.0); }
  let t = transmittance(p, s.dir, s.dist);
  if (all(t <= vec3f(0.0))) { return vec3f(0.0); }
  let pl = s.pdf / f32(count);
  let mis = pl * pl / (pl * pl + pb * pb);
  return f * t * s.radiance * mis / pl;
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
    let s = sample_light(lights[i], p);
    if (s.pdf <= 0.0) { continue; }
    let e = max(dot(n, s.dir), 0.0) * dot(s.radiance, vec3f(0.333)) / s.pdf;
    free += e;
    if (e > 0.0) { lit += e * dot(transmittance(p, s.dir, s.dist), vec3f(0.333)); }
  }
  if (live) { free *= f32(P.counts.x); lit *= f32(P.counts.x); }
  // Cosine-weighted dome samples: soft contact occlusion.
  let dome = select(2, 1, live);
  for (var k = 0; k < dome; k++) {
    let dir = cosine_dir(n, rnd2());
    let e = 2.4 / f32(dome);
    free += e;
    if (intersect(p, dir, 4000.0, true).tri < 0) { lit += e; }
  }
  if (free <= 0.0) { return 1.0; }
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
  let ng = normalize(cross(pos[3u * i + 1u].xyz - p0, pos[3u * i + 2u].xyz - p0));
  var n = normalize(t.n0.xyz * w + t.n1.xyz * h.b.x + t.n2.xyz * h.b.y);
  if (dot(n, ng) < 0.0) { n = -n; } // normals and winding may disagree
  let uv = t.uv01.xy * w + t.uv01.zw * h.b.x + t.uv2m.xy * h.b.y;
  var m = materials[u32(t.uv2m.z)];
  if (m.maps.x >= 0.0) {
    let c = map_texel(m.maps.x, uv);
    m.base = vec4f(m.base.xyz * decode_srgb(c.rgb), m.base.w);
    m.extra.z *= c.a;
  }
  if (m.maps.y >= 0.0) { m.emit = vec4f(m.emit.xyz * decode_srgb(map_texel(m.maps.y, uv).rgb), m.emit.w); }
  if (m.maps.z >= 0.0) {
    let orm = map_texel(m.maps.z, uv);
    m.spec.x *= orm.g;
    m.base.w *= orm.b;
  }
  if (m.maps.w >= 0.0 && dot(t.tangent.xyz, t.tangent.xyz) > 0.0) {
    let tn = map_texel(m.maps.w, uv).xyz * 2.0 - 1.0;
    let tg = normalize(t.tangent.xyz - n * dot(n, t.tangent.xyz));
    let bt = cross(n, tg) * t.tangent.w;
    let mapped = normalize(tg * tn.x + bt * tn.y + n * tn.z);
    if (dot(mapped, ng) > 0.0) { n = mapped; }
  }
  if (m.layers.w > 0.0) { n = droplets(n, uv, m.layers.w); }
  m.spec.x = max(m.spec.x, 0.002);
  return Surface(p, n, ng, uv, m, u32(t.uv2m.w));
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
  for (var i = 0u; i < P.counts.x; i++) {
    let l = lights[i];
    let tl = hit_light(p, d, l);
    if (tl < h.t && tl < tf) { return light_emit(l, p + d * tl, d); }
  }
  if (h.tri < 0 && tf >= BIG) { return environment(d) + sun_seen(d); }
  if (tf < h.t) {
    let q = p + d * tf;
    let n = vec3f(0.0, 1.0, 0.0);
    let dummy = Material(vec4f(0.0), vec4f(1.0, 0.0, 1.0, 0.0), vec4f(0.0), vec4f(1.5, 1.0, 1.0, 0.0), vec4f(-1.0), vec4f(0.0));
    return sample_lights(q + n * EPS, n, -d, dummy, Lobes(0.0, 0.0, 1.0, 0.0), true);
  }
  let sf = surface(p, d, h);
  var n = sf.n;
  var ng = sf.ng;
  if (dot(d, ng) > 0.0) { n = -n; ng = -ng; }
  let nv = max(dot(n, -d), 1e-4);
  shade_t = tris[u32(h.tri)].tangent.xyz;
  return sf.m.emit.xyz + sample_lights(sf.p + ng * EPS, n, -d, sf.m, lobe_probs(sf.m, nv), false);
}

// The sun disc as seen along an escaping ray (no MIS: used where light
// sampling cannot reach, such as mirror reflections).
fn sun_seen(d: vec3f) -> vec3f {
  var e = vec3f(0.0);
  for (var i = 0u; i < P.counts.x; i++) {
    let l = lights[i];
    if (light_kind(l) == 3u && dot(d, normalize(l.pos.xyz)) >= l.u.x) { e += l.radiance.xyz; }
  }
  return e;
}

struct Sample { radiance: vec3f, bg: vec3f, normal: vec3f, depth: f32, void_hit: f32, emit: vec3f, object: u32 }

fn camera_ray(px: vec2f) -> array<vec3f, 2> {
  let dims = vec2f(f32(P.dims.x), f32(P.dims.y));
  let ndc = (px / dims) * 2.0 - 1.0;
  let fwd = P.cam_fwd.xyz;
  let d = normalize(fwd + P.cam_right.xyz * ndc.x * P.cam_right.w - P.cam_up.xyz * ndc.y * P.cam_up.w);
  var ro = P.cam_pos.xyz;
  var rd = d;
  // Live uses a pinhole: depth of field appears as the view refines.
  if (P.cam_pos.w > 0.0 && P.opts.x == 0u) {
    let focus = ro + d * (P.cam_fwd.w / dot(d, fwd));
    let u = rnd2();
    let r = sqrt(u.x) * P.cam_pos.w;
    let a = 2.0 * PI * u.y;
    ro = ro + P.cam_right.xyz * r * cos(a) + P.cam_up.xyz * r * sin(a);
    rd = normalize(focus - ro);
  }
  return array<vec3f, 2>(ro, rd);
}

fn path(px: vec2f) -> Sample {
  var s = Sample(vec3f(0.0), vec3f(0.0), vec3f(0.0), -1.0, 0.0, vec3f(0.0), 0u);
  let ray = camera_ray(px);
  var ro = ray[0];
  var rd = ray[1];
  var tp = vec3f(1.0);
  var last_pdf = 0.0;   // BSDF pdf of the last bounce (0: camera or delta)
  var primary = true;   // still on the camera ray (passes through cut-outs keep it)
  var band = -1;        // dispersion: the RGB channel this path carries (-1: all)
  let max_b = P.counts.y;
  for (var b = 0u; b <= max_b + 2u; b++) {
    if (b > max_b && !primary) { break; }
    let h = intersect(ro, rd, BIG, false);
    let tf = floor_t(ro, rd);
    // Lights: invisible to the camera, visible in reflections.
    if (!primary) {
      for (var i = 0u; i < P.counts.x; i++) {
        let l = lights[i];
        let tl = hit_light(ro, rd, l);
        if (tl < h.t && tl < tf) {
          let e = light_emit(l, ro + rd * tl, rd);
          var mis = 1.0;
          if (last_pdf > 0.0) {
            let pl = light_pdf(l, ro, rd, tl) / f32(P.counts.x);
            mis = last_pdf * last_pdf / (last_pdf * last_pdf + pl * pl);
          }
          s.radiance += tp * e * mis;
          return s;
        }
      }
    }
    if (h.tri < 0 && tf >= BIG) {
      if (primary) {
        s.bg = P.backdrop.xyz;
        s.void_hit = 1.0;
      } else {
        s.radiance += tp * environment(rd);
        // The sun disc, weighted against light sampling.
        for (var i = 0u; i < P.counts.x; i++) {
          let l = lights[i];
          if (light_kind(l) == 3u && dot(rd, normalize(l.pos.xyz)) >= l.u.x) {
            var mis = 1.0;
            if (last_pdf > 0.0) {
              let pl = light_pdf(l, ro, rd, BIG) / f32(P.counts.x);
              mis = last_pdf * last_pdf / (last_pdf * last_pdf + pl * pl);
            }
            s.radiance += tp * l.radiance.xyz * mis;
          }
        }
      }
      return s;
    }
    if (tf < h.t) {
      let p = ro + rd * tf;
      if (primary) {
        s.bg = P.backdrop.xyz * catcher_ratio(p);
        s.void_hit = 1.0;
        return s;
      }
      // A matte floor in reflections: one diffuse bounce, lit by the lights.
      let n = vec3f(0.0, 1.0, 0.0);
      let dummy = Material(vec4f(0.0), vec4f(1.0, 0.0, 1.0, 0.0), vec4f(0.0), vec4f(1.5, 1.0, 1.0, 0.0), vec4f(-1.0), vec4f(0.0));
      s.radiance += tp * sample_lights(p + n * EPS, n, -rd, dummy, Lobes(0.0, 0.0, 1.0, 0.0), true);
      tp *= P.env_horizon.w;
      ro = p + n * EPS;
      rd = cosine_dir(n, rnd2());
      last_pdf = dot(n, rd) / PI;
      continue;
    }
    let sf = surface(ro, rd, h);
    var m = sf.m;
    shade_t = tris[u32(h.tri)].tangent.xyz;
    // Partly transparent surfaces: pass straight through with 1 − opacity.
    if (m.extra.z < 0.999 && rnd() >= m.extra.z) {
      ro = sf.p + rd * EPS;
      continue;
    }
    let front = dot(rd, sf.ng) < 0.0;
    var n = sf.n;
    var ng = sf.ng;
    if (!front) { n = -n; ng = -ng; }
    if (primary) { s.normal = n; s.depth = h.t; s.object = sf.object + 1u; }
    let v = -rd;
    // Emission (a product's screen, or a glowing part).
    if (any(m.emit.xyz > vec3f(0.0)) && front) {
      if (primary) { s.emit += m.emit.xyz; } else { s.radiance += tp * m.emit.xyz; }
    }
    var nv = max(dot(n, v), 1e-4);
    let p = sf.p + ng * EPS;
    // A near-mirror coat (cover glass, lacquer) is traced on every sample
    // instead of being picked 1 time in ~16, so its reflections never
    // sparkle; the path continues through the base layer.
    if (m.spec.y > 0.0 && m.spec.z < 0.05 && front && (primary || P.opts.x == 0u)) {
      let fc = m.spec.y * schlick1(0.04, nv);
      s.radiance += tp * fc * mirror_radiance(p, reflect(-v, n));
      tp *= 1.0 - fc;
      m.spec.y = 0.0;
    }
    primary = false;
    let w = lobe_probs(m, nv);
    // Direct light.
    if (w.trans < 0.999) { s.radiance += tp * sample_lights(p, n, v, m, w, false); }
    // Russian roulette after the third bounce.
    if (b >= 3u) {
      let q = clamp(max(tp.x, max(tp.y, tp.z)), 0.05, 0.95);
      if (rnd() > q) { return s; }
      tp /= q;
    }
    let pick = rnd();
    if (pick < w.trans) {
      // Rough dielectric transmission/reflection through a sampled microfacet.
      let a = max(m.spec.x * m.spec.x, 0.0005);
      let tb = basis(n);
      let hm = tb * sample_vndf(transpose(tb) * v, a, rnd2());
      // Dispersion: blue bends more than red. The first refraction picks one
      // channel for the rest of the path (weighted ×3 so the average holds).
      var ior = m.extra.x;
      if (m.layers.z > 0.0) {
        if (band < 0) {
          band = min(i32(rnd() * 3.0), 2);
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
        ro = sf.p + ng * EPS;
      } else {
        let r = refract(-v, hm, 1.0 / eta);
        if (dot(r, r) < 0.5) { rd = reflect(-v, hm); ro = sf.p + ng * EPS; }
        else { rd = r; ro = sf.p - ng * EPS; tp *= m.base.xyz; }
      }
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
    } else if (m.emit.w > 0.0) {
      // Subsurface: a random walk inside the object; the path leaves where it exits.
      let walk = random_walk(sf.p - ng * EPS, cosine_dir(-n, rnd2()), m);
      if (walk.w <= 0.0) { return s; }
      tp *= m.base.xyz;
      ro = walk.xyz;
      rd = cosine_dir(exit_n, rnd2());
      ro = ro + exit_n * EPS;
      last_pdf = max(dot(exit_n, rd), 1e-4) / PI;
      continue;
    } else {
      l = cosine_dir(n, rnd2());
    }
    if (dot(l, ng) <= 0.0) { return s; }
    let f = eval_reflect(m, n, v, l);
    let pdf = pdf_reflect(m, w, n, v, l) * wr;
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
  seed = pcg(idx ^ pcg(P.dims.z * 9781u + 1u));
  var rad = vec3f(0.0);
  var bg = vec3f(0.0);
  var nbg = 0.0;
  var em = vec3f(0.0);
  var nd = vec4f(0.0, 0.0, 0.0, -1.0);
  var object = 0u;
  for (var i = 0u; i < P.dims.w; i++) {
    let jitter = rnd2();
    var smp = path(vec2f(f32(gid.x), f32(gid.y)) + jitter);
    // A non-finite sample would spread through the denoiser; drop it.
    if (any(smp.radiance != smp.radiance) || any(abs(smp.radiance) > vec3f(1e20))) { smp.radiance = vec3f(0.0); }
    // Clamp fireflies from tiny bright paths (Live clamps harder).
    let lum = dot(smp.radiance, vec3f(0.2126, 0.7152, 0.0722));
    rad += smp.radiance * min(1.0, f32(P.counts.w) / max(lum, 1e-6));
    em += smp.emit;
    bg += smp.bg;
    nbg += smp.void_hit;
    if (i == 0u) { nd = vec4f(smp.normal, smp.depth); object = smp.object; }
  }
  let n = f32(P.dims.w);
  if (P.counts.z == 1u) {
    accum[idx] = vec4f(rad, n);
    bgsum[idx] = vec4f(bg, nbg);
    gbuf[idx] = nd;
    ebuf[idx] = vec4f(em, n);
    idbuf[idx] = object;
  } else {
    accum[idx] += vec4f(rad, n);
    bgsum[idx] += vec4f(bg, nbg);
    ebuf[idx] += vec4f(em, n);
  }
}
