//! A document frame for the viewer, through the display transform into a
//! real pool surface (Phase 2 tracer bullet; plan sections 8, 9, 18): the
//! composition lands where the legacy viewer places it, sampled 1:1 at Full
//! resolution with the colors pm-render drew, opaque inside its rectangle
//! (today's opaque canvas: a transparent background shows black) and
//! transparent around it, so the viewer's stage shows there; Half
//! resolution fills the same rectangle.
#![cfg(target_os = "macos")]

use pm_gpu::bridge_out::display::{DisplayParams, DisplayTransform, WorkingTexture};
use pm_gpu::bridge_out::pool::SurfacePool;
use pm_node::present::compose::{Composer, placement};
use pm_node::present::document::MediaMap;
use pm_protocol::color::{ViewerColorSpace, WorkingEncoding};
use pm_protocol::common::Rect;
use pm_protocol::common::{Seq, Size, Ticks};
use pm_protocol::frame::{MediaPreference, PreviewResolution, ViewerCanvas, ViewerState};
use pm_protocol::interop::SurfaceFormat;
use serde_json::json;

const W: u32 = 800;
const H: u32 = 600;

fn ch(v: serde_json::Value) -> serde_json::Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn doc(background: &str) -> pm_model::Document {
    doc_sized(background, (160, 90), (40.0, 30.0))
}

/// A `size` composition with a 40 × 20 red solid at `at`.
fn doc_sized(background: &str, size: (u32, u32), at: (f64, f64)) -> pm_model::Document {
    let layer = json!({
        "id": "box", "type": "solid", "name": "box", "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(at.0)),
            "position.y": ch(json!(at.1)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
        "d": { "color": "#FF0000", "w": 40, "h": 20 }, "locked_intent": {}
    });
    let fill = if background == "none" {
        json!({ "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] })
    } else {
        json!({ "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": background, "position": 0 }] })
    };
    let project = json!({
        "id": "P", "name": "p", "w": size.0, "h": size.1, "fps": 30, "dur": 2, "bg": "#000000",
        "backgroundFill": fill, "compId": "P-root", "compName": "p", "markers": [], "work": [0, 2],
        "params": {}, "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer],
        "assets": {}, "comps": {}
    });
    pm_model::Document::new(
        Some(pm_model::json::parse(&project.to_string()).unwrap()),
        1,
    )
}

fn state(resolution: PreviewResolution) -> ViewerState {
    ViewerState {
        headless: None,
        composition: "P-root".into(),
        size: Size::new(W, H),
        device_pixel_ratio: 2.0,
        zoom: 1.5,
        pan: [10.0, 0.0],
        resolution,
        media: MediaPreference::Automatic,
        color_space: ViewerColorSpace::Srgb,
        view: None,
        play_every_frame: false,
        edr: false,
        view_seq: Seq::new(1).unwrap(),
        vsync: None,
        depth_id: None,
        chrome_selection: None,
        canvas: None,
        surface_epoch: None,
    }
}

/// BGRA bytes of the surface at (x, y).
fn bgra(bytes: &[u8], bpr: usize, x: u32, y: u32) -> [u8; 4] {
    let p = &bytes[y as usize * bpr + x as usize * 4..];
    [p[0], p[1], p[2], p[3]]
}

fn draw(
    background: &str,
    resolution: PreviewResolution,
) -> (Vec<u8>, usize, pm_node::present::compose::Placement) {
    draw_state(background, &state(resolution), true)
}

fn draw_state(
    background: &str,
    s: &ViewerState,
    check_overlay: bool,
) -> (Vec<u8>, usize, pm_node::present::compose::Placement) {
    draw_doc(&doc(background), (160.0, 90.0), s, check_overlay)
}

fn draw_doc(
    document: &pm_model::Document,
    comp: (f64, f64),
    s: &ViewerState,
    check_overlay: bool,
) -> (Vec<u8>, usize, pm_node::present::compose::Placement) {
    let gpu = pm_gpu::Gpu::new("compose test").unwrap();
    let transform = DisplayTransform::new(&gpu.device, &gpu.queue);
    let working = WorkingTexture::new(&gpu.device, Size::new(W, H));
    let pool = SurfacePool::new(
        &gpu.device,
        1,
        Size::new(W, H),
        SurfaceFormat::Bgra8Unorm,
        4,
    )
    .unwrap();
    let mut composer = Composer::new(&gpu.device, &gpu.queue);
    let composed = composer
        .compose(
            document,
            "P-root",
            &MediaMap::default(),
            Ticks::ZERO,
            s,
            &working,
        )
        .unwrap();
    assert!(composed.failures.is_empty());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    transform
        .encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &working,
            &pool.surfaces[0],
            DisplayParams {
                working: WorkingEncoding::Standard,
                canvas: ViewerColorSpace::Srgb,
                noise_offset: [0, 0],
            },
        )
        .unwrap();
    let index = gpu.queue.submit([encoder.finish()]);
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: Some(std::time::Duration::from_secs(5)),
        })
        .unwrap();
    let surface = &pool.surfaces[0];
    let place = placement(s, comp.0, comp.1);
    // The overlay geometry names the layer where it is drawn.
    let layer = &composed.overlay.layers[0];
    assert_eq!(layer.layer.as_str(), "box");
    let scale = 1.5 * 2.0;
    assert!(
        !check_overlay || (layer.bounds[0][0] - (place.x + 40.0 * scale)).abs() < 1e-6,
        "{layer:?}"
    );
    assert!(
        !check_overlay || (layer.bounds[0][1] - (place.y + 30.0 * scale)).abs() < 1e-6,
        "{layer:?}"
    );
    (
        surface.iosurface.read(),
        surface.iosurface.bytes_per_row() as usize,
        place,
    )
}

