//! Moving the Agent popover by its header (agent-panel.md §1).
//!
//! The popover is a window of its own. AppKit never drags it (it is not
//! movable): the app does, so the popover follows the pointer one to one,
//! stays inside its editor's content area below the toolbar, and every press,
//! move and release of the drag belongs to the popover. Nothing under it (a
//! shape tool on the canvas, a timeline row) ever sees them. Over a dock
//! target the editor shows the same drop preview a tab drag shows, and a
//! release there docks the panel as Pin does.
//!
//! Points are AppKit screen points (origin bottom-left), as `NSEvent`'s
//! mouse location and window frames report them.

/// A rectangle in screen points: `x`, `y` its bottom-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
impl Area {
    pub fn top(&self) -> f64 {
        self.y + self.h
    }
}
/// A header drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeaderDrag {
    /// Pointer and popover origin at the press.
    pointer: (f64, f64),
    origin: (f64, f64),
    size: (f64, f64),
    /// Where the popover may go: the editor's content below its toolbar.
    area: Area,
    /// Past the threshold: the popover follows; before it a press is a press.
    pub moved: bool,
    /// Over a dock target, as the editor last answered.
    pub docking: bool,
}
/// A press that moves less than this is not a drag (`metrics.drag_threshold`).
const THRESHOLD: f64 = 3.;
impl HeaderDrag {
    pub fn new(pointer: (f64, f64), frame: Area, area: Area) -> Self {
        Self {
            pointer,
            origin: (frame.x, frame.y),
            size: (frame.w, frame.h),
            area,
            moved: false,
            docking: false,
        }
    }
    /// The origin the popover takes for the pointer at `to`: the press's
    /// offset kept (one to one), held inside the area. `None` while the
    /// pointer has not left the threshold.
    pub fn follow(&mut self, to: (f64, f64)) -> Option<(f64, f64)> {
        let (dx, dy) = (to.0 - self.pointer.0, to.1 - self.pointer.1);
        if !self.moved && dx.hypot(dy) < THRESHOLD {
            return None;
        }
        self.moved = true;
        Some(clamp(
            (self.origin.0 + dx, self.origin.1 + dy),
            self.size,
            self.area,
        ))
    }
    /// Where it started (Escape puts it back).
    pub fn start(&self) -> (f64, f64) {
        self.origin
    }
}
/// An origin held so a `size` window lies inside `area`; a window larger
/// than the area keeps its top-left corner in it.
pub(crate) fn clamp(origin: (f64, f64), size: (f64, f64), area: Area) -> (f64, f64) {
    let x = origin.0.min(area.x + area.w - size.0).max(area.x);
    let y = origin.1.max(area.y).min(area.top() - size.1);
    (x, y.max(area.top() - size.1.max(area.h)))
}
/// The pointer in an editor's own points (top-left origin), for its drop
/// targets: `content` is the editor's content area on screen.
pub(crate) fn in_editor(pointer: (f64, f64), content: Area) -> (f32, f32) {
    (
        (pointer.0 - content.x) as f32,
        (content.top() - pointer.1) as f32,
    )
}
/// Where a popover put down stays, relative to its editor's top-left
/// corner, so it reopens there (per window). GPUI points, top-left origin.
pub(crate) fn remembered(popover: (f32, f32), editor: (f32, f32)) -> (f32, f32) {
    (popover.0 - editor.0, popover.1 - editor.1)
}

#[cfg(target_os = "macos")]
pub(crate) mod native {
    use super::Area;
    use objc2_app_kit::{NSEvent, NSWindow};
    use objc2_foundation::NSPoint;
    /// The pointer on screen.
    pub fn pointer() -> (f64, f64) {
        let p = NSEvent::mouseLocation();
        (p.x, p.y)
    }
    pub fn frame(window: &NSWindow) -> Area {
        let f = window.frame();
        Area {
            x: f.origin.x,
            y: f.origin.y,
            w: f.size.width,
            h: f.size.height,
        }
    }
    /// The content area on screen (a full-size content view: the whole window).
    pub fn content(window: &NSWindow) -> Area {
        let rect = window.contentLayoutRect();
        let r = window.convertRectToScreen(rect);
        let f = window.frame();
        // A full-size content view starts at the window's top edge.
        Area {
            x: r.origin.x,
            y: r.origin.y,
            w: r.size.width,
            h: (f.origin.y + f.size.height) - r.origin.y,
        }
    }
    pub fn move_to(window: &NSWindow, origin: (f64, f64)) {
        window.setFrameOrigin(NSPoint::new(origin.0, origin.1));
    }
    /// See-through over a dock target, so the drop preview under it shows.
    pub fn see_through(window: &NSWindow, on: bool) {
        window.setAlphaValue(if on { 0.55 } else { 1. });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const EDITOR: Area = Area {
        x: 100.,
        y: 50.,
        w: 1200.,
        h: 800.,
    };
    #[test]
    fn the_popover_follows_the_pointer_one_to_one_past_a_threshold() {
        let frame = Area {
            x: 700.,
            y: 300.,
            w: 440.,
            h: 400.,
        };
        let mut drag = HeaderDrag::new((800., 690.), frame, EDITOR);
        assert_eq!(drag.follow((801., 691.)), None, "a press is a press");
        assert!(!drag.moved);
        assert_eq!(drag.follow((780., 650.)), Some((680., 260.)));
        // Back inside the threshold it still follows: the drag has begun.
        assert_eq!(drag.follow((800., 690.)), Some((700., 300.)));
        assert_eq!(drag.start(), (700., 300.));
    }
    #[test]
    fn it_never_leaves_the_editors_content_area() {
        let size = (440., 400.);
        // Past each edge: held at it.
        assert_eq!(clamp((-500., 300.), size, EDITOR), (100., 300.));
        assert_eq!(clamp((5000., 300.), size, EDITOR), (860., 300.));
        assert_eq!(clamp((500., -100.), size, EDITOR), (500., 50.));
        assert_eq!(clamp((500., 5000.), size, EDITOR), (500., 450.));
        // Taller than the area: its top edge stays in it.
        assert_eq!(clamp((500., 0.), (440., 900.), EDITOR), (500., -50.));
        let mut drag = HeaderDrag::new(
            (300., 600.),
            Area {
                x: 200.,
                y: 300.,
                w: 440.,
                h: 400.,
            },
            EDITOR,
        );
        assert_eq!(drag.follow((-2000., 9000.)), Some((100., 450.)));
    }
    #[test]
    fn the_pointer_reads_in_editor_points_and_a_put_down_place_is_kept() {
        assert_eq!(in_editor((150., 800.), EDITOR), (50., 50.));
        assert_eq!(remembered((640., 120.), (100., 40.)), (540., 80.));
    }
}
