// Studio path tracer (Phase 0). One sample per pixel per invocation of the
// sample loop; Live runs one frame of samples and denoises, Refine and Final
// keep accumulating.
//
// Units are millimetres. The floor is the plane y = 0; on screen it shows the
// seamless backdrop darkened only by the shadows and contact occlusion the
// objects cast (a shadow catcher), so the void keeps its exact display color.

struct Params {
  cam_pos: vec4f,      // w: lens radius (mm)
  cam_fwd: vec4f,      // w: focus distance (mm)
  cam_right: vec4f,    // w: tan(fov/2) * aspect
  cam_up: vec4f,       // w: tan(fov/2)
  backdrop: vec4f,     // display-referred rgb; w: shadow catcher on (1)
  env_zenith: vec4f,   // w: exposure
  env_horizon: vec4f,  // w: floor albedo for secondary bounces
  dims: vec4u,         // width, height, frame seed, samples this dispatch
  counts: vec4u,       // lights, max bounces, reset (1 = overwrite), firefly clamp (luminance)
  opts: vec4u,         // x: Live (cheaper floor shadows, primary-only mirror coat)
}

struct Tri { p0: vec4f, p1: vec4f, p2: vec4f, n0: vec4f, n1: vec4f, n2: vec4f, uv01: vec4f, uv2m: vec4f }
struct Node { lmin: vec3f, left: u32, lmax: vec3f, lcount: u32, rmin: vec3f, right: u32, rmax: vec3f, rcount: u32 }
struct Material { base: vec4f, spec: vec4f, emit: vec4f, extra: vec4f }
struct Light { center: vec4f, u: vec4f, v: vec4f, radiance: vec4f }

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> tris: array<Tri>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> materials: array<Material>;
@group(0) @binding(4) var<storage, read> lights: array<Light>;
@group(0) @binding(5) var screen_tex: texture_2d<f32>;
@group(0) @binding(6) var screen_samp: sampler;
// rgb: sum of scene radiance (linear); w: samples taken
@group(0) @binding(7) var<storage, read_write> accum: array<vec4f>;
// rgb: sum of backdrop display color for samples that saw the void/floor; w: how many
@group(0) @binding(8) var<storage, read_write> bgsum: array<vec4f>;
// xyz: first-hit normal; w: first-hit distance (-1 for the void)
@group(0) @binding(9) var<storage, read_write> gbuf: array<vec4f>;
// rgb: sum of emission seen directly by the camera (the display's composition):
// exact per pixel, so Live denoises only what is noisy.
@group(0) @binding(10) var<storage, read_write> ebuf: array<vec4f>;
// Corner positions, 3 per triangle in BVH order (intersection reads only these).
@group(0) @binding(11) var<storage, read> pos: array<vec4f>;

const PI: f32 = 3.14159265;
const EPS: f32 = 0.004;
const BIG: f32 = 1e30;

var<private> seed: u32;
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

fn occluded(ro: vec3f, rd: vec3f, dist: f32) -> bool {
  return intersect(ro, rd, dist - 2.0 * EPS, true).tri >= 0;
}

fn floor_t(ro: vec3f, rd: vec3f) -> f32 {
  if (P.backdrop.w < 0.5 || rd.y >= -1e-6 || ro.y <= 0.0) { return BIG; }
  return -ro.y / rd.y;
}

fn environment(d: vec3f) -> vec3f {
  let k = clamp(d.y * 0.5 + 0.5, 0.0, 1.0);
  return mix(P.env_horizon.xyz * 0.6, P.env_zenith.xyz, k) + P.env_horizon.xyz * 0.4 * exp(-abs(d.y) * 6.0);
}

// ── Lights ──────────────────────────────────────────────
fn light_normal(l: Light) -> vec3f { return normalize(cross(l.u.xyz, l.v.xyz)); }

// Distance to a softbox along the ray, or BIG.
fn hit_light(ro: vec3f, rd: vec3f, l: Light) -> f32 {
  let n = light_normal(l);
  let dn = dot(rd, n);
  if (dn >= 0.0) { return BIG; } // one-sided, emits along n
  let t = dot(l.center.xyz - ro, n) / dn;
  if (t <= EPS) { return BIG; }
  let q = ro + rd * t - l.center.xyz;
  let uu = dot(q, l.u.xyz) / dot(l.u.xyz, l.u.xyz);
  let vv = dot(q, l.v.xyz) / dot(l.v.xyz, l.v.xyz);
  if (abs(uu) > 1.0 || abs(vv) > 1.0) { return BIG; }
  return t;
}

