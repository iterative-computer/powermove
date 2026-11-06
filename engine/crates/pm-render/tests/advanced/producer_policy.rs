//! E02/E08: producer policies use the pinned stage/space contract.
use super::*;
#[test]
fn e02_matted_hidden_producer_source_masks_effects_and_source_space() {
    let mut renderer = Renderer::headless().unwrap();
    for space in ["composition", "source"] {
        for stage in ["source", "masks", "effects"] {
            let mut value = fixture(
                "setMatte",
                1,
                json!({"channel":0,"composite":false}),
                json!({"map":{"layerId":"provider","stage":stage,"space":space}}),
                None,
            );
            let mut provider = layer(
                "provider",
                "solid",
                json!({"w":32,"h":32,"color":"#FFFFFF"}),
                0.,
                0.,
                1.,
            );
            provider["on"] = json!(false);
            provider["matteSource"] = json!("clip");
            provider["matteMode"] = json!("alpha");
            let mut matte = layer(
                "clip",
                "solid",
                json!({"w":16,"h":32,"color":"#FFFFFF"}),
                0.,
                0.,
                1.,
            );
            matte["on"] = json!(false);
            value["layers"]
                .as_array_mut()
                .unwrap()
                .extend([provider, matte]);
            let actual = draw(&mut renderer, &value, &MapResolver::default(), 32);
            for (i, p) in actual.chunks_exact(4).enumerate() {
                // Source raster stages precede track matte, transform and opacity.
                // Composition effects include track matte only for a complete
                // producer, after masks/effects; earlier prefixes omit it.
                let expected = if space == "composition" && stage == "effects" && i % 32 >= 16 {
                    0.
                } else {
                    1.
                };
                assert_eq!(p[3], expected, "{space}/{stage} at {}:{}", i % 32, i / 32);
            }
        }
    }
}
