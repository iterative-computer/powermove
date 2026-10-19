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
use pm_protocol::common::{Seq, Size, Ticks};
use pm_protocol::frame::{MediaPreference, PreviewResolution, ViewerState};
use pm_protocol::interop::SurfaceFormat;
use serde_json::json;

const W: u32 = 800;
const H: u32 = 600;

fn ch(v: serde_json::Value) -> serde_json::Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn doc(background: &str) -> pm_model::Document {
    let layer = json!({
        "id": "box", "type": "solid", "name": "box", "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(40)),
            "position.y": ch(json!(30)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
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
        "id": "P", "name": "p", "w": 160, "h": 90, "fps": 30, "dur": 2, "bg": "#000000",
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
    let document = doc(background);
    let s = state(resolution);
    let composed = composer
        .compose(
            &document,
            "P-root",
            &MediaMap::default(),
            Ticks::ZERO,
            &s,
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
    let place = placement(&s, 160.0, 90.0);
    // The overlay geometry names the layer where it is drawn.
    let layer = &composed.overlay.layers[0];
    assert_eq!(layer.layer.as_str(), "box");
    let scale = 1.5 * 2.0;
    assert!(
        (layer.bounds[0][0] - (place.x + 40.0 * scale)).abs() < 1e-6,
        "{layer:?}"
    );
    assert!(
        (layer.bounds[0][1] - (place.y + 30.0 * scale)).abs() < 1e-6,
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
