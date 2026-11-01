//! Real engine→IOSurface→CoreVideo→GPUI Metal test, without a visible window.
use gpui::{
    Bounds, ContentMask, DevicePixels, PaintSurface, Quad, ScaledPixels, Scene, SurfaceCompletion,
    point, size,
};
use gpui_apple::metal_renderer::{InstanceBufferPool, MetalRenderer};
use pm_mac_engine_host::{EngineHost, FrameHandle, MainThreadFonts, RenderRequest, Size, Ticks};
use pm_mac_panel_viewer::{Placement, Viewport, pixel_buffer};
use pm_model::{
    Document, canonical,
    container::{ReadOptions, read_container_bytes},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Debug)]
struct Probe {
    frame: FrameHandle,
    completions: Arc<AtomicUsize>,
}
impl SurfaceCompletion for Probe {
    fn metal_registry_id(&self) -> Option<u64> {
        Some(self.frame.metal_registry_id)
    }
    fn gpu_completed(&self) {
        self.frame.consumer_completed();
        self.completions.fetch_add(1, Ordering::SeqCst);
    }
}
fn wait(host: &EngineHost, fonts: &mut MainThreadFonts, id: u64) -> FrameHandle {
    let start = Instant::now();
    loop {
        fonts.pump();
        assert!(host.last_error().is_none(), "{:?}", host.last_error());
        if let Some(frame) = host.latest().filter(|f| f.request_id == id) {
            return frame;
        }
        assert!(start.elapsed() < Duration::from_secs(40));
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds::new(point(x.into(), y.into()), size(w.into(), h.into()))
}
fn main() {
    let mut fonts = MainThreadFonts::new(Arc::new(|| {})).unwrap();
    let host = EngineHost::start(fonts.broker());
    let bytes = include_bytes!("../fixtures/frames.pmv");
    let container = read_container_bytes(bytes, ReadOptions { verify_media: true }).unwrap();
    let project = canonical::canonical_project(canonical::into_project(container.document));
    let media = tempfile::tempdir().unwrap();
    let image_path = media.path().join("still.png");
    let m = &container.media[0];
    std::fs::write(
        &image_path,
        &bytes[m.offset as usize..(m.offset + m.length) as usize],
    )
    .unwrap();
    let mut resolver = pm_render::MapResolver::default();
    resolver.paths.insert("img".into(), image_path);
    let mut request = RenderRequest {
        document: Arc::new(Document::new(Some(project.clone()), 10)),
        composition: "Cviewer".into(),
        time: Ticks::ZERO,
        size: Size::new(640, 360),
        assets: Arc::new(resolver),
    };
    let id = host.request(request.clone()).unwrap();
    let first = wait(&host, &mut fonts, id);
    let mut renderer = MetalRenderer::new_headless(Arc::new(parking_lot::Mutex::new(
        InstanceBufferPool::default(),
    )));
    let output = size(DevicePixels(800), DevicePixels(600));
    let placement = Placement::fit(
        [0.0, 0.0, 800.0, 600.0],
        [640, 360],
        Viewport::default(),
        2.0,
    )
    .unwrap();
    let completions = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(Probe {
        frame: first.clone(),
        completions: completions.clone(),
    });
    let weak = Arc::downgrade(&probe);
    let mut scene = Scene::default();
    let clip = ContentMask {
        bounds: bounds(20.0, 90.0, 760.0, 420.0),
    };
    scene.insert_primitive(PaintSurface {
        order: 0,
        bounds: bounds(
            placement.origin[0],
            placement.origin[1],
            placement.size[0],
            placement.size[1],
        ),
        content_mask: clip,
        image_buffer: pixel_buffer(&first).unwrap(),
        completion: Some(probe.clone()),
    });
    scene.insert_primitive(Quad {
        bounds: bounds(100.0, 130.0, 20.0, 20.0),
        content_mask: ContentMask {
            bounds: bounds(0.0, 0.0, 800.0, 600.0),
        },
        background: gpui::rgb(0x00ff00).into(),
        ..Default::default()
    });
    scene.finish();
    drop(probe);
    let image = renderer.render_scene_to_image(&scene, output).unwrap();
    assert_eq!(
        image.get_pixel(194, 288).0,
        [255, 107, 26, 255],
        "BGRA channels / top-down shape"
    );
    assert_eq!(
        image.get_pixel(525, 406).0,
        [245, 180, 45, 255],
        "embedded still"
    );
    assert_eq!(
        image.get_pixel(100, 200).0,
        [24, 33, 43, 255],
        "sRGB background"
    );
    assert_eq!(image.get_pixel(10, 200).0, [0, 0, 0, 255], "left clip");
    assert_eq!(image.get_pixel(400, 80).0, [0, 0, 0, 255], "top clip");
    assert_eq!(
        image.get_pixel(110, 140).0,
        [0, 255, 0, 255],
        "GPUI overlay above surface"
    );
    assert_eq!(completions.load(Ordering::SeqCst), 1, "consumer completion");
    assert!(first.display_latency().is_some());
    // Paused scenes can be submitted repeatedly. One completion must not free
    // a source still referenced by a reusable scene or another command buffer.
    renderer.render_scene_to_image(&scene, output).unwrap();
    assert_eq!(completions.load(Ordering::SeqCst), 2);
    assert!(
        weak.upgrade().is_some(),
        "scene retains its lease after completion"
    );
    renderer.render_scene(&scene, output).unwrap();
    drop(scene);
    let start = Instant::now();
    while weak.upgrade().is_some() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        completions.load(Ordering::SeqCst),
        3,
        "async command retained callback through completion"
    );
    let first_latency = first.display_latency().unwrap();
    drop(first);
    // A later tick travels through the same host/ring and Metal surface path.
    request.time = Ticks::new(pm_time::TICKS_PER_SECOND).unwrap();
    let id = host.request(request.clone()).unwrap();
    let second = wait(&host, &mut fonts, id);
    let mut scene = Scene::default();
    scene.insert_primitive(PaintSurface {
        order: 0,
        bounds: bounds(0.0, 0.0, 640.0, 360.0),
        content_mask: ContentMask {
            bounds: bounds(0.0, 0.0, 640.0, 360.0),
        },
        image_buffer: pixel_buffer(&second).unwrap(),
        completion: Some(Arc::new(Probe {
            frame: second.clone(),
            completions: completions.clone(),
        })),
    });
    scene.finish();
    let image = renderer
        .render_scene_to_image(&scene, size(DevicePixels(640), DevicePixels(360)))
        .unwrap();
    assert_eq!(
        image.get_pixel(155, 170).0,
        [24, 33, 43, 255],
        "old animated position cleared"
    );
    assert_eq!(
        image.get_pixel(237, 170).0,
        [255, 107, 26, 255],
        "new tick drawn"
    );
    drop(scene);
    drop(second);
    // Prove premultiplied source-over on the actual GPUI RGB pipeline: a 50%
    // red shape over blue-grey must not be multiplied by alpha a second time.
    let mut json: serde_json::Value =
        serde_json::from_str(&pm_model::json::to_canonical_string(&project)).unwrap();
    let c = &mut json["comps"]["Cviewer"];
    c["layers"]
        .as_array_mut()
        .unwrap()
        .retain(|l| l["id"] == "shape");
    c["backgroundFill"]["type"] = "none".into();
    c["layers"][0]["d"]["color"] = "#FF0000".into();
    c["layers"][0]["p"]["opacity"]["v"] = 50.into();
    request.document = Arc::new(Document::new(
        Some(pm_model::json::parse(&json.to_string()).unwrap()),
        11,
    ));
    request.time = Ticks::ZERO;
    let id = host.request(request).unwrap();
    let alpha = wait(&host, &mut fonts, id);
    let mut scene = Scene::default();
    let mask = ContentMask {
        bounds: bounds(0.0, 0.0, 640.0, 360.0),
    };
    scene.insert_primitive(Quad {
        bounds: mask.bounds,
        content_mask: mask,
        background: gpui::rgb(0x204060).into(),
        ..Default::default()
    });
    scene.insert_primitive(PaintSurface {
        order: 0,
        bounds: mask.bounds,
        content_mask: mask,
        image_buffer: pixel_buffer(&alpha).unwrap(),
        completion: Some(Arc::new(Probe {
            frame: alpha.clone(),
            completions: completions.clone(),
        })),
    });
    scene.finish();
    let image = renderer
        .render_scene_to_image(&scene, size(DevicePixels(640), DevicePixels(360)))
        .unwrap();
    let actual = image.get_pixel(155, 170).0;
    assert!(
        actual
            .iter()
            .zip([144u8, 32, 48, 255])
            .all(|(&a, b)| (a as i32 - b as i32).abs() <= 1),
        "premultiplied GPUI blend: {actual:?}"
    );
    assert_eq!(
        image.get_pixel(5, 5).0,
        [32, 64, 96, 255],
        "transparent source reveals underlay"
    );
    println!(
        "PASS engine→RGB CoreVideo→GPUI Metal pixels, clipping, overlays, alpha, repeated/async completion; first request→GPU consumer {:.2} ms",
        first_latency.as_secs_f64() * 1000.0
    );
}