#[test]
fn a_document_frame_is_placed_opaque_in_its_rectangle_and_transparent_around_it() {
    let (bytes, bpr, place) = draw("#3366CC", PreviewResolution::Full);
    // 160×90 at zoom 1.5 and DPR 2: 480×270 frame pixels, centered, panned 20 px.
    assert_eq!(place.output, Size::new(480, 270));
    assert_eq!((place.x, place.y), (180.0, 165.0));
    // Around the composition: transparent.
    assert_eq!(bgra(&bytes, bpr, 10, 10), [0, 0, 0, 0]);
    assert_eq!(bgra(&bytes, bpr, 179, 300), [0, 0, 0, 0]);
    assert_eq!(bgra(&bytes, bpr, 660, 300), [0, 0, 0, 0]);
    // Inside: the background and the layer, opaque, exact (1:1 sampling).
    assert_eq!(bgra(&bytes, bpr, 180, 165), [0xCC, 0x66, 0x33, 255]);
    assert_eq!(bgra(&bytes, bpr, 659, 434), [0xCC, 0x66, 0x33, 255]);
    let (lx, ly) = (180 + 40 * 3, 165 + 30 * 3);
    assert_eq!(bgra(&bytes, bpr, lx + 1, ly + 1), [0, 0, 255, 255]);
    assert_eq!(
        bgra(&bytes, bpr, lx + 40 * 3 - 2, ly + 20 * 3 - 2),
        [0, 0, 255, 255]
    );
    assert_eq!(
        bgra(&bytes, bpr, lx + 40 * 3 + 1, ly + 1),
        [0xCC, 0x66, 0x33, 255]
    );
}

#[test]
fn a_transparent_background_shows_black_and_half_resolution_fills_the_same_rectangle() {
    let (bytes, bpr, place) = draw("none", PreviewResolution::Half);
    assert_eq!(place.output, Size::new(240, 135));
    // Today's opaque canvas: a transparent composition background is black.
    assert_eq!(bgra(&bytes, bpr, 182, 167), [0, 0, 0, 255]);
    assert_eq!(bgra(&bytes, bpr, 657, 432), [0, 0, 0, 255]);
    assert_eq!(bgra(&bytes, bpr, 178, 300), [0, 0, 0, 0]);
    assert_eq!(bgra(&bytes, bpr, 662, 300), [0, 0, 0, 0]);
    // The layer, upscaled from half resolution, is red inside its interior.
    let (lx, ly) = (180 + 40 * 3, 165 + 30 * 3);
    assert_eq!(bgra(&bytes, bpr, lx + 20, ly + 10), [0, 0, 255, 255]);
}

