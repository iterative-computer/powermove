// Present one layer's pixels from a shared trace: a full-screen triangle that
// reads the composed display buffer and the first-hit object ids (gbuf.z), keeping
// only `object` (0 keeps everything). Output is premultiplied RGBA, upright.

// dims: width, height, object id + 1 (0: all), flags (1: flip y, 2: include the void)
struct Present { dims: vec4u }

@group(0) @binding(0) var<uniform> Q: Present;
@group(0) @binding(1) var<storage, read> display: array<vec4f>;
@group(0) @binding(2) var<storage, read> gbuf: array<vec4f>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) frag: vec4f) -> @location(0) vec4f {
  let x = min(u32(frag.x), Q.dims.x - 1u);
  var y = min(u32(frag.y), Q.dims.y - 1u);
  if ((Q.dims.w & 1u) != 0u) { y = Q.dims.y - 1u - y; }
  let i = y * Q.dims.x + x;
  let c = display[i];
  let id = u32(gbuf[i].z + 0.5);
  if (Q.dims.z != 0u && id != Q.dims.z) {
    // Another layer's object; the void only for the layer that owns the backdrop.
    if (id != 0u || (Q.dims.w & 2u) == 0u) { return vec4f(0.0); }
  }
  return c;
}
