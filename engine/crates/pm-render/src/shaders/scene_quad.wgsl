// A linear 3D source reaches the same scene-output seam as before: ACES once
// after the scene's transparent blend and MSAA resolve, then the working
// encoding. The pipeline selector uses this only for scene draws.
fn scene_fragment(in: VsOut) -> vec4<f32> {
    let c = sample_quad(in);
    let flags = u32(q.color.w + 0.5);
    let background = vec4<f32>(q.scene.rgb, f32(flags & 1u));
    return scene_output(c, q.scene.w, background, (q.color.z < 2.5), ((flags & 2u) != 0u)) * q.color.x;
}

@fragment
fn fs_scene(in: VsOut) -> @location(0) vec4<f32> {
    return scene_fragment(in);
}

@fragment
fn fs_scene_half(in: VsOut) -> @location(0) vec4<f32> {
    return pm_scene_half_color(scene_fragment(in));
}
