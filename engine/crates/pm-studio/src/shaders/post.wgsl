// Resolve, denoise and pack.
//
// resolve: the noisy part of each pixel, linear: rgb = scene radiance without
//          the emission the camera sees directly; a = the backdrop's mean
//          shadow-catcher brightness.
// atrous:  one edge-avoiding à-trous pass (5×5 B-spline) guided by the first
//          hit's normal and distance (Live only). The void and the product
//          never blend.
// compose: + exact direct emission → exposure → Khronos PBR Neutral → sRGB,
//          weighted by coverage, plus the backdrop's display color (already
//          display-referred, so the void stays exactly #F6F5F8).
// pack:    display color → RGBA8.

// temporal: Live reuses the previous frame. Each pixel's world point (from
//          its first-hit distance) is projected into the previous camera; if
//          the history there saw the same surface (distance and normal agree)
//          the new sample blends into it, up to 16 frames deep.

// dims: width, height, step, history valid (1); k: exposure, color sigma;
// backdrop: display rgb, w: its opacity (1: the void shows the backdrop; 0:
// the void is transparent). Cameras: position (w: unused), forward, right
// (w: tan(fov/2)·aspect), up (w: tan(fov/2)) — now and one frame ago.
struct Post {
  dims: vec4u, k: vec4f, backdrop: vec4f,
  cur_pos: vec4f, cur_fwd: vec4f, cur_right: vec4f, cur_up: vec4f,
  prev_pos: vec4f, prev_fwd: vec4f, prev_right: vec4f, prev_up: vec4f,
}
// History: accumulated linear color (w: frames) and the surface it saw (normal, distance).
struct Hist { c: vec4f, g: vec4f }

@group(0) @binding(0) var<uniform> Q: Post;
@group(0) @binding(1) var<storage, read> accum: array<vec4f>;
@group(0) @binding(2) var<storage, read> bgsum: array<vec4f>;
@group(0) @binding(3) var<storage, read> gbuf: array<vec4f>;
@group(0) @binding(4) var<storage, read> src: array<vec4f>;
@group(0) @binding(5) var<storage, read_write> dst: array<vec4f>;
@group(0) @binding(6) var<storage, read_write> packed: array<u32>;
@group(0) @binding(7) var<storage, read> ebuf: array<vec4f>;
@group(0) @binding(8) var<storage, read> hist_in: array<Hist>;
@group(0) @binding(9) var<storage, read_write> hist_out: array<Hist>;

fn neutral(c0: vec3f) -> vec3f {
  var c = c0;
  let start = 0.8 - 0.04;
  let desat = 0.15;
  let x = min(c.r, min(c.g, c.b));
  let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
  c -= offset;
  let peak = max(c.r, max(c.g, c.b));
  if (peak < start) { return c; }
  let d = 1.0 - start;
  let np = 1.0 - d * d / (peak + d - start);
  c *= np / peak;
  let g = 1.0 - 1.0 / (desat * (peak - np) + 1.0);
  return mix(c, vec3f(np), g);
}

fn encode(c: vec3f) -> vec3f {
  let lo = c * 12.92;
  let hi = 1.055 * pow(max(c, vec3f(0.0)), vec3f(1.0 / 2.4)) - 0.055;
  return select(hi, lo, c <= vec3f(0.0031308));
}

@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) g: vec3u) {
  if (g.x >= Q.dims.x || g.y >= Q.dims.y) { return; }
  let i = g.y * Q.dims.x + g.x;
  let a = accum[i];
  let b = bgsum[i];
  let scene_n = max(a.w - b.w, 0.0);
  var rgb = vec3f(0.0);
  if (scene_n > 0.0) { rgb = a.xyz / scene_n; }
  var shade = 1.0;
  if (b.w > 0.0) { shade = dot(b.xyz / b.w, vec3f(1.0)) / max(dot(Q.backdrop.xyz, vec3f(1.0)), 1e-6); }
  dst[i] = vec4f(rgb, shade);
}

@compute @workgroup_size(8, 8)
fn compose(@builtin(global_invocation_id) g: vec3u) {
  if (g.x >= Q.dims.x || g.y >= Q.dims.y) { return; }
  let i = g.y * Q.dims.x + g.x;
  let a = accum[i];
  let b = bgsum[i];
  let d = src[i];
  let n = max(a.w, 1.0);
  let scene_n = max(a.w - b.w, 0.0);
  // Premultiplied: scene color weighted by coverage, plus the backdrop.
  let void_a = (b.w / n) * Q.backdrop.w;
  var c = Q.backdrop.xyz * d.w * void_a;
  if (scene_n > 0.0) {
    let mean = (d.xyz + ebuf[i].xyz / scene_n) * Q.k.x;
    c += encode(clamp(neutral(mean), vec3f(0.0), vec3f(1.0))) * (scene_n / n);
  }
  dst[i] = vec4f(c, scene_n / n + void_a);
}

