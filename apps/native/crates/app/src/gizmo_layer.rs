//! The viewer's on-canvas outline and handles as Core Animation shape
//! layers over the picture, set on the main thread whenever the gizmo
//! changes. A drag's box therefore follows the pointer at the pointer's pace
//! however long the engine takes to draw the picture under it (the picture
//! follows on its own frames). Guides and safe areas stay in the engine's
//! overlay.
use objc2::{msg_send, rc::Retained, runtime::AnyObject};
use objc2_core_graphics::{CGColor, CGColorSpace, CGMutablePath};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CAShapeLayer, CATransaction};
use pm_engine_service::OverlayLine;

/// Where the lines go: the composition's rectangle in the viewer view's
/// coordinates (points, top-left origin), and that view's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The composition's top-left corner and size in the viewer view (pt).
    pub origin: [f64; 2],
    pub size: [f64; 2],
    /// The viewer view's size (pt): its layer is y-up.
    pub view: [f64; 2],
}

/// What a mark is: a square or a circle `size` points across, or a stroke
/// to another place in the composition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Square,
    Circle,
    /// A stroke to here (the composition's unit square), `width` points wide.
    To([f32; 2]),
}
/// A mark of the gizmo that keeps its size on screen whatever the zoom
/// (viewer-chrome.md): a handle, the rotation knob, the anchor's rings, the
/// box's keyline. `at` is in the composition's unit square, as lines are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub shape: Shape,
    pub at: [f32; 2],
    /// Across (a square's side, a circle's diameter), in points.
    pub size: f32,
    pub fill: Option<[f32; 4]>,
    pub stroke: Option<[f32; 4]>,
    pub width: f32,
    /// Drawn under the lines (a keyline), else over them.
    pub under: bool,
}
/// The lines grouped by colour, in the viewer view's y-up layer points.
pub fn segments(lines: &[OverlayLine], at: Placement) -> Vec<([f32; 4], Vec<[f64; 4]>)> {
    let mut groups: Vec<([f32; 4], Vec<[f64; 4]>)> = vec![];
    for line in lines {
        let [x0, y0, x1, y1] = line.ends.map(f64::from);
        let point = |x: f64, y: f64| {
            (
                at.origin[0] + x * at.size[0],
                at.view[1] - (at.origin[1] + y * at.size[1]),
            )
        };
        let (a, b) = (point(x0, y0), point(x1, y1));
        if ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite()) {
            continue;
        }
        let segment = [a.0, a.1, b.0, b.1];
        match groups.iter_mut().find(|(c, _)| *c == line.color) {
            Some((_, s)) => s.push(segment),
            None => groups.push((line.color, vec![segment])),
        }
    }
    groups
}

