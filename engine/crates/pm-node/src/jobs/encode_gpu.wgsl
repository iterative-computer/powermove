@group(0) @binding(0) var source: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let v = textureLoad(source, vec2<i32>(p.xy), 0);
    // Match the RGBA8 compositor attachment / CPU round_ties_even boundary.
    // Opaque WebM composites premultiplied RGB over black and ignores alpha.
    return vec4(round(clamp(v.rgb, vec3(0.0), vec3(1.0)) * 255.0) / 255.0, 1.0);
}
