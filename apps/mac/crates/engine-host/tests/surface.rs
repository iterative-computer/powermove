//! Main-thread harness: the CoreText service must never run on a test thread.
use pm_mac_engine_host::{EngineHost, FrameHandle, MainThreadFonts, RenderRequest, Size, Ticks};
use pm_model::{
    Document, canonical,
    container::{ReadOptions, read_container_bytes},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn wait(host: &EngineHost, fonts: &mut MainThreadFonts, id: u64) -> FrameHandle {
    let start = Instant::now();
    loop {
        fonts.pump();
        if let Some(error) = host.last_error() {
            panic!("render failed: {error}");
        }
        if let Some(frame) = host.latest().filter(|f| f.request_id == id) {
            return frame;
        }
        assert!(
            start.elapsed() < Duration::from_secs(40),
            "render timed out for request {id}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn at(frame: &FrameHandle, x: u32, y: u32) -> [u8; 4] {
    // The engine's diagnostic readback, never used by interactive presentation.
    let bytes = frame.surface.iosurface.read();
    let offset = (y * frame.surface.iosurface.bytes_per_row() + x * 4) as usize;
    let bgra = &bytes[offset..offset + 4];
    [bgra[2], bgra[1], bgra[0], bgra[3]]
}
fn main() {
    let mut fonts = MainThreadFonts::new(Arc::new(|| {})).unwrap();
    let host = EngineHost::start(fonts.broker());
    let fixture = include_bytes!("../../panel-viewer/fixtures/frames.pmv");
    let container = read_container_bytes(fixture, ReadOptions { verify_media: true }).unwrap();
    let mut project = canonical::canonical_project(canonical::into_project(container.document));
    // Extract the fixture's actual embedded still for the engine resolver.
    let media_dir = std::env::temp_dir().join(format!("pm-viewer-test-{}", std::process::id()));
    std::fs::create_dir_all(&media_dir).unwrap();
    let path = media_dir.join("still.png");
    let record = &container.media[0];
    std::fs::write(
        &path,
        &fixture[record.offset as usize..(record.offset + record.length) as usize],
    )
    .unwrap();
    let mut assets = pm_render::MapResolver::default();
    assets.paths.insert("img".into(), path);
    let assets: Arc<dyn pm_render::AssetResolver> = Arc::new(assets);
    let document = Arc::new(Document::new(Some(project.clone()), 7));
    let mut request = RenderRequest {
        document,
        composition: "Cviewer".into(),
        time: Ticks::ZERO,
        size: Size::new(640, 360),
        assets,
    };
    let id = host.request(request.clone()).unwrap();
    let first = wait(&host, &mut fonts, id);
    assert_eq!(first.revision, 7);
    let bg = at(&first, 5, 5);
    assert_eq!(bg, [24, 33, 43, 255], "background/color/order");
    let shape = at(&first, 155, 170);
    assert_eq!(shape, [255, 107, 26, 255], "shape center");
    let image = at(&first, 420, 265);
    assert_eq!(image, [245, 180, 45, 255], "embedded image");
    // Text must have real ink above the background, rather than merely render
    // successfully after omitting an unsupported font layer.
    let pixels = first.surface.iosurface.read();
    let pitch = first.surface.iosurface.bytes_per_row() as usize;
    assert!(
        (40..110).any(|y| (180..460).any(|x| {
            let i = y * pitch + x * 4;
            pixels[i] > 150 && pixels[i + 1] > 150 && pixels[i + 2] > 150
        })),
        "text ink missing"
    );
    // Keep all three leases: the fourth request must stall without corruption.
    request.time = Ticks::new(pm_time::TICKS_PER_SECOND).unwrap();
    let id = host.request(request.clone()).unwrap();
    let second = wait(&host, &mut fonts, id);
    request.time = Ticks::new(2 * pm_time::TICKS_PER_SECOND).unwrap();
    let id = host.request(request.clone()).unwrap();
    let third = wait(&host, &mut fonts, id);
    assert_eq!(at(&first, 155, 170), shape);
    request.size = Size::new(320, 180);
    let id = host.request(request.clone()).unwrap();
    for _ in 0..15 {
        fonts.pump();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        host.latest().unwrap().request_id,
        third.request_id,
        "ring must not overwrite held slots"
    );
    drop(second);
    let resized = wait(&host, &mut fonts, id);
    assert!(resized.generation > first.generation);
    assert_eq!(resized.size, request.size);
    assert_eq!(
        at(&first, 155, 170),
        shape,
        "old generation stayed immutable"
    );
    drop(first);
    drop(third);
    drop(resized);
    // Validate semitransparency: render a simple 50% red shape over transparent.
    let mut json: serde_json::Value =
        serde_json::from_str(&pm_model::json::to_canonical_string(&project)).unwrap();
    json["comps"]["Cviewer"]["layers"]
        .as_array_mut()
        .unwrap()
        .retain(|l| l["id"] == "shape");
    json["comps"]["Cviewer"]["backgroundFill"]["type"] = "none".into();
    json["comps"]["Cviewer"]["layers"][0]["d"]["color"] = "#FF0000".into();
    json["comps"]["Cviewer"]["layers"][0]["p"]["opacity"]["v"] = 50.into();
    project = pm_model::json::parse(&json.to_string()).unwrap();
    request.document = Arc::new(Document::new(Some(project), 8));
    request.size = Size::new(640, 360);
    request.time = Ticks::ZERO;
    let id = host.request(request.clone()).unwrap();
    let alpha = wait(&host, &mut fonts, id);
    let red = at(&alpha, 155, 170);
    assert!(
        (red[0] as i32 - 128).abs() <= 1
            && red[1] == 0
            && red[2] == 0
            && (red[3] as i32 - 128).abs() <= 1,
        "premultiplied alpha: {red:?}"
    );
    assert_eq!(at(&alpha, 5, 5), [0, 0, 0, 0]);
    // A burst collapses to the final request, preserving its exact tick stamp.
    let mut id = 0;
    for i in 0..100 {
        request.time = Ticks::new(i * 100000).unwrap();
        id = host.request(request.clone()).unwrap();
    }
    drop(alpha);
    let last = wait(&host, &mut fonts, id);
    assert_eq!(last.time, request.time);
    // Invalid composition keeps the last good frame.
    request.composition = "missing".into();
    host.request(request).unwrap();
    let start = Instant::now();
    while host.last_error().is_none() {
        fonts.pump();
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(host.latest().unwrap().request_id, last.request_id);
    drop(host);
    drop(fonts);
    std::fs::remove_dir_all(media_dir).unwrap();
    println!(
        "PASS fixture pixels/text/image, alpha, bounded ring/resize, coalescing, last-good frame"
    );
}
