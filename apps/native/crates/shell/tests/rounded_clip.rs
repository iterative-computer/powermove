#![cfg(target_os = "macos")]
//! A rounded element that hides its overflow clips its children to its rounded
//! (continuous) shape: an opaque square child does not show in the corners.
//! Real GPUI tree + the headless Metal renderer, no AppKit windows.
use gpui::{
    AppContext, Context, HeadlessAppContext, IntoElement, ParentElement, Render, Styled, Window,
    div, px, rgb, size,
};
use pm_design::{Accessibility, Scheme, Tokens};
use std::sync::Arc;

const GROUND: u32 = 0x202020;
const CHILD: u32 = 0xf0f0f0;

struct Clipped {
    radius: f32,
    smoothing: f32,
    border: bool,
    /// The child sits in a square clip inset a point inside the group (a
    /// scroll view in a panel), not straight in the group.
    inset: bool,
}

impl Render for Clipped {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let group = div()
            .absolute()
            .left(px(20.))
            .top(px(20.))
            .size(px(160.))
            .rounded(px(self.radius))
            .rounded_smoothing(self.smoothing)
            .overflow_hidden();
        let group = if self.border {
            group.border(px(4.)).border_color(rgb(0x3060c0))
        } else {
            group
        };
        // An opaque square child with sharp corners: straight in the group,
        // or in a square clip inset inside it. Either way it takes the
        // group's corner.
        let child = div().size_full().bg(rgb(CHILD));
        let body = if self.inset {
            div()
                .size_full()
                .p(px(1.))
                .child(div().size_full().overflow_hidden().child(child))
        } else {
            child
        };
        div().size_full().bg(rgb(GROUND)).child(group.child(body))
    }
}

fn capture(view: Clipped) -> (image::RgbaImage, u32) {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(
            gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                .with_automatic_optical_sizing(),
        ),
        Arc::new(()),
        || Some(Box::new(gpui_apple::metal_renderer::MetalHeadlessRenderer::new())),
    );
    let window = cx
        .open_window(size(px(200.), px(200.)), |_, cx| cx.new(|_| view))
        .unwrap();
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into()).unwrap();
    let scale = image.width() / 200;
    (image, scale)
}

fn rgb3(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

#[test]
fn rounded_overflow_clips_children_to_the_corner() {
    objc2::rc::autoreleasepool(|_| {
        let t = Tokens::new(Scheme::Dark, Accessibility::default());
        let radius = t.metrics.panel_radius;
        let smooth = t.radius.smoothing;
        for (smoothing, border, inset) in
            [(smooth, false, false), (0., false, false), (smooth, true, false), (smooth, false, true)]
        {
            let what = format!("smoothing {smoothing}, border {border}, inset {inset}");
            let (image, scale) = capture(Clipped { radius, smoothing, border, inset });
            let at = |x: f32, y: f32| {
                let p = image.get_pixel((x * scale as f32) as u32, (y * scale as f32) as u32).0;
                [p[0], p[1], p[2]]
            };
            // Each corner's outermost pixel is the ground, not the child.
            let o = if inset { 21.5 } else { 20.5 };
            for (x, y) in [(o, o), (200. - o, o), (200. - o, 200. - o), (o, 200. - o)] {
                assert_eq!(at(x, y), rgb3(GROUND), "{what}: corner at {x},{y}");
            }
            // Inside the curve and along the straight edges the child shows.
            assert_eq!(at(100., 100.), rgb3(CHILD), "{what}: middle");
            let edge = if border { 6. } else { 2. };
            assert_eq!(at(100., 20. + edge), rgb3(CHILD), "{what}: top edge");
            assert_eq!(at(20. + edge, 100.), rgb3(CHILD), "{what}: left edge");
            // A pixel on the diagonal just inside the corner's reach is
            // cut (the ground, or the border's own corner): a square clip
            // would paint the child there.
            let d = radius * 0.2;
            for (x, y) in [(20. + d, 20. + d), (180. - d, 180. - d)] {
                assert_ne!(at(x, y), rgb3(CHILD), "{what}: corner curve at {x},{y}");
            }
        }
    });
}
