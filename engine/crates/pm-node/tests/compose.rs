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
use pm_render::{OutputKind, RenderRequest, Renderer};
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
    doc_sized_with(background, size, at, 100, 8)
}

fn doc_sized_with(
    background: &str,
    size: (u32, u32),
    at: (f64, f64),
    opacity: u32,
    color_depth: u32,
) -> pm_model::Document {
    let layer = json!({
        "id": "box", "type": "solid", "name": "box", "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(at.0)),
            "position.y": ch(json!(at.1)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(opacity)), "skew": ch(json!(0))
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
        "id": "P", "name": "p", "w": size.0, "h": size.1, "fps": 30, "dur": 2, "bg": "#000000", "colorDepth": color_depth,
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

/// Full resolution uses the device box even if a legacy caller sends the
/// old backing-canvas dimensions. The edge is rendered at its final size.
#[test]
fn full_viewer_uses_device_box_with_legacy_canvas_metadata() {
    let mut s = state(PreviewResolution::Full);
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
    let (pixels, bpr, place) = draw_state("#3366CC", &s, false);
    assert_eq!(place.output, Size::new(480, 270));
    assert_eq!((place.fx, place.fy), (1.0, 1.0));
    assert_eq!(bgra(&pixels, bpr, 299, 255), [0xCC, 0x66, 0x33, 255]);
    assert_eq!(bgra(&pixels, bpr, 300, 255), [0, 0, 255, 255]);
}

#[test]
fn sixteen_bit_viewer_keeps_sub_byte_values_until_display() {
    let gpu = pm_gpu::Gpu::new("viewer precision test").unwrap();
    let working = WorkingTexture::new(&gpu.device, Size::new(W, H));
    let document = doc_sized_with("#000000", (160, 90), (40.0, 30.0), 32, 16);
    let mut composer = Composer::new(&gpu.device, &gpu.queue);
    let composed = composer
        .compose(
            &document,
            "P-root",
            &MediaMap::default(),
            Ticks::ZERO,
            &state(PreviewResolution::Full),
            &working,
        )
        .unwrap();
    assert!(composed.failures.is_empty());
    let rgba = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &working.texture).unwrap();
    let red = rgba[((255 * W + 320) * 4) as usize];
    // 32% is separated from the nearest 8-bit code by about 0.00157;
    // 0.31 was too close to 79/255 after the half-float store.
    let authored = 0.32_f32;
    let nearest_byte = (authored * 255.0).round() / 255.0;
    assert!((red - authored).abs() < 0.00025, "working red = {red}");
    assert!(
        (red - nearest_byte).abs() > 0.001,
        "working red {red} passed through RGBA8"
    );

    // The viewer's Full box is 480 × 270 at this zoom and DPR. With no
    // view-only override or motion blur, every working pixel must be the
    // same as an export requested at that output size.
    let mut renderer = Renderer::new(gpu.device.clone(), gpu.queue.clone());
    let request = RenderRequest {
        export: true,
        output_kind: OutputKind::Final,
        ..RenderRequest::sized(Size::new(480, 270), pm_protocol::color::Precision::Float16)
    };
    let frame = renderer
        .render(
            &document,
            "P-root",
            Ticks::ZERO,
            &request,
            &MediaMap::default(),
        )
        .unwrap();
    let exported = renderer.read_rgba(&frame).unwrap();
    for y in 0..270u32 {
        for x in 0..480u32 {
            let viewer = (((y + 165) * W + x + 180) * 4) as usize;
            let output = ((y * 480 + x) * 4) as usize;
            assert_eq!(
                &rgba[viewer..viewer + 4],
                &exported[output..output + 4],
                "viewer/export at {x}, {y}"
            );
        }
    }
}