@compute @workgroup_size(8, 8)
fn atrous(@builtin(global_invocation_id) g: vec3u) {
  if (g.x >= Q.dims.x || g.y >= Q.dims.y) { return; }
  let w = i32(Q.dims.x);
  let h = i32(Q.dims.y);
  let step = i32(Q.dims.z);
  let i = g.y * Q.dims.x + g.x;
  let c0 = src[i];
  let g0 = gbuf[i];
  let kern = array<f32, 5>(1.0 / 16.0, 1.0 / 4.0, 3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0);
  var sum = vec4f(0.0);
  var wsum = 0.0;
  let sigma_c = Q.k.y;
  for (var dy = -2; dy <= 2; dy++) {
    for (var dx = -2; dx <= 2; dx++) {
      let x = clamp(i32(g.x) + dx * step, 0, w - 1);
      let y = clamp(i32(g.y) + dy * step, 0, h - 1);
      let j = u32(y * w + x);
      let cq = src[j];
      let gq = gbuf[j];
      var wt = kern[dx + 2] * kern[dy + 2];
      // Never blend the void with the product.
      if ((g0.w < 0.0) != (gq.w < 0.0)) { continue; }
      if (g0.w >= 0.0) {
        wt *= pow(max(dot(g0.xyz, gq.xyz), 0.0), 32.0);
        wt *= exp(-abs(g0.w - gq.w) / (0.02 * g0.w + 0.5));
      }
      // Product: edge-stop on relative luminance, loosely (1-sample noise is
      // high-contrast). The void's shadows are smooth: blur them by position only.
      if (g0.w >= 0.0) {
        let l0 = dot(c0.xyz, vec3f(0.2126, 0.7152, 0.0722));
        let lq = dot(cq.xyz, vec3f(0.2126, 0.7152, 0.0722));
        let dc = (l0 - lq) / (0.15 + 0.5 * (l0 + lq));
        wt *= exp(-dc * dc / (sigma_c * sigma_c + 1e-6));
      }
      sum += cq * wt;
      wsum += wt;
    }
  }
  dst[i] = sum / max(wsum, 1e-6);
}

@compute @workgroup_size(8, 8)
fn pack(@builtin(global_invocation_id) g: vec3u) {
  if (g.x >= Q.dims.x || g.y >= Q.dims.y) { return; }
  let i = g.y * Q.dims.x + g.x;
  let c = clamp(src[i], vec4f(0.0), vec4f(1.0));
  let q = vec4u(c * 255.0 + 0.5);
  packed[i] = q.x | (q.y << 8u) | (q.z << 16u) | (q.w << 24u);
}

@compute @workgroup_size(8, 8)
fn temporal(@builtin(global_invocation_id) g: vec3u) {
  if (g.x >= Q.dims.x || g.y >= Q.dims.y) { return; }
  let i = g.y * Q.dims.x + g.x;
  let cur = src[i];
  let geo = gbuf[i];
  // The void (no first hit) and frames without history pass straight through.
  if (geo.w < 0.0 || Q.dims.w == 0u) {
    hist_out[i] = Hist(vec4f(cur.xyz, 1.0), geo);
    dst[i] = cur;
    return;
  }
  let dims = vec2f(f32(Q.dims.x), f32(Q.dims.y));
  let ndc = (vec2f(f32(g.x), f32(g.y)) + 0.5) / dims * 2.0 - 1.0;
  let d = normalize(Q.cur_fwd.xyz + Q.cur_right.xyz * ndc.x * Q.cur_right.w - Q.cur_up.xyz * ndc.y * Q.cur_up.w);
  let p = Q.cur_pos.xyz + d * geo.w;
  // Into the previous camera.
  let v = p - Q.prev_pos.xyz;
  let z = dot(v, Q.prev_fwd.xyz);
  var ok = z > 1e-4;
  var prev = vec2i(0);
  if (ok) {
    let pn = vec2f(dot(v, Q.prev_right.xyz) / (z * Q.prev_right.w), -dot(v, Q.prev_up.xyz) / (z * Q.prev_up.w));
    let px = (pn * 0.5 + 0.5) * dims;
    prev = vec2i(floor(px));
    ok = all(prev >= vec2i(0)) && all(prev < vec2i(i32(Q.dims.x), i32(Q.dims.y)));
  }
  var out = vec4f(cur.xyz, 1.0);
  if (ok) {
    let h = hist_in[u32(prev.y) * Q.dims.x + u32(prev.x)];
    let dist = length(v);
    let same = h.g.w >= 0.0 && abs(h.g.w - dist) < 0.03 * dist + 1e-3 && dot(h.g.xyz, geo.xyz) > 0.9;
    if (same) {
      let n = min(h.c.w + 1.0, 16.0);
      out = vec4f(mix(h.c.xyz, cur.xyz, 1.0 / n), n);
    }
  }
  hist_out[i] = Hist(out, geo);
  dst[i] = vec4f(out.xyz, cur.w);
}
