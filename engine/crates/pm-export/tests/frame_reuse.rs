//! An export reads its frames through one staging buffer and draws each into
//! the target the previous frame gave back (`Renderer::read_rgba_reusing`,
//! `Renderer::recycle`; docs/engine/perfbench.md). The pixels must be the
//! ones a fresh renderer draws: transparent and opaque backgrounds, whole
//! frames and banded ones, frames of different content one after another.

#![cfg(target_os = "macos")]

mod support;

use std::sync::atomic::AtomicBool;

use pm_export::render::*;
use pm_protocol::color::Precision;
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use pm_render::{RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;

fn seconds(s: f64) -> Ticks {
    Ticks::new((s * TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

/// Moving, rotating and fading solids over `background`, a group and a
/// precomp: every frame differs from the last.
fn document(background: serde_json::Value) -> pm_model::Document {
    let mut layers = Vec::new();
    for i in 0..12 {
        let color = ["#E8553A", "#3FA7D6", "#59CD90"][i % 3];
        let mut l = layer(
            &format!("s{i}"),
            "solid",
            json!({ "color": color, "w": 60 + i * 7, "h": 40 }),
            20.0 + 23.0 * i as f64,
            30.0 + 11.0 * i as f64,
            2.0,
        );
        set(
            &mut l,
            "position.x",
            keys(
                json!(0),
                &[(0.0, json!(10 + i * 20)), (2.0, json!(300 - i * 9))],
            ),
        );
        set(
            &mut l,
            "rotation",
            keys(json!(0), &[(0.0, json!(0)), (2.0, json!(90 + i * 10))]),
        );
        set(
            &mut l,
            "opacity",
            keys(json!(100), &[(0.0, json!(100)), (2.0, json!(35))]),
        );
        if i % 4 == 1 {
            l["group"] = json!("g");
        }
        layers.push(l);
    }
    let mut g = layer("g", "group", json!({}), 0.0, 0.0, 2.0);
    set(&mut g, "opacity", json!(80));
    layers.insert(0, g);
    let mut pre = layer(
        "pre",
        "precomp",
        json!({ "comp": "inner", "w": 160, "h": 90 }),
        200.0,
        120.0,
        2.0,
    );
    set(
        &mut pre,
        "scale.x",
        keys(json!(50), &[(0.0, json!(40)), (2.0, json!(90))]),
    );
    layers.insert(1, pre);
    let mut doc = project(320, 180, 2.0, background, layers);
    composition(
        &mut doc,
        "inner",
        160,
        90,
        2.0,
        vec![layer(
            "in",
            "solid",
            json!({ "color": "#F2A541", "w": 120, "h": 70 }),
            80.0,
            45.0,
            2.0,
        )],
    );
    load(&doc)
}

fn frames(
    renderer: &mut Renderer,
    doc: &pm_model::Document,
    request: &RenderRequest,
) -> Vec<Vec<f32>> {
    (0..8)
        .map(|n| {
            render_frame_reusing(
                renderer,
                doc,
                "Ptest-root",
                seconds(f64::from(n) * 0.25),
                request,
                &MediaPaths::default(),
                &Unrestricted,
                1,
                &AtomicBool::new(false),
            )
            .unwrap()
            .rgba
        })
        .collect()
}

#[test]
fn reused_targets_and_staging_draw_what_a_fresh_renderer_draws() {
    for background in [solid_fill("#14161C"), no_fill()] {
        let doc = document(background);
        for precision in [Precision::Float16, Precision::Float32] {
            let request = RenderRequest::sized(Size::new(320, 180), precision);
            let mut reused = Renderer::headless().unwrap();
            let kept = frames(&mut reused, &doc, &request);
            for (n, frame) in kept.iter().enumerate() {
                // A fresh renderer for every frame: nothing to reuse.
                let mut fresh = Renderer::headless().unwrap();
                let want = render_frame(
                    &mut fresh,
                    &doc,
                    "Ptest-root",
                    seconds(n as f64 * 0.25),
                    &request,
                    &MediaPaths::default(),
                    &Unrestricted,
                    1,
                    &AtomicBool::new(false),
                )
                .unwrap()
                .rgba;
                assert!(
                    frame
                        .iter()
                        .zip(&want)
                        .all(|(a, b)| a.to_bits() == b.to_bits())
                        && frame.len() == want.len(),
                    "frame {n} ({precision:?}) differs from a fresh renderer's"
                );
            }
            assert!(
                reused.frame_reuse_bytes() > 0,
                "an export's frames keep their staging"
            );
            reused.release_frame_reuse();
            assert_eq!(reused.frame_reuse_bytes(), 0);
        }
    }
}

/// A one-off frame (an agent still, a parity frame) and a trim leave nothing
/// kept for a next frame: no staging buffer, target or pixels idle in a job
/// renderer.
#[test]
fn one_off_frames_and_trims_keep_nothing() {
    let doc = document(solid_fill("#14161C"));
    let request = RenderRequest::sized(Size::new(320, 180), Precision::Float16);
    let mut renderer = Renderer::headless().unwrap();
    let one_off = |renderer: &mut Renderer, n: u32| {
        render_frame(
            renderer,
            &doc,
            "Ptest-root",
            seconds(f64::from(n) * 0.25),
            &request,
            &MediaPaths::default(),
            &Unrestricted,
            1,
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    one_off(&mut renderer, 0);
    assert_eq!(renderer.frame_reuse_bytes(), 0, "after a one-off frame");
    let mut stills = StillRouting::default();
    render_still(
        &mut renderer,
        &doc,
        "Ptest-root",
        seconds(0.5),
        320,
        &MediaPaths::default(),
        &mut stills,
        &Unrestricted,
        7,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(renderer.frame_reuse_bytes(), 0, "after an agent still");
    render_frame_reusing(
        &mut renderer,
        &doc,
        "Ptest-root",
        seconds(0.75),
        &request,
        &MediaPaths::default(),
        &Unrestricted,
        1,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(renderer.frame_reuse_bytes() > 0);
    renderer.trim(u64::MAX);
    assert_eq!(renderer.frame_reuse_bytes(), 0, "after a trim");
}