// Soft edges: falloff toward the softbox rim, like a diffusion panel.
fn light_falloff(l: Light, p: vec3f) -> f32 {
  let q = p - l.center.xyz;
  let uu = abs(dot(q, l.u.xyz) / dot(l.u.xyz, l.u.xyz));
  let vv = abs(dot(q, l.v.xyz) / dot(l.v.xyz, l.v.xyz));
  return smoothstep(1.0, 0.82, max(uu, vv));
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

struct Lobes { coat: f32, spec: f32, diff: f32, trans: f32 }

fn lobe_probs(m: Material, nv: f32) -> Lobes {
  let metallic = m.base.w;
  let tr = m.spec.w * (1.0 - metallic);
  let fc = m.spec.y * schlick1(0.04, nv);
  let f0 = mix(vec3f(0.04 * m.extra.y), m.base.xyz, metallic);
  let fs = schlick(f0, nv);
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
  let f0 = mix(vec3f(0.04 * m.extra.y), m.base.xyz, metallic);
  let F = schlick(f0, vh);
  let spec = F * ggx_d(nh, a) * smith_g2(nv, nl, a) / (4.0 * nv);
  let tr = m.spec.w * (1.0 - metallic);
  let diff = m.base.xyz * (1.0 - metallic) * (1.0 - tr) * (1.0 - schlick(f0, nl)) * (1.0 - schlick(f0, nv)) * nl / PI;
  let ca = m.spec.z * m.spec.z;
  let fc = m.spec.y * schlick1(0.04, vh);
  let coat = fc * ggx_d(nh, ca) * smith_g2(nv, nl, ca) / (4.0 * nv);
  let atten = (1.0 - m.spec.y * schlick1(0.04, nv)) * (1.0 - m.spec.y * schlick1(0.04, nl));
  return vec3f(coat) + (spec + diff) * atten;
}

fn pdf_reflect(m: Material, w: Lobes, n: vec3f, v: vec3f, l: vec3f) -> f32 {
  let nl = dot(n, l);
  let nv = dot(n, v);
  if (nl <= 0.0 || nv <= 0.0) { return 0.0; }
  let h = normalize(v + l);
  let nh = max(dot(n, h), 0.0);
  let a = m.spec.x * m.spec.x;
  let ca = m.spec.z * m.spec.z;
  return w.coat * spec_pdf(nv, nh, ca) + w.spec * spec_pdf(nv, nh, a) + w.diff * nl / PI;
}

fn cosine_dir(n: vec3f, u: vec2f) -> vec3f {
  let r = sqrt(u.x);
  let phi = 2.0 * PI * u.y;
  return basis(n) * vec3f(r * cos(phi), r * sin(phi), sqrt(max(0.0, 1.0 - u.x)));
}

// One light sample with MIS against the BSDF; returns radiance × throughput factor.
fn sample_lights(p: vec3f, n: vec3f, v: vec3f, m: Material, w: Lobes, floor_hit: bool) -> vec3f {
  let count = P.counts.x;
  if (count == 0u) { return vec3f(0.0); }
  let li = min(u32(rnd() * f32(count)), count - 1u);
  let l = lights[li];
  let u = rnd2() * 2.0 - 1.0;
  let q = l.center.xyz + l.u.xyz * u.x + l.v.xyz * u.y;
  let d = q - p;
  let dist2 = dot(d, d);
  let dist = sqrt(dist2);
  let dir = d / dist;
  let cl = dot(-dir, light_normal(l));
  if (cl <= 0.0) { return vec3f(0.0); }
  var f = vec3f(0.0);
  var pb = 0.0;
  if (floor_hit) {
    let nl = dot(n, dir);
    if (nl <= 0.0) { return vec3f(0.0); }
    f = vec3f(P.env_horizon.w * nl / PI);
    pb = nl / PI;
  } else {
    f = eval_reflect(m, n, v, dir);
    pb = pdf_reflect(m, w, n, v, dir) * (1.0 - w.trans);
  }
  if (all(f <= vec3f(0.0))) { return vec3f(0.0); }
  if (occluded(p, dir, dist)) { return vec3f(0.0); }
  let pl = dist2 / (cl * l.center.w) / f32(count);
  let mis = pl * pl / (pl * pl + pb * pb);
  return f * l.radiance.xyz * light_falloff(l, q) * mis / pl;
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
    let l = lights[i];
    let u = rnd2() * 2.0 - 1.0;
    let q = l.center.xyz + l.u.xyz * u.x + l.v.xyz * u.y;
    let d = q - p;
    let dist2 = dot(d, d);
    let dir = d * inverseSqrt(dist2);
    let c = max(dot(n, dir), 0.0) * max(dot(-dir, light_normal(l)), 0.0) * l.center.w / dist2;
    let e = c * dot(l.radiance.xyz, vec3f(0.333));
    free += e;
    if (e > 0.0 && !occluded(p, dir, sqrt(dist2))) { lit += e; }
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

struct Surface { p: vec3f, n: vec3f, ng: vec3f, uv: vec2f, mat: u32 }

// Radiance arriving along a mirror direction: a softbox, the dome, the floor
// or another object (its emission plus one light sample).
fn mirror_radiance(p: vec3f, d: vec3f) -> vec3f {
  let h = intersect(p, d, BIG, false);
  let tf = floor_t(p, d);
  for (var i = 0u; i < P.counts.x; i++) {
    let l = lights[i];
    let tl = hit_light(p, d, l);
    if (tl < h.t && tl < tf) { return l.radiance.xyz * light_falloff(l, p + d * tl); }
  }
  if (h.tri < 0 && tf >= BIG) { return environment(d); }
  if (tf < h.t) {
    let q = p + d * tf;
    let n = vec3f(0.0, 1.0, 0.0);
    let dummy = Material(vec4f(0.0), vec4f(1.0, 0.0, 1.0, 0.0), vec4f(0.0), vec4f(1.5, 1.0, 0.0, 0.0));
    return sample_lights(q + n * EPS, n, -d, dummy, Lobes(0.0, 0.0, 1.0, 0.0), true);
  }
  let sf = surface(p, d, h);
  let m = materials[sf.mat];
  var n = sf.n;
  var ng = sf.ng;
  if (dot(d, ng) > 0.0) { n = -n; ng = -ng; }
  var e = m.emit.xyz;
  if (m.emit.w > 0.5) { e *= textureSampleLevel(screen_tex, screen_samp, sf.uv, 0.0).rgb; }
  let nv = max(dot(n, -d), 1e-4);
  return e + sample_lights(sf.p + ng * EPS, n, -d, m, lobe_probs(m, nv), false);
}

fn surface(ro: vec3f, rd: vec3f, h: Hit) -> Surface {
  let t = tris[u32(h.tri)];
  let w = 1.0 - h.b.x - h.b.y;
  let p = ro + rd * h.t;
  let ng = normalize(cross(t.p1.xyz - t.p0.xyz, t.p2.xyz - t.p0.xyz));
  var n = normalize(t.n0.xyz * w + t.n1.xyz * h.b.x + t.n2.xyz * h.b.y);
  if (dot(n, ng) < 0.0) { n = -n; } // normals and winding may disagree
  let uv = t.uv01.xy * w + t.uv01.zw * h.b.x + t.uv2m.xy * h.b.y;
  return Surface(p, n, ng, uv, u32(t.uv2m.z));
}

struct Sample { radiance: vec3f, bg: vec3f, normal: vec3f, depth: f32, void_hit: f32, emit: vec3f }

fn camera_ray(px: vec2f) -> array<vec3f, 2> {
  let dims = vec2f(f32(P.dims.x), f32(P.dims.y));
  let ndc = (px / dims) * 2.0 - 1.0;
  let fwd = P.cam_fwd.xyz;
  let d = normalize(fwd + P.cam_right.xyz * ndc.x * P.cam_right.w - P.cam_up.xyz * ndc.y * P.cam_up.w);
  var ro = P.cam_pos.xyz;
  var rd = d;
  if (P.cam_pos.w > 0.0) {
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
  var s = Sample(vec3f(0.0), vec3f(0.0), vec3f(0.0), -1.0, 0.0, vec3f(0.0));
  let ray = camera_ray(px);
  var ro = ray[0];
  var rd = ray[1];
  var tp = vec3f(1.0);
  var last_pdf = 0.0;   // BSDF pdf of the last bounce (0: camera or delta)
  var inside = false;
  let max_b = P.counts.y;
  for (var b = 0u; b <= max_b; b++) {
    let h = intersect(ro, rd, BIG, false);
    let tf = floor_t(ro, rd);
    // Softboxes: invisible to the camera, visible in reflections.
    if (b > 0u) {
      for (var i = 0u; i < P.counts.x; i++) {
        let l = lights[i];
        let tl = hit_light(ro, rd, l);
        if (tl < h.t && tl < tf) {
          var mis = 1.0;
          if (last_pdf > 0.0) {
            let q = ro + rd * tl;
            let cl = max(dot(-rd, light_normal(l)), 1e-4);
            let pl = tl * tl / (cl * l.center.w) / f32(P.counts.x);
            mis = last_pdf * last_pdf / (last_pdf * last_pdf + pl * pl);
            s.radiance += tp * l.radiance.xyz * light_falloff(l, q) * mis;
          } else {
            s.radiance += tp * l.radiance.xyz * light_falloff(l, ro + rd * tl);
          }
          return s;
        }
      }
    }
    if (h.tri < 0 && tf >= BIG) {
      if (b == 0u) { s.bg = P.backdrop.xyz; s.void_hit = 1.0; } else { s.radiance += tp * environment(rd); }
      return s;
    }
    if (tf < h.t) {
      let p = ro + rd * tf;
      if (b == 0u) {
        s.bg = P.backdrop.xyz * catcher_ratio(p);
        s.void_hit = 1.0;
        return s;
      }
      // A matte floor in reflections: one diffuse bounce, lit by the softboxes.
      let n = vec3f(0.0, 1.0, 0.0);
      let dummy = Material(vec4f(0.0), vec4f(1.0, 0.0, 1.0, 0.0), vec4f(0.0), vec4f(1.5));
      s.radiance += tp * sample_lights(p + n * EPS, n, -rd, dummy, Lobes(0.0, 0.0, 1.0, 0.0), true);
      tp *= P.env_horizon.w;
      ro = p + n * EPS;
      rd = cosine_dir(n, rnd2());
      last_pdf = dot(n, rd) / PI;
      continue;
    }
    let sf = surface(ro, rd, h);
    var m = materials[sf.mat];
    let front = dot(rd, sf.ng) < 0.0;
    var n = sf.n;
    var ng = sf.ng;
    if (!front) { n = -n; ng = -ng; }
    if (b == 0u) { s.normal = n; s.depth = h.t; }
    let v = -rd;
    // Emission (the display's composition, or a glowing part).
    if (any(m.emit.xyz > vec3f(0.0))) {
      var e = m.emit.xyz;
      if (m.emit.w > 0.5) { e *= textureSampleLevel(screen_tex, screen_samp, sf.uv, 0.0).rgb; }
      if (front) {
        if (b == 0u) { s.emit += e; } else { s.radiance += tp * e; }
      }
    }
    var nv = max(dot(n, v), 1e-4);
    let p = sf.p + ng * EPS;
    // A near-mirror coat (cover glass, lacquer) is traced on every sample
    // instead of being picked 1 time in ~16, so its reflections never
    // sparkle; the path continues through the base layer.
    if (m.spec.y > 0.0 && m.spec.z < 0.05 && front && (b == 0u || P.opts.x == 0u)) {
      let fc = m.spec.y * schlick1(0.04, nv);
      s.radiance += tp * fc * mirror_radiance(p, reflect(-v, n));
      tp *= 1.0 - fc;
      m.spec.y = 0.0;
    }
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
      let vl = transpose(tb) * v;
      let hm = tb * sample_vndf(vl, a, rnd2());
      let eta = select(1.0 / m.extra.x, m.extra.x, front);
      let cosi = dot(v, hm);
      let F = fresnel_dielectric(abs(cosi), select(1.0 / eta, eta, true));
      if (rnd() < F) {
        rd = reflect(-v, hm);
        ro = sf.p + ng * EPS;
      } else {
        let r = refract(-v, hm, 1.0 / eta);
        if (dot(r, r) < 0.5) { rd = reflect(-v, hm); ro = sf.p + ng * EPS; }
        else { rd = r; ro = sf.p - ng * EPS; tp *= m.base.xyz; inside = !inside; }
      }
      last_pdf = 0.0;
      continue;
    }
    var l = vec3f(0.0);
    let r = (pick - w.trans) / max(1.0 - w.trans, 1e-6);
    let wr = 1.0 - w.trans;
    if (r < w.coat / wr) {
      let ca = m.spec.z * m.spec.z;
      let tb = basis(n);
      let hm = tb * sample_vndf(transpose(tb) * v, ca, rnd2());
      l = reflect(-v, hm);
    } else if (r < (w.coat + w.spec) / wr) {
      let a = m.spec.x * m.spec.x;
      let tb = basis(n);
      let hm = tb * sample_vndf(transpose(tb) * v, a, rnd2());
      l = reflect(-v, hm);
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
  for (var i = 0u; i < P.dims.w; i++) {
    let jitter = rnd2();
    var smp = path(vec2f(f32(gid.x), f32(gid.y)) + jitter);
    // A non-finite sample would spread through the denoiser; drop it.
    if (any(smp.radiance != smp.radiance) || any(abs(smp.radiance) > vec3f(1e20))) { smp.radiance = vec3f(0.0); }
    // Clamp fireflies from tiny bright paths (Live and Refine only differ in count).
    let lum = dot(smp.radiance, vec3f(0.2126, 0.7152, 0.0722));
    rad += smp.radiance * min(1.0, f32(P.counts.w) / max(lum, 1e-6));
    em += smp.emit;
    bg += smp.bg;
    nbg += smp.void_hit;
    if (i == 0u) { nd = vec4f(smp.normal, smp.depth); }
  }
  let n = f32(P.dims.w);
  if (P.counts.z == 1u) {
    accum[idx] = vec4f(rad, n);
    bgsum[idx] = vec4f(bg, nbg);
    gbuf[idx] = nd;
    ebuf[idx] = vec4f(em, n);
  } else {
    ebuf[idx] += vec4f(em, n);
    accum[idx] += vec4f(rad, n);
    bgsum[idx] += vec4f(bg, nbg);
  }
}
