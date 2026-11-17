//! Powermove P-SLOT-PICTURE: while a surface's body scrolls, each AppKit
//! view in it (a control, a text editor that is not being edited) is drawn
//! with the rows as a picture AppKit takes of the view itself (its value,
//! state and appearance, light or dark), and the view stays hidden where it
//! was: moving AppKit views with a scroll made every frame a synchronous
//! present. The surface holds its views' pictures while the scroll moves and
//! lets them go when it rests; one view is itself again at once when it is
//! clicked, given a key or its value changes. One hold is shared by a
//! surface's views, so a view made for a row that scrolls in starts as a
//! picture too.
use gpui::*;
use gpui_macos::native_control::SlotPicture;
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

/// A surface's hold on its views' pictures.
#[derive(Clone, Default)]
pub struct PictureHold(Rc<Cell<bool>>);
impl PictureHold {
    pub fn held(&self) -> bool {
        self.0.get()
    }
    /// Whether the hold changed.
    pub fn set(&self, held: bool) -> bool {
        self.0.replace(held) != held
    }
}

/// One view's picture.
#[derive(Default)]
pub struct Picture {
    pub hold: PictureHold,
    /// Taken by the first paint of a hold.
    image: Rc<RefCell<Option<Arc<RenderImage>>>>,
    /// The real view for the rest of the hold.
    live: Rc<Cell<bool>>,
}
thread_local! {
    /// Pictures of views dropped mid-hold, freed by the next paint (the
    /// sprite atlas belongs to the window).
    static RETIRED: RefCell<Vec<Arc<RenderImage>>> = RefCell::default();
}
impl Picture {
    /// The surface's hold began or ended: a new hold pictures every view.
    pub fn hold_changed(&self) {
        self.live.set(false);
    }
    /// The real view for the rest of this hold (clicked, a key, a value).
    pub fn go_live(&self) {
        self.live.set(true);
    }
    /// Whether its picture stands in for the view in the scene.
    pub fn standing(&self) -> bool {
        self.hold.held() && !self.live.get() && self.image.borrow().is_some()
    }
    /// Frees the picture with the next paint (the view is being dropped).
    pub fn retire(&self) {
        if let Some(image) = self.image.borrow_mut().take() {
            RETIRED.with_borrow_mut(|r| r.push(image));
        }
    }
    pub fn painter(&self) -> Painter {
        Painter {
            hold: self.hold.clone(),
            image: self.image.clone(),
            live: self.live.clone(),
        }
    }
    /// A press on the picture: the real view comes back, and once a frame
    /// has shown it the press is delivered again, to the view. False when
    /// no picture stands (the press is the view's or the row's own).
    pub fn press(&self, window: &mut Window) -> bool {
        if !self.standing() {
            return false;
        }
        self.live.set(true);
        let mtm = MainThreadMarker::new().expect("main thread");
        if let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() {
            window.on_next_frame(move |window, _| {
                window.on_next_frame(move |_, _| {
                    let mtm = MainThreadMarker::new().expect("main thread");
                    NSApplication::sharedApplication(mtm).postEvent_atStart(&event, true);
                });
            });
        }
        true
    }
}

/// A view's picture, for its paint.
pub struct Painter {
    hold: PictureHold,
    image: Rc<RefCell<Option<Arc<RenderImage>>>>,
    live: Rc<Cell<bool>>,
}
impl Painter {
    /// While the hold stands, paints the view's picture (taken by `take` the
    /// first time) at `bounds` and returns true: the caller hides the view.
    /// Otherwise frees a picture left from a hold and returns false: the
    /// caller places the view.
    pub fn paint(
        &self,
        bounds: Bounds<Pixels>,
        take: impl FnOnce() -> Option<SlotPicture>,
        window: &mut Window,
    ) -> bool {
        for image in RETIRED.with_borrow_mut(std::mem::take) {
            let _ = window.drop_image(image);
        }
        if self.hold.held() && !self.live.get() {
            let mut kept = self.image.borrow_mut();
            if kept.is_none() {
                *kept = take().and_then(render_image);
            }
            if let Some(image) = kept.clone() {
                let _ = window.paint_image(bounds, bounds, Corners::default(), 0., image, 0, false);
                return true;
            }
        } else if let Some(image) = self.image.borrow_mut().take() {
            let _ = window.drop_image(image);
        }
        false
    }
}

/// A slot's picture as GPUI's image.
fn render_image(p: SlotPicture) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(p.width, p.height, p.bgra)?;
    Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
}