/// Today's viewer canvas (decision V1): at 150% on a 2x display today's
/// canvas is the composition's size (`min(1, zoom × DPR)`), and the browser
/// scales it into its CSS box (here 3×), bilinearly, from its 8-bit pixels.
/// The engine renders the 160 × 90 canvas and scales it the same way.
#[test]
fn todays_canvas_is_rendered_at_its_size_and_scaled_bilinearly_into_its_box() {
    let mut s = state(PreviewResolution::Automatic);
    // Stage 400 × 300 CSS: the frame at ((400 − 240) / 2 + 10, (300 − 135) / 2).
    s.canvas = Some(ViewerCanvas {
        width: 160,
        height: 90,
        crop: Rect {
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 90.0,
        },
        css: Rect {
            x: 90.0,
            y: 82.5,
            width: 240.0,
            height: 135.0,
        },
    });
    let (bytes, bpr, place) = draw_state("#3366CC", &s, false);
    assert_eq!(place.output, Size::new(160, 90));
    assert!(place.canvas);
    assert_eq!((place.fx, place.fy), (3.0, 3.0));
    assert_eq!((place.x, place.y), (180.0, 165.0));
    // Outside the snapped box: transparent; inside its edges: the background.
    assert_eq!(bgra(&bytes, bpr, 179, 300), [0, 0, 0, 0]);
    assert_eq!(bgra(&bytes, bpr, 660, 300), [0, 0, 0, 0]);
    assert_eq!(bgra(&bytes, bpr, 180, 165), [0xCC, 0x66, 0x33, 255]);
    assert_eq!(bgra(&bytes, bpr, 659, 434), [0xCC, 0x66, 0x33, 255]);
    // The box's left edge at canvas texel 40 (frame 300): frame pixels sample
    // `u = (i + 0.5 − 180) / 3 − 0.5` between the background and red.
    for i in 297..304u32 {
        let u = (f64::from(i) + 0.5 - 180.0) / 3.0 - 0.5;
        let t = (u - 39.0).clamp(0.0, 1.0);
        let expect = |bg: f64, fg: f64| (bg * (1.0 - t) + fg * t).round() as i32;
        let px = bgra(&bytes, bpr, i, 165 + 40 * 3);
        let want = [expect(204.0, 0.0), expect(102.0, 0.0), expect(51.0, 255.0)];
        for c in 0..3 {
            assert!(
                (i32::from(px[c]) - want[c]).abs() <= 1,
                "frame x {i}: {px:?} against {want:?}"
            );
        }
    }
}

/// A fractional box (61% on a 2x display, panned): each edge snaps to the
/// nearest device pixel as Blink snaps a canvas's content box, and only the
/// canvas texels the visible pixels read are rendered.
#[test]
fn todays_canvas_box_snaps_each_edge_and_renders_what_is_visible() {
    let mut s = state(PreviewResolution::Automatic);
    s.zoom = 0.61;
    s.canvas = Some(ViewerCanvas {
        width: 160,
        height: 90,
        crop: Rect {
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 90.0,
        },
        css: Rect {
            x: -10.3,
            y: 20.26,
            width: 97.6,
            height: 54.9,
        },
    });
    let p = placement(&s, 160.0, 90.0);
    // −20.6 → −21, 174.6 → 175; 40.52 → 41, 150.32 → 150.
    assert_eq!((p.x, p.y), (-21.0, 41.0));
    assert!((p.fx - 196.0 / 160.0).abs() < 1e-12 && (p.fy - 109.0 / 90.0).abs() < 1e-12);
    let clip = p.clip.unwrap();
    assert_eq!((clip.x, clip.y, clip.width, clip.height), (0, 41, 175, 109));
    // Frame pixel 0 reads texels 16 and 17 (u = 21.5 / 1.225 − 0.5 = 17.05).
    let r = p.region.unwrap();
    assert_eq!((r.x, r.y, r.width, r.height), (17, 0, 143, 90));
}

