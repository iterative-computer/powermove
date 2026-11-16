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

/// Panel `inspector` is drawn by a native surface below the window (an
/// extension's page) when `native`: the shell leaves only its body open.
struct Native(bool);

impl pm_shell::integration::Integration for Native {
    fn observe(&self, _: &mut Context<pm_shell::view::Editor>) {}
    fn command(&self, _: &pm_shell::Command, _: &mut Window, _: &mut gpui::App) -> bool {
        false
    }
    fn panel(
        &self,
        _: &str,
        _: pm_shell::layout::Rect,
        _: &mut Window,
        _: &mut gpui::App,
    ) -> Option<gpui::AnyElement> {
        None
    }
    fn native_panel(&self, id: &str, _: &gpui::App) -> bool {
        self.0 && id == "inspector"
    }
    fn project_id(&self, _: &gpui::App) -> Option<String> {
        None
    }
    fn status(&self, _: &gpui::App) -> String {
        String::new()
    }
    fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut gpui::App) {}
}

fn editor(native: bool) -> (image::RgbaImage, f32, pm_shell::layout::Rect) {
    let tokens = Tokens::new(Scheme::Light, Accessibility::default());
    let m = tokens.metrics;
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)),
        Arc::new(()),
        || Some(Box::new(gpui_apple::metal_renderer::MetalHeadlessRenderer::new())),
    );
    let window = cx
        .open_window(size(px(m.window_width), px(m.window_height)), |_, cx| {
            cx.new(|cx| {
                let mut v = pm_shell::view::Editor::new(tokens, None, cx);
                v.layout = pm_shell::layout::Layout::preset("design");
                v.integration = Some(std::rc::Rc::new(Native(native)));
                v
            })
        })
        .unwrap();
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into()).unwrap();
    let rect = window
        .update(&mut cx, |v, _, _| v.panel_rect("inspector"))
        .unwrap()
        .expect("the design workspace shows Properties");
    let scale = image.width() as f32 / m.window_width;
    (image, scale, rect)
}

#[test]
fn a_panel_with_a_native_body_is_one_surface_around_it() {
    objc2::rc::autoreleasepool(|_| {
        let (drawn, scale, r) = editor(false);
        let (native, _, same) = editor(true);
        assert_eq!(r, same);
        let m = Tokens::new(Scheme::Light, Accessibility::default()).metrics;
        let edge = m.border.max(m.splitter_reach);
        let at = |image: &image::RgbaImage, x: f32, y: f32| {
            image.get_pixel((x * scale) as u32, (y * scale) as u32).0
        };
        // Around the body (the header's top, the side and bottom edges the
        // page leaves the dividers, the corners it rounds off) the panel's
        // own material, as a panel the shell draws whole.
        let bottom = r.y + r.h - edge / 2.;
        let body_bottom = r.y + r.h - edge - 0.5;
        for (what, x, y) in [
            ("header", r.x + r.w * 0.6, r.y + 3.),
            ("left edge", r.x + edge / 2. + 0.5, r.y + r.h / 2.),
            ("right edge", r.x + r.w - edge / 2. - 0.5, r.y + r.h / 2.),
            ("bottom edge", r.x + r.w / 2., bottom),
            ("rounded-off corner", r.x + edge + 0.5, body_bottom),
            ("rounded-off corner", r.x + r.w - edge - 0.5, body_bottom),
        ] {
            // (A path's fill rounds a channel apart from a quad's.)
            let (a, b) = (at(&native, x, y), at(&drawn, x, y));
            assert!((0..4).all(|i| a[i].abs_diff(b[i]) <= 2), "{what} at {x},{y}: {a:?} vs {b:?}");
        }
        // The body itself is left open for the page.
        let (x, y) = (r.x + r.w / 2., r.y + r.h / 2.);
        assert_ne!(at(&native, x, y), at(&drawn, x, y), "the body is open");
    });
}
