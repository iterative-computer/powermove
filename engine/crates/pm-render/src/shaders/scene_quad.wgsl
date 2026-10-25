// A linear 3D source reaches the same scene-output seam as before: ACES once
// after the scene's transparent blend and MSAA resolve, then the working
// encoding. The pipeline selector uses this only for scene draws.
fn scene_fragment(in: VsOut) -> vec4<f32> {
    let c = sample_quad(in);
    return scene_output(c, q.scene.w, vec4<f32>(q.scene.rgb, q.color.w), (q.color.z < 2.5), (q.res.z > 0.5)) * q.color.x;
}

@fragment
fn fs_scene(in: VsOut) -> @location(0) vec4<f32> {
    return scene_fragment(in);
}

@fragment
fn fs_scene_half(in: VsOut) -> @location(0) vec4<f32> {
    return pm_scene_half_color(scene_fragment(in));
}