/// One piece of a shape layer's path, in the viewer view's y-up points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Draw {
    Rect(NSRect),
    Ellipse(NSRect),
    Line([f64; 4]),
}
/// What the gizmo layer draws, as shape layers bottom to top: keylines under
/// the lines, the lines (1 pt, grouped by colour), then the marks over them.
pub fn drawing(lines: &[OverlayLine], marks: &[Mark], at: Placement) -> Vec<(Look, Vec<Draw>)> {
    let mut groups: Vec<(Look, Vec<Draw>)> = vec![];
    fn push(groups: &mut Vec<(Look, Vec<Draw>)>, look: Look, draw: Draw) {
        match groups.iter_mut().find(|(l, _)| *l == look) {
            Some((_, draws)) => draws.push(draw),
            None => groups.push((look, vec![draw])),
        }
    }
    let place = |p: [f32; 2]| {
        (
            at.origin[0] + f64::from(p[0]) * at.size[0],
            at.view[1] - (at.origin[1] + f64::from(p[1]) * at.size[1]),
        )
    };
    let add = |under: bool, groups: &mut Vec<(Look, Vec<Draw>)>| {
        for mark in marks.iter().filter(|m| m.under == under) {
            let (x, y) = place(mark.at);
            if !(x.is_finite() && y.is_finite()) {
                continue;
            }
            let look = Look {
                fill: mark.fill,
                stroke: mark.stroke,
                width: mark.width,
            };
            let half = f64::from(mark.size) / 2.;
            let rect = NSRect::new(
                NSPoint::new(x - half, y - half),
                NSSize::new(2. * half, 2. * half),
            );
            let draw = match mark.shape {
                Shape::Square => Draw::Rect(rect),
                Shape::Circle => Draw::Ellipse(rect),
                Shape::To(to) => {
                    let (x1, y1) = place(to);
                    if !(x1.is_finite() && y1.is_finite()) {
                        continue;
                    }
                    Draw::Line([x, y, x1, y1])
                }
            };
            push(groups, look, draw);
        }
    };
    add(true, &mut groups);
    for (color, segments) in segments(lines, at) {
        let look = Look {
            fill: None,
            stroke: Some(color),
            width: 1.,
        };
        for segment in segments {
            push(&mut groups, look, Draw::Line(segment));
        }
    }
    add(false, &mut groups);
    groups
}
/// One shape layer's paint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub fill: Option<[f32; 4]>,
    pub stroke: Option<[f32; 4]>,
    pub width: f32,
}
pub struct GizmoLayer {
    parent: Retained<CALayer>,
    root: Retained<CALayer>,
    shapes: Vec<Retained<CAShapeLayer>>,
}
impl GizmoLayer {
    pub fn new(parent: Retained<CALayer>) -> Self {
        let root = CALayer::new();
        root.setMasksToBounds(true);
        root.setZPosition(10.);
        parent.addSublayer(&root);
        Self {
            parent,
            root,
            shapes: vec![],
        }
    }
    /// Still attached to `parent` (the viewer's layer can be replaced).
    pub fn on(&self, parent: &CALayer) -> bool {
        std::ptr::eq(&*self.parent, parent)
    }
    pub fn set(&mut self, lines: &[OverlayLine], marks: &[Mark], at: Placement, scale: f64) {
        let groups = drawing(lines, marks, at);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.root.setFrame(NSRect::new(
            NSPoint::new(0., 0.),
            NSSize::new(at.view[0], at.view[1]),
        ));
        while self.shapes.len() < groups.len() {
            let shape = CAShapeLayer::new();
            self.root.addSublayer(&shape);
            self.shapes.push(shape);
        }
        let space = CGColorSpace::with_name(Some(unsafe {
            objc2_core_graphics::kCGColorSpaceExtendedLinearDisplayP3
        }));
        let colour = |c: Option<[f32; 4]>| {
            let components = c?.map(f64::from);
            space
                .as_deref()
                .and_then(|space| unsafe { CGColor::new(Some(space), components.as_ptr()) })
        };
        let null = std::ptr::null();
        for (i, shape) in self.shapes.iter().enumerate() {
            let Some((look, draws)) = groups.get(i) else {
                shape.setHidden(true);
                continue;
            };
            let path = CGMutablePath::new();
            for draw in draws {
                unsafe {
                    match *draw {
                        Draw::Rect(r) => CGMutablePath::add_rect(Some(&path), null, r),
                        Draw::Ellipse(r) => CGMutablePath::add_ellipse_in_rect(Some(&path), null, r),
                        Draw::Line([x0, y0, x1, y1]) => {
                            CGMutablePath::move_to_point(Some(&path), null, x0, y0);
                            CGMutablePath::add_line_to_point(Some(&path), null, x1, y1);
                        }
                    }
                }
            }
            shape.setHidden(false);
            shape.setContentsScale(scale);
            shape.setFillColor(colour(look.fill).as_deref());
            shape.setStrokeColor(colour(look.stroke).as_deref());
            shape.setLineWidth(f64::from(look.width));
            shape.setPath(Some(&path));
        }
        CATransaction::commit();
    }
    pub fn clear(&mut self) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for shape in &self.shapes {
            shape.setHidden(true);
        }
        CATransaction::commit();
    }
}
impl Drop for GizmoLayer {
    fn drop(&mut self) {
        self.root.removeFromSuperlayer();
    }
}
/// The viewer view's layer, as a CALayer.
pub fn layer_of(object: &AnyObject) -> Option<Retained<CALayer>> {
    let layer: Option<Retained<CALayer>> = unsafe { msg_send![object, layer] };
    layer
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A line across the composition's top edge lands at the composition's
    /// rectangle in the viewer view, in its y-up layer coordinates.
    #[test]
    fn lines_land_on_the_composition_in_the_viewer() {
        let at = Placement {
            origin: [100., 50.],
            size: [400., 225.],
            view: [600., 400.],
        };
        let red = [1., 0., 0., 1.];
        let lines = [
            OverlayLine {
                ends: [0., 0., 1., 0.],
                color: red,
            },
            OverlayLine {
                ends: [0.5, 0.5, 0.5, 1.],
                color: red,
            },
            OverlayLine {
                ends: [0., 0., 0., 1.],
                color: [0., 1., 0., 1.],
            },
        ];
        let groups = segments(&lines, at);
        assert_eq!(groups.len(), 2, "one shape per colour");
        assert_eq!(groups[0].1[0], [100., 350., 500., 350.]);
        assert_eq!(groups[0].1[1], [300., 237.5, 300., 125.]);
        assert_eq!(groups[1].1[0], [100., 350., 100., 125.]);
    }
    /// Keylines go under the lines and the marks over them, each mark its
    /// size in points wherever the composition is scaled.
    #[test]
    fn keylines_go_under_the_lines_and_marks_over_them() {
        let at = Placement {
            origin: [100., 50.],
            size: [400., 225.],
            view: [600., 400.],
        };
        let accent = [1., 0.5, 0., 1.];
        let dark = [0.05, 0.05, 0.05, 1.];
        let lines = [OverlayLine {
            ends: [0., 0., 1., 0.],
            color: accent,
        }];
        let mark = |shape, under, fill| Mark {
            shape,
            at: [0.5, 0.5],
            size: 7.,
            fill,
            stroke: Some(dark),
            width: 1.,
            under,
        };
        let marks = [
            mark(Shape::Square, false, Some(accent)),
            mark(Shape::To([1., 0.5]), true, None),
        ];
        let groups = drawing(&lines, &marks, at);
        assert_eq!(groups.len(), 3);
        assert!(matches!(groups[0].1[0], Draw::Line([300., 237.5, 500., 237.5])), "keyline first");
        assert_eq!(groups[1].0.stroke, Some(accent), "then the lines");
        let Draw::Rect(r) = groups[2].1[0] else {
            panic!("the handle last");
        };
        assert_eq!((r.size.width, r.size.height), (7., 7.));
        assert_eq!((r.origin.x, r.origin.y), (296.5, 234.));
        assert_eq!(groups[2].0.fill, Some(accent));
    }
}
