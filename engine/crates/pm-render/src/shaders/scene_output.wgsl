// r180's authored ACES Filmic curve, evaluated once on the resolved linear
// light after transparent surfaces have blended. Matrix constants are columns.
fn scene_aces(rgb: vec3<f32>, exposure: f32) -> vec3<f32> {
    let input = mat3x3<f32>(
        vec3<f32>(0.59719, 0.07600, 0.02840),
        vec3<f32>(0.35458, 0.90834, 0.13383),
        vec3<f32>(0.04823, 0.01566, 0.83777));
    let output = mat3x3<f32>(
        vec3<f32>(1.60475, -0.10208, -0.00327),
        vec3<f32>(-0.53108, 1.10813, -0.07276),
        vec3<f32>(-0.07367, -0.00605, 1.07602));
    let v = input * (rgb * (exposure / 0.6));
    let a = v * (v + vec3<f32>(0.0245786)) - vec3<f32>(0.000090537);
    let b = v * (0.983729 * v + vec3<f32>(0.4329510)) + vec3<f32>(0.238081);
    return clamp(output * (a / b), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Luminance-weighted resolved samples (`scene3d::karis`): color scaled by
// 1 / (1 + luma * exposure / 0.6), coverage as is. The weights' sum is the
// coverage less the stored color's scaled luma (Rec. 709).
fn scene_unweight(y: vec4<f32>, exposure: f32) -> vec4<f32> {
    if (y.a <= 0.0) { return vec4<f32>(0.0); }
    let weights = y.a - max(dot(y.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0) * (exposure / 0.6);
    return vec4<f32>(y.rgb * (y.a / max(weights, 1.0 / 16384.0)), y.a);
}

// A display background bypasses ACES but joins the object in linear light.
fn scene_output(stored: vec4<f32>, exposure: f32, background: vec4<f32>, linear: bool, weighted: bool) -> vec4<f32> {
    let src = select(stored, scene_unweight(stored, exposure), weighted);
    var rgb = vec3<f32>(0.0);
    if (src.a > 0.0) { rgb = scene_aces(src.rgb / src.a, exposure); }
    var result = vec4<f32>(rgb * src.a, src.a);
    if (background.w > 0.5) {
        result = vec4<f32>(result.rgb + (1.0 - src.a) * pm_e_decode3(background.rgb), 1.0);
    }
    if (!linear) { result = pm_premultiplied_linear_to_e(result); }
    return result;
}
