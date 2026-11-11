use gpui::{
    BackdropFilter, Bounds, ContentMask, DevicePixels, ScaledFilter, ScaledPixels, Scene, point,
    size,
};
use gpui_apple::metal_renderer::MetalRenderer;

/// These tests read the shared device's allocation counter: run one at a time.
static DEVICE_COUNTER: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn home_scene_releases_the_editors_full_window_filter_targets() {
    let _counter = DEVICE_COUNTER.lock().unwrap_or_else(|e| e.into_inner());
    let device = metal::Device::system_default().unwrap();
    let mut renderer = MetalRenderer::new_headless_with_device(Default::default(), device.clone());
    let target = size(DevicePixels(512), DevicePixels(512));
    let mut empty = Scene::default();
    empty.finish();
    let pixels = renderer.render_scene_to_image(&empty, target).unwrap();
    let baseline = device.current_allocated_size();
    let bounds = Bounds {
        origin: point(ScaledPixels(0.), ScaledPixels(0.)),
        size: size(ScaledPixels(512.), ScaledPixels(512.)),
    };
    let mut filtered = Scene::default();
    filtered.insert_primitive(BackdropFilter {
        bounds,
        content_mask: ContentMask {
            bounds,
            ..Default::default()
        },
        filters: [ScaledFilter::Blur(ScaledPixels(4.))].into_iter().collect(),
        opacity: 1.,
        ..Default::default()
    });
    filtered.finish();
    renderer.render_scene_to_image(&filtered, target).unwrap();
    assert!(device.current_allocated_size() > baseline + 1024 * 1024);
    let home = renderer.render_scene_to_image(&empty, target).unwrap();
    assert_eq!(pixels, home, "releasing old targets changed the next scene");
    assert!(
        device.current_allocated_size() <= baseline + 64 * 1024,
        "simple scene retained old filter targets: {} vs {baseline}",
        device.current_allocated_size()
    );
}

#[test]
fn an_idle_window_gives_back_its_targets_and_redraws_the_same_pixels() {
    let _counter = DEVICE_COUNTER.lock().unwrap_or_else(|e| e.into_inner());
    let device = metal::Device::system_default().unwrap();
    let mut renderer = MetalRenderer::new_headless_with_device(Default::default(), device.clone());
    let target = size(DevicePixels(512), DevicePixels(512));
    let mut empty = Scene::default();
    empty.finish();
    renderer.render_scene_to_image(&empty, target).unwrap();
    let baseline = device.current_allocated_size();
    let bounds = Bounds {
        origin: point(ScaledPixels(0.), ScaledPixels(0.)),
        size: size(ScaledPixels(512.), ScaledPixels(512.)),
    };
    let mut filtered = Scene::default();
    filtered.insert_primitive(BackdropFilter {
        bounds,
        content_mask: ContentMask {
            bounds,
            ..Default::default()
        },
        filters: [ScaledFilter::Blur(ScaledPixels(4.))].into_iter().collect(),
        opacity: 1.,
        ..Default::default()
    });
    filtered.finish();
    let first = renderer.render_scene_to_image(&filtered, target).unwrap();
    assert!(device.current_allocated_size() > baseline + 1024 * 1024);
    assert!(
        !renderer.release_idle_targets(std::time::Duration::from_secs(3600)),
        "a window that drew just now keeps its targets"
    );
    assert!(renderer.release_idle_targets(std::time::Duration::ZERO));
    assert!(
        device.current_allocated_size() <= baseline + 64 * 1024,
        "idle window retained its targets: {} vs {baseline}",
        device.current_allocated_size()
    );
    let again = renderer.render_scene_to_image(&filtered, target).unwrap();
    assert_eq!(first, again, "remade targets changed the scene");
}