#[cfg(test)]
mod native_control_picture_tests {
    use super::*;
    use crate::native_control::{BuildSnapshot, reveal_paint};
    use gpui::prelude::*;

    struct Frame {
        build: Option<BuildSnapshot>,
        y: f32,
        ground: u32,
        picture: Picture,
    }
    impl Render for Frame {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let painter = self.picture.painter();
            let (filters, alpha) = reveal_paint(self.build);
            div().size_full().bg(rgb(self.ground)).child(
                div()
                    .absolute()
                    .left(px(20.))
                    .top(px(self.y))
                    .w(px(64.))
                    .h(px(32.))
                    .filter(filters)
                    .opacity(alpha)
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, _| {
                                assert!(painter.paint(
                                    bounds,
                                    || {
                                        Some(SlotPicture {
                                            width: 64,
                                            height: 32,
                                            bgra: (0..64 * 32)
                                                .flat_map(|i| {
                                                    if i % 64 < 32 {
                                                        [40, 180, 220, 255]
                                                    } else {
                                                        [200, 60, 80, 255]
                                                    }
                                                })
                                                .collect(),
                                        })
                                    },
                                    window
                                ));
                            },
                        )
                        .absolute()
                        .inset_0(),
                    ),
            )
        }
    }
    fn build(revision: Option<u64>, resolve: f32) -> BuildSnapshot {
        BuildSnapshot {
            revision,
            ready: (resolve > 0.).then_some(resolve),
            filter: Some(Filter::DotMatrix {
                detail: 2.5,
                resolve,
                accent: 0xff8800,
            }),
        }
    }
    fn capture(build: Option<BuildSnapshot>, y: f32) -> image::RgbaImage {
        capture_on_ground(build, y, 0x101010)
    }
    fn capture_on_ground(build: Option<BuildSnapshot>, y: f32, ground: u32) -> image::RgbaImage {
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(gpui_parley::ParleyTextSystem::new(
                gpui_parley::SystemFonts::Load,
            )),
            Arc::new(()),
            || {
                Some(Box::new(
                    gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                ))
            },
        );
        let window = cx
            .open_window(size(px(120.), px(100.)), |_, cx| {
                cx.new(|_| {
                    let picture = Picture::default();
                    picture.hold.set(true);
                    Frame {
                        build,
                        y,
                        ground,
                        picture,
                    }
                })
            })
            .unwrap();
        cx.run_until_parked();
        let image = cx.capture_screenshot(window.into()).unwrap();
        let scale = image.width() / 120;
        image::imageops::crop_imm(&image, 20 * scale, y as u32 * scale, 64 * scale, 32 * scale)
            .to_image()
    }
    #[::core::prelude::v1::test]
    fn scrolling_picture_gets_dots_and_reveal_alpha() {
        let plain = capture(None, 20.);
        let dotted = capture(Some(build(Some(7), 0.)), 20.);
        assert_ne!(plain, dotted, "scroll picture bypassed DotMatrix");
        assert!(
            dotted.pixels().any(|p| p.0[..3] != [16, 16, 16]),
            "no dots painted"
        );
        let hidden = capture(Some(build(None, 0.)), 20.);
        assert!(
            hidden.pixels().all(|p| p.0[..3] == [16, 16, 16]),
            "uncommitted picture bypassed reveal alpha"
        );
    }
    #[::core::prelude::v1::test]
    fn scrolling_moves_dot_grid_without_resampling_or_progress_change() {
        let reveal = Some(build(Some(7), 0.));
        assert_eq!(capture(reveal, 20.), capture(reveal, 37.));
    }
    #[::core::prelude::v1::test]
    fn resolved_scroll_picture_matches_integrations_unfiltered_picture() {
        for (scheme, ground) in [("Dark", 0x101010), ("Light", 0xf5f5f5)] {
            let plain = capture_on_ground(None, 20., ground);
            let dotted = capture_on_ground(Some(build(Some(7), 0.)), 20., ground);
            let resolved = capture_on_ground(Some(build(Some(7), 1.)), 20., ground);
            assert_ne!(dotted, plain, "{scheme} picture bypassed dots");
            assert_eq!(resolved, plain, "{scheme} resolved picture changed");
            if let Some(dir) = std::env::var_os("PM_SCROLL_REVEAL_CAPTURE") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                for (stage, image) in [("before", plain), ("dots", dotted), ("resolved", resolved)]
                {
                    image
                        .save(dir.join(format!("{scheme}-{stage}.png")))
                        .unwrap();
                }
            }
        }
    }
}
