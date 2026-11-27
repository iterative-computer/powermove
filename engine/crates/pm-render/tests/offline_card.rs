//! The Media Offline card (docs/native/design/media-offline.md): frames that
//! isolate failing layers draw it in the layer's box; any other render keeps
//! reporting the offline media as a layer failure.
mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;
use support::ticks;

const W: u32 = 192;
const H: u32 = 108;

fn offline_doc() -> pm_model::Document {
    // A 96 × 54 video layer centred in a 192 × 108 composition, its asset
    // known to the project but not to the resolver: offline.
    let layer = layer(
        "clip",
        "video",
        json!({"asset": "missing", "w": 96, "h": 54}),
        96.0,
        54.0,
        2.0,
    );
    let mut doc = project(W, H, 2.0, no_fill(), vec![layer]);
    asset(&mut doc, "missing", "video", "mp4", 96, 54, 2.0);
    doc["assets"]["missing"]["name"] = json!("interview take 3.mov");
    load(&doc)
}

fn alpha_at(rgba: &[f32], x: u32, y: u32) -> f32 {
    rgba[((y * W + x) * 4 + 3) as usize]
}

#[test]
fn isolating_frames_draw_the_card_in_the_layer_box() {
    let doc = offline_doc();
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_isolate_layers(true);
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .expect("render");
    assert!(
        frame.failures.is_empty(),
        "the card names the file: {:?}",
        frame.failures
    );
    let rgba = renderer.read_rgba(&frame).unwrap();
    // Opaque inside the layer's box, untouched outside it.
    assert!(alpha_at(&rgba, 96, 54) > 0.99);
    assert!(alpha_at(&rgba, 52, 30) > 0.99);
    assert!(alpha_at(&rgba, 10, 10) < 0.01);
    assert!(alpha_at(&rgba, 180, 100) < 0.01);
}

#[test]
fn other_renders_keep_failing_offline_media() {
    let doc = offline_doc();
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let mut renderer = Renderer::headless().unwrap();
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .expect("render");
    assert_eq!(frame.failures.len(), 1);
    assert!(frame.failures[0].message.contains("offline"));
    let rgba = renderer.read_rgba(&frame).unwrap();
    assert!(alpha_at(&rgba, 96, 54) < 0.01);
}

/// Media the host is still looking up (just imported or relinked) is not
/// offline: an isolating frame draws nothing for it and reports no failure.
#[test]
fn media_still_resolving_draws_no_card_and_no_failure() {
    struct Resolving;
    impl pm_render::AssetResolver for Resolving {
        fn path(&self, _: &str, _: Option<&pm_model::Node>) -> Option<std::path::PathBuf> {
            None
        }
        fn resolving(&self, asset: &str) -> bool {
            asset == "missing"
        }
    }
    let doc = offline_doc();
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_isolate_layers(true);
    let frame = renderer
        .render(&doc, "Ptest-root", ticks(0.0), &request, &Resolving)
        .expect("render");
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let rgba = renderer.read_rgba(&frame).unwrap();
    assert!(alpha_at(&rgba, 96, 54) < 0.01, "no card in the layer's box");
}
