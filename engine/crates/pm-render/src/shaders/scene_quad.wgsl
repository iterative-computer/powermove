// A linear 3D source reaches the same scene-output seam as before: ACES once
// after the scene's transparent blend and MSAA resolve, then the working
// encoding. The pipeline selector uses this only for scene draws.
@fragment
fn fs_scene(in: VsOut) -> @location(0) vec4<f32> {
    let c = sample_quad(in);
    return scene_output(c, q.scene.w, vec4<f32>(q.scene.rgb, q.color.w), q.color.z < 2.5) * q.color.x;
}
