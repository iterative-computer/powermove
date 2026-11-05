// Final's denoiser: an edge-avoiding à-trous filter guided by the first hit
// (aov.wgsl's albedo, normals, coverage and distance, averaged over the pixel
// and the lens) and by each pixel's own noise, after SVGF
// (Schied et al. 2017) for a single frame.
//
// It filters everything that is noisy: the product's light and emission,
// premultiplied by how much of the pixel the product covers (in shallow focus
// that coverage is as noisy as the light), and the backdrop's shadow-catcher
// shading. Pixels the guides see as wholly product or wholly void never blend,
// so in-focus edges stay sharp and the void stays exactly the backdrop.
//
// demod:       light ÷ albedo (paint and textures stay sharp; only light is
//              smoothed; not under shallow focus) and coverage → color; each pixel's variance from its
//              samples (accum, ebuf.a); shading and its variance (from its
//              neighbours) → side.
// atrous_step: one 5×5 B-spline pass at `step`. Weights stop at coverage,
//              albedo, normals and distance, and at luminance
//              differences larger than the noise explains (σ_l · its standard
//              deviation). Variance filters with squared weights, so each pass
//              trusts the last more.
// finish:      × albedo, into what post.wgsl's compose reads: color = (mean
//              light, shading), and side = a bgsum whose w is the void's share
//              (compose then gets its coverage from the filtered one).

// dims: width, height, step; k: σ_l, the lens radius (> 0: shallow focus).
struct Den { dims: vec4u, k: vec4f }

@group(0) @binding(0) var<uniform> D: Den;
@group(0) @binding(2) var<storage, read> aov: array<vec4f>;
// Color: light ÷ albedo, premultiplied by coverage (rgb), coverage (a).
@group(0) @binding(3) var<storage, read> src: array<vec4f>;
@group(0) @binding(4) var<storage, read_write> dst: array<vec4f>;
// Side: x the color's variance (luminance), y the shading's, z the shading.
@group(0) @binding(5) var<storage, read> side_in: array<vec4f>;
@group(0) @binding(6) var<storage, read_write> side_out: array<vec4f>;
@group(0) @binding(7) var<storage, read> accum: array<vec4f>;
@group(0) @binding(8) var<storage, read> bgsum: array<vec4f>;
@group(0) @binding(9) var<storage, read> ebuf: array<vec4f>;

fn lum(c: vec3f) -> f32 { return dot(c, vec3f(0.2126, 0.7152, 0.0722)); }
// What light is divided by: the product's albedo (averaged over the guide rays
// that hit it, so not over its share of the pixel), keeping paint and texture
// sharp. Not under shallow focus: there the guides' albedo is a different
// mixture of materials in each pixel, and a division that varies pixel by
// pixel (or fades with focus) splits pixels from their neighbours, so light is
// filtered as it is.
// A lens with an aperture (k.y: its radius): coverage is noisy and filtered.
fn shallow() -> bool { return D.k.y > 0.0; }
fn albedo(i: u32) -> vec3f { return select(max(aov[2u * i].xyz, vec3f(0.02)), vec3f(1.0), D.k.y > 0.0); }
// The share of the guide rays that hit the product.
fn hits(i: u32) -> f32 { return aov[2u * i].w; }
fn normal(i: u32) -> vec3f {
  let n = aov[2u * i + 1u].xyz;
  let l = length(n);
  return select(n / l, vec3f(0.0), l < 1e-4);
}
fn index(x: i32, y: i32) -> u32 {
  return u32(clamp(y, 0, i32(D.dims.y) - 1)) * D.dims.x + u32(clamp(x, 0, i32(D.dims.x) - 1));
}

// How alike two pixels' first hits are.
fn guide(i: u32, j: u32, step: f32) -> f32 {
  let h0 = hits(i);
  let hq = hits(j);
  let dh = h0 - hq;
  var w = exp(-dh * dh / 0.25);
  let da = aov[2u * i].xyz - aov[2u * j].xyz;
  // Loose: dividing by albedo already keeps paint apart, and pixels on a
  // boundary between two materials (an in-between albedo) need neighbours.
  w *= exp(-dot(da, da) / 0.1);
  let n0 = normal(i);
  let nq = normal(j);
  if (dot(n0, n0) > 0.5 && dot(nq, nq) > 0.5) { w *= pow(max(dot(n0, nq), 0.0), 64.0); }
  // Distance, averaged over the lens like the rest (one sample's would jump in shallow focus).
  if (h0 > 0.0 && hq > 0.0) {
    let z0 = aov[2u * i + 1u].w;
    w *= exp(-abs(z0 - aov[2u * j + 1u].w) / (0.004 * z0 * step + 1e-4));
  }
  return w;
}