/// Today's crop at 500% on a 2x display of a 1920 × 1080 composition,
/// panned to its right edge (`previewRenderViewport` for a 400 × 300 CSS
/// stage: a 656 × 556 CSS crop at composition (1788.86, 444.4), a 1312 ×
/// 1112 canvas). The output is today's canvas at today's full scale (10),
/// starting at the crop's unrounded origin (17,888.6, 4,444), so every
/// layer lands where today's does, at today's resolution (decision V1b).
#[test]
fn a_crop_far_from_the_origin_is_drawn_at_todays_scale_in_todays_box() {
    let mut s = state(PreviewResolution::Automatic);
    s.zoom = 5.0;
    s.canvas = Some(ViewerCanvas {
        width: 1312,
        height: 1112,
        crop: Rect {
            x: 1788.86,
            y: 444.4,
            width: 131.2,
            height: 111.2,
        },
        css: Rect {
            x: -256.0,
            y: -128.0,
            width: 656.0,
            height: 556.0,
        },
    });
    // The red solid covers composition (1880, 500) to (1920, 520): output
    // (911.4, 556) to (1311.4, 756), frame (399.4, 300) to (799.4, 500).
    let document = doc_sized("#3366CC", (1920, 1080), (1880.0, 500.0));
    let (bytes, bpr, place) = draw_doc(&document, (1920.0, 1080.0), &s, false);
    assert!(place.canvas);
    assert_eq!(place.output, Size::new(1312, 1112));
    let (ox, oy) = place.origin.unwrap();
    assert!((ox - 17888.6).abs() < 1e-6 && (oy - 4444.0).abs() < 1e-6);
    let red = [0, 0, 255, 255];
    let bg = [0xCC, 0x66, 0x33, 255];
    // A solid covers each texel by its exact area (quality-ledger.md, "Solid
    // corner radius"): texel 911 (911.4 to 912) is 0.6 red, at frame pixel 399, and
    // texel 1311 (1311 to 1311.4) 0.4 red, at frame 799. A 1:1 box samples
    // each texel exactly. The default 16-bit project composites into half
    // floats (one 2⁻¹¹ step in [0.5, 1)) and the display stores 8 bits
    // (0.5 CV), so each edge is within the sum of the f64 composite.
    let bound = 0.5 + 255.0 / 2048.0;
    let over = |c: f64| -> [f64; 4] {
        std::array::from_fn(|k| f64::from(red[k]) * c + f64::from(bg[k]) * (1.0 - c))
    };
    for (x, c) in [(399, 0.6), (799, 0.4)] {
        let got = bgra(&bytes, bpr, x, 400);
        let error = (0..4)
            .map(|k| (f64::from(got[k]) - over(c)[k]).abs())
            .fold(0.0, f64::max);
        assert!(error <= bound, "frame {x}: {got:?} vs {:?}", over(c));
    }
    assert_eq!(bgra(&bytes, bpr, 398, 400), bg);
    assert_eq!(bgra(&bytes, bpr, 400, 400), red);
    assert_eq!(bgra(&bytes, bpr, 798, 400), red);
    assert_eq!(bgra(&bytes, bpr, 800, 400), bg);
    assert_eq!(bgra(&bytes, bpr, 600, 300), red);
    assert_eq!(bgra(&bytes, bpr, 600, 299), bg);
    assert_eq!(bgra(&bytes, bpr, 600, 499), red);
    assert_eq!(bgra(&bytes, bpr, 600, 500), bg);
    assert_eq!(bgra(&bytes, bpr, 10, 10), bg);
}
