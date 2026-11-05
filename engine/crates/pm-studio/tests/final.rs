//! Real GPU witnesses: unavailable adapters fail, never silently pass.
use pm_studio::{gpu::Renderer, scene::preset};

#[test]
fn seeded_final_guides_and_denoise_failures_are_observable() {
    let mut r = Renderer::new(64, 48).expect("Metal or Vulkan adapter required");
    eprintln!("Studio GPU witness: {}", r.adapter);
    let s = preset("hero", "space-gray", 0.).unwrap();
    r.set_scene(&s, &s.prepare());
    r.caustics = false;
    r.trace_seeded(8, 8, true, 64, 71);
    let raw = r.finish(0).0;
    let guided = r.finish_guided().0;
    assert_eq!(guided.len(), 64 * 48 * 4);
    assert!(guided.chunks_exact(4).any(|p| p[0] > 0 && p[0] != p[1]));
    assert_ne!(raw, guided, "the guided filter must execute");
    r.trace_seeded(8, 8, true, 64, 71);
    assert_eq!(raw, r.finish(0).0, "same frame seed must reproduce pixels");
    assert_eq!(
        guided,
        r.finish_guided().0,
        "guides share deterministic frame salt"
    );
    r.trace_seeded(8, 8, true, 64, 72);
    assert_ne!(raw, r.finish(0).0, "adjacent frames must receive new noise");
    let err = r
        .try_finish_with(&|_, _, _| Err("synthetic OIDN failure".into()))
        .unwrap_err();
    assert_eq!(err, "synthetic OIDN failure");
}

#[test]
fn zero_size_and_invalid_map_arrays_are_refused() {
    assert!(Renderer::new(0, 1).is_err());
    let mut r = Renderer::new(8, 8).expect("Metal or Vulkan adapter required");
    let maps = pm_studio::extract::TextureMaps {
        width: 2,
        height: 2,
        layers: vec![vec![255; 15]],
    };
    assert!(r.set_maps(&maps).is_err());
}
