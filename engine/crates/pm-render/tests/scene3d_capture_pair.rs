//! Applies the production 3D comparison bar to an independently captured
//! Chromium/native pair. The private images are supplied by the caller.
mod support;

#[test]
#[ignore = "requires PM_SCENE3D_CAPTURE_PAIR from the paired app capture"]
fn captured_pair_meets_the_existing_3d_bar() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("PM_SCENE3D_CAPTURE_PAIR").expect("paired capture directory"),
    );
    let (width, height, reference) = support::read_png(&dir.join("legacy.png"));
    let (nw, nh, native) = support::read_png(&dir.join("native.png"));
    assert_eq!((width, height), (nw, nh));
    let object: Vec<bool> = reference.chunks_exact(4).map(|p| p[3] != 0).collect();
    assert!(object.iter().filter(|&&occupied| occupied).count() > 100);
    // With no AA0 reference, no edge is excluded. This is stricter than
    // the existing bar, and avoids deriving an exclusion from native pixels.
    let edges = vec![false; object.len()];
    let result = pm_render::parity::bar_3d(
        &reference,
        &native,
        &edges,
        &object,
        width as usize,
        height as usize,
    );
    eprintln!(
        "within2CV={} maxCV={} maxOutsideEdges={} objectSSIM={:?}",
        result.fraction_within_2, result.max_cv, result.max_cv_outside_edges, result.object_ssim
    );
    assert!(result.pass(), "paired capture exceeds the existing 3D bar");
}
