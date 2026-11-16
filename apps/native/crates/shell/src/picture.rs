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