@compute @workgroup_size(8, 8)
fn demod(@builtin(global_invocation_id) g: vec3u) {
  if (g.x >= D.dims.x || g.y >= D.dims.y) { return; }
  let i = g.y * D.dims.x + g.x;
  let a = accum[i];
  let e = ebuf[i];
  let n = max(a.w, 1.0);
  let al = albedo(i);
  // Light and emission. Under shallow focus, over all samples (the void's count
  // as none) and so premultiplied by coverage, which is as noisy as the light
  // there and is filtered with it. In focus, coverage is exact (only edges are
  // partial): light is the mean of the product's samples and coverage is kept.
  let scene = max(a.w - bgsum[i].w, 0.0);
  let p = select((a.xyz + e.xyz) / max(scene, 1.0), (a.xyz + e.xyz) / n, shallow());
  dst[i] = vec4f(p / al, scene / n);
  // The variance of the mean of the samples' luminance.
  let mean = lum(p);
  let l = max(lum(al), 0.02);
  var v = max(e.w / n - mean * mean, 0.0) / max(n - 1.0, 1.0) / (l * l);
  // From few samples that estimate is often far too low (they happen to
  // agree): then the spread among alike neighbours bounds it from below.
  if (n < 16.0) {
    var c1 = 0.0;
    var c2 = 0.0;
    var cw = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
      for (var dx = -2; dx <= 2; dx++) {
        let j = index(i32(g.x) + dx, i32(g.y) + dy);
        let wt = guide(i, j, 1.0);
        let aj = accum[j];
        let lj = lum((aj.xyz + ebuf[j].xyz) / max(aj.w, 1.0)) / l;
        c1 += wt * lj;
        c2 += wt * lj * lj;
        cw += wt;
      }
    }
    v = max(v, max(c2 / cw - (c1 / cw) * (c1 / cw), 0.0));
  }
  // Shading: from neighbours that see some void.
  var m1 = 0.0;
  var m2 = 0.0;
  var ws = 0.0;
  for (var dy = -2; dy <= 2; dy++) {
    for (var dx = -2; dx <= 2; dx++) {
      let j = index(i32(g.x) + dx, i32(g.y) + dy);
      let wt = 1.0 - hits(j);
      let s = src[j].w;
      m1 += wt * s;
      m2 += wt * s * s;
      ws += wt;
    }
  }
  let vs = select(0.0, max(m2 / ws - (m1 / ws) * (m1 / ws), 0.0), ws > 1e-3);
  side_out[i] = vec4f(v, vs, src[i].w, 0.0);
}

@compute @workgroup_size(8, 8)
fn atrous_step(@builtin(global_invocation_id) g: vec3u) {
  if (g.x >= D.dims.x || g.y >= D.dims.y) { return; }
  let i = g.y * D.dims.x + g.x;
  let step = i32(D.dims.z);
  let c0 = src[i];
  let s0 = side_in[i];
  let l0 = lum(c0.xyz);
  // A 3×3 blur of the variance steadies the edge-stop (it comes from few samples).
  var vb = vec2f(0.0);
  for (var dy = -1; dy <= 1; dy++) {
    for (var dx = -1; dx <= 1; dx++) {
      let k = select(0.25, 0.5, dx == 0) * select(0.25, 0.5, dy == 0);
      vb += k * side_in[index(i32(g.x) + dx, i32(g.y) + dy)].xy;
    }
  }
  let sigma = D.k.x * sqrt(vb) + vec2f(1e-5);
  let kern = array<f32, 5>(1.0 / 16.0, 1.0 / 4.0, 3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0);
  var sum = vec4f(0.0);
  var var_c = 0.0;
  var wsum = 0.0;
  var shade = 0.0;
  var var_s = 0.0;
  var ssum = 0.0;
  for (var dy = -2; dy <= 2; dy++) {
    for (var dx = -2; dx <= 2; dx++) {
      let j = index(i32(g.x) + dx * step, i32(g.y) + dy * step);
      let cq = src[j];
      let sq = side_in[j];
      let k = kern[dx + 2] * kern[dy + 2];
      let wc = k * guide(i, j, f32(step)) * exp(-abs(l0 - lum(cq.xyz)) / sigma.x);
      sum += wc * cq;
      var_c += wc * wc * sq.x;
      wsum += wc;
      // Shading blends among pixels that see the void, by position and value.
      let wsh = k * (1.0 - hits(j)) * exp(-abs(s0.z - sq.z) / sigma.y);
      shade += wsh * sq.z;
      var_s += wsh * wsh * sq.y;
      ssum += wsh;
    }
  }
  // The pixel itself always counts toward its color; not always toward shading.
  dst[i] = select(vec4f((sum / wsum).xyz, c0.w), sum / wsum, shallow());
  var out = vec4f(var_c / (wsum * wsum), s0.yzw);
  if (ssum > 1e-12) { out = vec4f(out.x, var_s / (ssum * ssum), shade / ssum, 0.0); }
  side_out[i] = out;
}

@compute @workgroup_size(8, 8)
fn finish(@builtin(global_invocation_id) g: vec3u) {
  if (g.x >= D.dims.x || g.y >= D.dims.y) { return; }
  let i = g.y * D.dims.x + g.x;
  let c = src[i];
  let cover = clamp(c.w, 0.0, 1.0);
  let light = select(c.xyz * albedo(i), select(vec3f(0.0), c.xyz * albedo(i) / cover, cover > 1e-4), shallow());
  dst[i] = vec4f(light, side_in[i].z);
  side_out[i] = vec4f(0.0, 0.0, 0.0, 1.0 - cover);
}
