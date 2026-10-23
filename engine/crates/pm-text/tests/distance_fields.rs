//! Atlas bytes from the pinned Chromium 152 CPU generator. Regenerate with
//! pm-vector/tools/graphite-oracle/sdf.cpp, linked against pinned Skia.
#![cfg(feature = "render")]

#[test]
fn atlas_bytes_match_the_independent_skia_generator() {
    let expected = [
        0xdd74d7ddee8b02af,
        0xf12a523940845411,
        0x9ee193d0a0802ea7,
        0x3a47325410160733,
        0x739385b08c66e369,
        0x468c646d3a05012d,
        0x19fee4017130fe66,
        0x580232e2d9dfb1c6,
    ];
    for (seed, expected) in expected.into_iter().enumerate() {
        let (w, h) = (11 + seed * 3, 9 + seed * 2);
        let a8: Vec<u8> = (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| ((x * 17 + y * 43 + x * y * 7 + seed * 31) & 255) as u8)
            })
            .collect();
        let df = pm_text::render::sdf::distance_field(&a8, w, h);
        let actual = df.into_iter().fold(14695981039346656037u64, |hash, b| {
            (hash ^ u64::from(b)).wrapping_mul(1099511628211)
        });
        assert_eq!(actual, expected, "seed {seed}, {w}x{h}");
    }
}
