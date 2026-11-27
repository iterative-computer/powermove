use pm_studio::{gpu::Renderer, scene::preset};

#[test]
fn resized_targets_keep_the_scene_and_match_a_fresh_final_render() {
    let scene = preset("hero", "blue", 0.).unwrap();
    let prepared = scene.prepare();
    let Ok(mut resized) = Renderer::new(128, 72) else { return };
    resized.set_scene(&scene, &prepared);
    resized.trace(4, 4, true, 64);
    resized.finish(0);
    // Shrink, grow, then return to a previous size. No scene upload is repeated.
    for (width, height) in [(96, 54), (160, 90), (128, 72)] {
        assert!(resized.resize(width, height).unwrap());
        assert_eq!(resized.samples, 0);
        assert!(!resized.resize(width, height).unwrap());
        resized.trace(4, 4, true, 64);
        let actual = resized.finish(0).0;
        let mut fresh = Renderer::new(width, height).unwrap();
        fresh.set_scene(&scene, &prepared);
        fresh.trace(4, 4, true, 64);
        assert_eq!(actual, fresh.finish(0).0);
    }
    assert!(resized.resize(0, 72).is_err());
    assert_eq!(resized.size(), (128, 72));
}

#[cfg(all(feature = "metalfx", target_os = "macos"))]
#[test]
fn live_resolution_steps_keep_metalfx_output_at_the_requested_size() {
    let scene = preset("hero", "blue", 0.).unwrap();
    let Ok(mut renderer) = Renderer::new(64, 36) else { return };
    if !renderer.metalfx_enabled { return; }
    renderer.set_scene(&scene, &scene.prepare());
    renderer.live = true;
    for (width, height) in [(64, 36), (48, 27), (56, 32), (64, 36)] {
        renderer.resize(width, height).unwrap();
        renderer.trace(1, 3, true, 12);
        let pixels = renderer.finish_metalfx(128, 72, false, None).unwrap().0;
        assert_eq!(pixels.len(), 128 * 72 * 4);
        assert!(pixels.chunks_exact(4).any(|p| p[0] != p[1] || p[1] != p[2]));
    }
}
