//! Exercises the production native chrome passes, including a selected mesh
//! behind an occluder. Transport values must survive half-float readback.
mod support;
use pm_gpu::Target;
use pm_protocol::frame::ChromeSelection;
use pm_render::{MapResolver, Renderer};
use serde_json::json;
use support::{doc::*, ticks};

#[test]
fn native_depth_ids_and_unoccluded_selection_share_one_view() {
    let (w, h) = (160, 90);
    let behind = object3d("behind", "box", json!({}), "#00FF00", w, h, 2.0);
    let mut front = object3d("front", "box", json!({}), "#FF0000", w, h, 2.0);
    set(&mut front, "position.z", json!(0.75));
    let doc = load(&project(
        w,
        h,
        2.0,
        solid_fill("#000000"),
        vec![camera("cam", [0.0, 0.0, 3.0], false, 2.0), behind, front],
    ));
    let mut r = Renderer::headless().unwrap();
    let target = |name| {
        Target::new(
            r.device(),
            name,
            w,
            h,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        )
    };
    let depth = target("viewport depth packet");
    let coverage = target("viewport object IDs and selection");
    let (legend, glyphs, selection) = r
        .render_viewport_chrome(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &MapResolver::default(),
            None,
            Some(&ChromeSelection {
                selected: vec!["behind".into()],
                active: vec![],
            }),
            (w, h),
            [0.0, 0.0, f64::from(w), f64::from(h)],
            &depth.view,
            &coverage.view,
        )
        .unwrap();
    assert_eq!(legend, vec!["behind", "front"]);
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].id, "cam");
    assert_eq!(selection.selected, vec!["behind"]);
    let d = pm_gpu::readback::read_f32(r.device(), r.queue(), &depth.texture).unwrap();
    let c = pm_gpu::readback::read_f32(r.device(), r.queue(), &coverage.texture).unwrap();
    let at = ((h / 2 * w + w / 2) * 4) as usize;
    let bits = (d[at] * 2048.0).round() as u32
        | ((d[at + 1] * 2048.0).round() as u32) << 11
        | ((d[at + 2] * 2048.0).round() as u32) << 22;
    let window_depth = f32::from_bits(bits);
    assert!(
        window_depth > 0.0 && window_depth < 1.0,
        "finite native window depth: {window_depth}"
    );
    let id = (c[at] * 2048.0).round() as usize | ((c[at + 1] * 2048.0).round() as usize) << 11;
    assert_eq!(legend[id - 1], "front", "topmost ID");
    assert_eq!(
        c[at + 2],
        0.5,
        "selection behind the occluder is still covered"
    );
    assert_eq!(c[3], 1.0);
}
