@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let p = mix(it.rect.xy, it.rect.zw, a);
    return vec4<f32>(p.x / it.params.z * 2.0 - 1.0, 1.0 - p.y / it.params.w * 2.0, 0.0, 1.0);
}
