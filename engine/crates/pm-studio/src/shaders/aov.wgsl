// Guide images for the Final denoiser, traced with the same camera: first-hit
// albedo and normal per pixel from 16 jittered rays (enough for shallow
// focus). Albedo: the product's, averaged over the rays that hit it; its w,
// their share (the floor and the void are not the product). Normal: averaged
// over all rays (the floor's is up); its w, the hits' mean distance. Appended to
// trace.wgsl by each host, which declares `aov` (2 vec4f a pixel) at a binding
// of its own: it reads only the scene, never the accumulation buffers.

@compute @workgroup_size(8, 8)
fn aovs(@builtin(global_invocation_id) gid: vec3u) {
  let w = P.dims.x;
  let h = P.dims.y;
  if (gid.x >= w || gid.y >= h) { return; }
  let idx = gid.y * w + gid.x;
  // Seeded by the frame, like the trace, so lens blur in the guides moves too.
  seed = pcg(idx ^ pcg(P.dims.z ^ 0x5bd1e995u));
  var albedo = vec3f(0.0);
  var normal = vec3f(0.0);
  var hits = 0.0;
  var dist = 0.0;
  for (var k = 0; k < 16; k++) {
    let ray = camera_ray(vec2f(f32(gid.x), f32(gid.y)) + rnd2());
    let hit = intersect(ray[0], ray[1], BIG, false);
    let tf = floor_t(ray[0], ray[1]);
    if (tf < hit.t) {
      normal += vec3f(0.0, 1.0, 0.0);
    } else if (hit.tri >= 0) {
      let sf = surface(ray[0], ray[1], hit);
      // Glass, mirrors and glowing screens: white (a denoiser expects it for
      // specular surfaces; a screen's light is its own, not its glass's color's).
      let clear = sf.m.spec.w * (1.0 - sf.m.base.w) > 0.5 || (sf.m.base.w > 0.5 && sf.m.spec.x < 0.1) || any(sf.m.emit.xyz > vec3f(0.0));
      albedo += select(sf.m.base.xyz, vec3f(1.0), clear);
      normal += select(sf.n, -sf.n, dot(sf.n, ray[1]) > 0.0);
      hits += 1.0;
      dist += hit.t;
    }
  }
  aov[2u * idx] = vec4f(albedo / max(hits, 1.0), hits / 16.0);
  aov[2u * idx + 1u] = vec4f(normal / 16.0, dist / max(hits, 1.0));
}
